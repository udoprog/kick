use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use clap::Parser;
use semver::{Comparator, Op, Prerelease, Version, VersionReq};
use toml_edit::{Formatted, Item, Table, Value};

use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::validation::Validation;
use crate::workspace;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// A version specification to set.
    #[arg(long = "set", short = 's', name = "[<crate>=]version")]
    set: Vec<String>,
    /// Perform a major version bump. This will remove any existing pre-release setting.
    #[arg(long)]
    major: bool,
    /// Perform a minor version bump. This will remove any existing pre-release setting.
    #[arg(long)]
    minor: bool,
    /// Perform a patch bump. This will remove any existing pre-release setting.
    #[arg(long)]
    patch: bool,
    /// Set a prerelease string.
    #[arg(long)]
    pre: Option<String>,
    /// Use the existing crate version just so that we can perform all checks.
    #[arg(long = "existing")]
    existing: bool,
    /// Make a commit with the current version with the message `Release <version>`.
    #[arg(long)]
    commit: bool,
}

pub(crate) fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut version_set = VersionSet {
        major: opts.major,
        minor: opts.minor,
        patch: opts.patch,
        pre: match &opts.pre {
            Some(pre) if !pre.is_empty() => {
                Some(Prerelease::new(pre).with_context(|| pre.to_owned())?)
            }
            Some(..) => Some(Prerelease::EMPTY),
            _ => None,
        },
        existing: opts.existing,
        ..VersionSet::default()
    };

    // Parse explicit version upgrades.
    for version in &opts.set {
        if let Some((id, version)) = version.split_once('=') {
            version_set
                .crates
                .insert(id.to_string(), Version::parse(version)?);
        } else {
            version_set.base = Some(Version::parse(version)?);
        }
    }

    for module in cx.modules() {
        version(cx, opts, module, &version_set).with_context(|| module.path().to_owned())?;
    }

    Ok(())
}

// #[tracing::instrument(skip_all, fields(path = module.path().as_str()))]
fn version(cx: &Ctxt<'_>, opts: &Opts, module: &Module, version_set: &VersionSet) -> Result<()> {
    let Some(workspace) = workspace::open(cx, module)? else {
        bail!("not a workspace");
    };

    let mut versions = HashMap::new();
    let mut packages = Vec::new();

    for package in workspace.packages() {
        if !package.manifest.is_publish()? {
            continue;
        }

        let name = package.manifest.crate_name()?;

        let current_version = if let Some(version) = package.manifest.version()? {
            Some(Version::parse(version)?)
        } else {
            None
        };

        if version_set.is_bump() {
            if let Some(from) = &current_version {
                let mut to = from.clone();

                if version_set.major {
                    to.major += 1;
                    to.minor = 0;
                    to.patch = 0;
                    to.pre = Prerelease::default();
                } else if version_set.minor {
                    to.minor += 1;
                    to.patch = 0;
                    to.pre = Prerelease::default();
                } else if version_set.patch {
                    to.patch += 1;
                    to.pre = Prerelease::default();
                }

                if let Some(pre) = &version_set.pre {
                    to.pre = pre.clone();
                }

                tracing::trace!(
                    ?name,
                    from = from.to_string(),
                    to = to.to_string(),
                    ?name,
                    "Bump version"
                );

                versions.insert(name.to_string(), to);
            }
        }

        if let Some(version) = version_set.crates.get(name).or(version_set.base.as_ref()) {
            tracing::info!(?name, version = ?version.to_string(), ?name, "Set version");
            versions.insert(name.to_string(), version.clone());
        }

        packages.push((package.clone(), current_version));
    }

    for (package, _) in &mut packages {
        let name = package.manifest.crate_name()?;

        let mut changed_manifest = false;
        let mut replaced = Vec::new();

        if let Some(version) = versions.get(name) {
            let root = package.manifest_dir.to_path(cx.root);
            let version_string = version.to_string();

            for replacement in cx.config.version(module) {
                if matches!(&replacement.crate_name, Some(id) if id != name) {
                    continue;
                }

                replaced.extend(
                    replacement
                        .replace_in(&root, "version", &version_string)
                        .context("Failed to replace version string")?,
                );
            }

            if package.manifest.version()? != Some(version_string.as_str()) {
                package.manifest.insert_version(&version_string)?;
                changed_manifest = true;
            }
        }

        if let Some(deps) = package.manifest.dependencies_mut() {
            if modify_dependencies(deps, &versions)? {
                changed_manifest = true;
            }
        }

        if let Some(deps) = package.manifest.dev_dependencies_mut() {
            if modify_dependencies(deps, &versions)? {
                changed_manifest = true;
            }
        }

        if let Some(deps) = package.manifest.build_dependencies_mut() {
            if modify_dependencies(deps, &versions)? {
                changed_manifest = true;
            }
        }

        if changed_manifest {
            cx.validation(Validation::SavePackage {
                package: package.clone(),
            });
        }

        for replaced in replaced {
            cx.validation(Validation::Replace { replaced });
        }
    }

    if opts.commit {
        let primary = workspace.primary_crate()?;

        let version = versions
            .get(primary.manifest.crate_name()?)
            .context("missing version for primary manifest")?;

        cx.validation(Validation::ReleaseCommit {
            path: primary.manifest_dir.clone(),
            version: version.clone(),
        });
    }

    Ok(())
}

#[derive(Debug, Default)]
struct VersionSet {
    base: Option<Version>,
    crates: HashMap<String, Version>,
    major: bool,
    minor: bool,
    patch: bool,
    pre: Option<Prerelease>,
    existing: bool,
}

impl VersionSet {
    fn is_bump(&self) -> bool {
        self.major || self.minor || self.patch || self.pre.is_some() || self.existing
    }
}

/// Extract package name.
fn package_name<'a>(key: &'a str, dep: &'a Item) -> &'a str {
    if let Some(Item::Value(value)) = dep.get("package") {
        if let Some(value) = value.as_str() {
            return value;
        }
    }

    key
}

/// Modify dependencies in place.
fn modify_dependencies(deps: &mut Table, versions: &HashMap<String, Version>) -> Result<bool> {
    let mut changed = false;

    for (key, dep) in deps.iter_mut() {
        let name = package_name(key.get(), dep);

        let (Some(version), Some(existing)) = (versions.get(name), find_version_mut(dep)) else {
            continue;
        };

        let existing_string = existing
            .as_str()
            .context("found version was not a string")?
            .to_owned();

        let new = modify_version_req(&existing_string, version)?;

        if existing_string != new {
            *existing = Value::String(Formatted::new(new));
            changed = true;
        }
    }

    Ok(changed)
}

/// Find the value corresponding to the version in use.
fn find_version_mut(item: &mut Item) -> Option<&mut Value> {
    match item {
        Item::Value(value) => match value {
            value @ Value::String(..) => Some(value),
            Value::InlineTable(table) => table.get_mut("version"),
            _ => None,
        },
        Item::Table(table) => {
            if let Item::Value(value) = table.get_mut("version")? {
                Some(value)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Parse and return a modified version requirement.
fn modify_version_req(req: &str, version: &Version) -> Result<String> {
    let mut req = VersionReq::parse(req)?;

    if let [Comparator { op: Op::Caret, .. }] = &req.comparators[..] {
        return Ok(version.to_string());
    }

    for comparator in &mut req.comparators {
        comparator.major = version.major;
        comparator.minor = Some(version.minor);
        comparator.patch = Some(version.patch);
        comparator.pre = version.pre.clone();
    }

    Ok(req.to_string())
}
