use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use clap::Parser;
use semver::{Comparator, Op, Prerelease, Version, VersionReq};
use toml_edit::{Formatted, Item, TableLike, Value};

use crate::cargo;
use crate::changes::Change;
use crate::cli::WithRepos;
use crate::ctxt::Ctxt;
use crate::model::Repo;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Version overrides to use in [crate=]version form.
    #[arg(long)]
    r#override: Vec<String>,
    /// Perform a major version bump.
    #[arg(long)]
    major: bool,
    /// Perform a minor version bump.
    #[arg(long)]
    minor: bool,
    /// Perform a patch bump.
    #[arg(long)]
    patch: bool,
    /// Set a prerelease string.
    #[arg(long)]
    pre: Option<String>,
    /// Make a commit with the current version with the message `Release <version>`.
    #[arg(long)]
    commit: bool,
    /// Filter crate names to bump.
    crates: Vec<String>,
}

pub(crate) fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    let mut version_set = VersionSet {
        major: opts.major,
        minor: opts.minor,
        patch: opts.patch,
        pre: match &opts.pre {
            Some(pre) if !pre.is_empty() => {
                Some(Prerelease::new(pre).with_context(|| pre.clone())?)
            }
            Some(..) => Some(Prerelease::EMPTY),
            _ => None,
        },
        ..VersionSet::default()
    };

    // Parse explicit version upgrades.
    for version in &opts.r#override {
        if let Some((id, version)) = version.split_once('=') {
            version_set
                .crates
                .insert(id.to_string(), Version::parse(version)?);
        } else {
            version_set.any = Some(Version::parse(version)?);
        }
    }

    let filter = opts
        .crates
        .iter()
        .map(|s| s.as_str())
        .collect::<HashSet<_>>();

    with_repos.run(
        "bump version",
        format_args!("version: {opts:?}"),
        |cx, repo| version(cx, opts, repo, &version_set, &filter),
    )?;

    Ok(())
}

struct VersionChange {
    old: Version,
    new: Version,
}

#[tracing::instrument(skip_all)]
fn version(
    cx: &Ctxt<'_>,
    opts: &Opts,
    repo: &Repo,
    version_set: &VersionSet,
    filter: &HashSet<&str>,
) -> Result<()> {
    let workspace = repo.workspace(cx)?;

    let mut versions = HashMap::new();

    for manifest in workspace.manifests() {
        let Some(package) = manifest.as_package() else {
            continue;
        };

        if !package.is_publish() {
            continue;
        }

        let name = package.name()?;

        if !filter.is_empty() && !filter.contains(name) {
            continue;
        }

        let current_version = if let Some(version) = package.version() {
            Some(Version::parse(version)?)
        } else {
            None
        };

        if version_set.is_bump()
            && let Some(from) = &current_version
        {
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
                name,
                from = from.to_string(),
                to = to.to_string(),
                "Bump version"
            );

            versions.insert(
                name.to_string(),
                VersionChange {
                    old: from.clone(),
                    new: to,
                },
            );

            continue;
        }

        if let Some(version) = version_set.crates.get(name).or(version_set.any.as_ref()) {
            tracing::info!(?name, version = ?version.to_string(), ?name, "Set version");

            versions.insert(
                name.to_string(),
                VersionChange {
                    old: version.clone(),
                    new: version.clone(),
                },
            );
        }
    }

    for manifest in workspace.manifests() {
        let mut changed_manifest = false;
        let mut replaced = Vec::new();
        let mut modified = manifest.clone();

        if let Some(package) = manifest.as_package() {
            let name = package.name()?;

            if let Some(VersionChange { new: version, .. }) = versions.get(name) {
                let root = cx.to_path(modified.dir());
                let version_string = version.to_string();

                for replacement in cx.config.version(repo) {
                    if matches!(&replacement.package_name, Some(id) if id != name) {
                        continue;
                    }

                    replaced.extend(
                        replacement
                            .replace_in(&root, "version", &version_string)
                            .context("Failed to replace version string")?,
                    );
                }

                if package.version() != Some(version_string.as_str()) {
                    modified
                        .ensure_package_mut()?
                        .insert_version(&version_string)?;
                    changed_manifest = true;
                }
            }
        }

        let mut handle_table_like = |table: &mut dyn TableLike| -> Result<()> {
            if let Some(target) = table.get_mut(cargo::TARGET)
                && let Some(targets) = target.as_table_like_mut()
            {
                for (_, table) in targets.iter_mut() {
                    for key in cargo::DEPS {
                        if let Some(deps) = table.get_mut(key).and_then(|d| d.as_table_like_mut())
                            && modify_dependencies(deps, &versions)?
                        {
                            changed_manifest = true;
                        }
                    }
                }
            }

            for key in cargo::DEPS {
                if let Some(deps) = table.get_mut(key).and_then(|d| d.as_table_like_mut())
                    && modify_dependencies(deps, &versions)?
                {
                    changed_manifest = true;
                }
            }

            Ok(())
        };

        handle_table_like(modified.as_table_like_mut())?;

        if let Some(workspace) = modified
            .get_mut(cargo::WORKSPACE)
            .and_then(|d| d.as_table_like_mut())
        {
            handle_table_like(workspace)?;
        }

        if changed_manifest {
            cx.change(Change::SavePackage {
                manifest: modified.clone(),
            });
        }

        for replaced in replaced {
            cx.change(Change::Replace { replaced });
        }
    }

    if opts.commit {
        let manifest = workspace.primary_package()?;
        let primary = manifest.ensure_package()?;

        let version = versions
            .get(primary.name()?)
            .context("Missing version for primary package")?
            .new
            .clone();

        cx.change(Change::ReleaseCommit {
            path: manifest.dir().to_owned(),
            version,
        });
    }

    Ok(())
}

#[derive(Debug, Default)]
struct VersionSet {
    any: Option<Version>,
    crates: HashMap<String, Version>,
    major: bool,
    minor: bool,
    patch: bool,
    pre: Option<Prerelease>,
}

impl VersionSet {
    fn is_bump(&self) -> bool {
        self.major || self.minor || self.patch || self.pre.is_some()
    }
}

/// Extract package name.
fn package_name<'a>(key: &'a str, dep: &'a Item) -> &'a str {
    if let Some(Item::Value(value)) = dep.get("package")
        && let Some(value) = value.as_str()
    {
        return value;
    }

    key
}

/// Modify dependencies in place.
fn modify_dependencies(
    deps: &mut dyn TableLike,
    versions: &HashMap<String, VersionChange>,
) -> Result<bool> {
    let mut changed = false;

    for (key, dep) in deps.iter_mut() {
        let name = package_name(key.get(), dep);

        let (Some(VersionChange { old, new }), Some(existing)) =
            (versions.get(name), find_version_mut(dep))
        else {
            continue;
        };

        let existing_string = existing
            .as_str()
            .context("found version was not a string")?
            .to_owned();

        let new = modify_version_req(&existing_string, old, new)?;

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
fn modify_version_req(req: &str, old: &Version, new: &Version) -> Result<String> {
    let mut req = VersionReq::parse(req)?;

    if let [Comparator { op: Op::Caret, .. }] = &req.comparators[..] {
        return Ok(new.to_string());
    }

    for c in &mut req.comparators {
        if !(c.major == old.major
            && c.minor == Some(old.minor)
            && c.patch == Some(old.patch)
            && c.pre == old.pre)
        {
            continue;
        }

        c.major = new.major;
        c.minor = Some(new.minor);
        c.patch = Some(new.patch);
        c.pre = new.pre.clone();
    }

    Ok(req.to_string())
}
