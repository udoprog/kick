use std::collections::HashMap;

use anyhow::{bail, Context, Result};
use clap::Parser;
use semver::{Comparator, Op, Prerelease, Version, VersionReq};
use toml_edit::{Formatted, Item, Table, Value};

use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::workspace;

#[derive(Debug, Default)]
struct VersionSet {
    base: Option<Version>,
    crates: HashMap<String, Version>,
    pre: Prerelease,
    major: bool,
    minor: bool,
    patch: bool,
}

impl VersionSet {
    fn is_bump(&self) -> bool {
        self.major || self.minor || self.patch || !self.pre.is_empty()
    }
}

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
    /// Filter by the specified modules.
    #[arg(long = "module", short = 'm', name = "module")]
    modules: Vec<String>,
}

pub(crate) fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut version_set = VersionSet::default();

    version_set.major = opts.major;
    version_set.minor = opts.minor;
    version_set.patch = opts.patch;
    version_set.pre = match &opts.pre {
        Some(pre) => Prerelease::new(pre).with_context(|| pre.to_owned())?,
        None => Prerelease::EMPTY,
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

    for module in cx.modules(&opts.modules) {
        version(cx, module, &version_set).with_context(|| module.path.clone())?;
    }

    Ok(())
}

#[tracing::instrument(skip_all, fields(path = module.path.as_str()))]
fn version(cx: &Ctxt<'_>, module: &Module, version_set: &VersionSet) -> Result<()> {
    let Some(workspace) = workspace::open(cx, module)? else {
        bail!("not a workspace");
    };

    let mut new_versions = HashMap::new();
    let mut packages = Vec::new();

    for package in workspace.packages() {
        if !package.manifest.is_publish()? {
            continue;
        }

        let name = package.manifest.crate_name()?;

        if version_set.is_bump() {
            if let Some(version) = package.manifest.version()? {
                let from = Version::parse(version)?;
                let mut to = from.clone();
                to.major += u64::from(version_set.major);
                to.minor += u64::from(version_set.minor);
                to.patch += u64::from(version_set.patch);
                to.pre = version_set.pre.clone();
                tracing::info!(?name, ?from, ?to, ?name, "bump version");
                new_versions.insert(name.to_string(), to);
            }
        }

        if let Some(version) = version_set.crates.get(name).or(version_set.base.as_ref()) {
            tracing::info!(?name, ?version, ?name, "set version");
            new_versions.insert(name.to_string(), version.clone());
        }

        packages.push(package.clone());
    }

    for package in &mut packages {
        let name = package.manifest.crate_name()?;

        let mut changed = false;

        if let Some(version) = new_versions.get(name) {
            package.manifest.insert_version(&version.to_string())?;
            changed = true;
        }

        if let Some(deps) = package.manifest.dependencies_mut() {
            if modify_dependencies(deps, &new_versions)? {
                changed = true;
            }
        }

        if let Some(deps) = package.manifest.dev_dependencies_mut() {
            if modify_dependencies(deps, &new_versions)? {
                changed = true;
            }
        }

        if let Some(deps) = package.manifest.build_dependencies_mut() {
            if modify_dependencies(deps, &new_versions)? {
                changed = true;
            }
        }

        if changed {
            tracing::info!("Saving {}", package.manifest_path);
            let out = package.manifest_path.to_path(cx.root);
            package.manifest.save_to(out)?;
        }
    }

    Ok(())
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
fn modify_dependencies(deps: &mut Table, new_versions: &HashMap<String, Version>) -> Result<bool> {
    let mut changed = false;

    for (key, dep) in deps.iter_mut() {
        let name = package_name(key.get(), dep);

        let Some(version) = new_versions.get(name) else {
            continue;
        };

        match dep {
            Item::Value(value) => match value {
                Value::String(string) => {
                    let req = string.value();
                    let req = modify_version_req(req, version)?;
                    *string = Formatted::new(req);
                    changed = true;
                }
                Value::InlineTable(table) => {
                    let Some(value) = table.get_mut("version") else {
                        continue;
                    };

                    let req = value.as_str().context("missing value")?;
                    let req = modify_version_req(req, version)?;
                    *value = Value::String(Formatted::new(req));
                    changed = true;
                }
                _ => {
                    continue;
                }
            },
            Item::Table(table) => {
                let Some(value) = table.get_mut("version") else {
                    continue;
                };

                let req = value.as_str().context("missing value")?;
                let req = modify_version_req(req, version)?;
                *value = Item::Value(Value::String(Formatted::new(req)));
                changed = true;
            }
            _ => {
                continue;
            }
        }
    }

    Ok(changed)
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
