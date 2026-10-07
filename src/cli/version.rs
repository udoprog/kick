use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, Result, bail};
use clap::Parser;
use semver::{Comparator, Op, Prerelease, Version, VersionReq};
use toml_edit::{Formatted, Item, TableLike, Value};

use crate::cargo;
use crate::changes::Change;
use crate::cli::WithRepos;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::version_groups::{self, PackageVersion, ResolvedGroups};

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
    /// Ignore `[[version_group]]` configuration, and only change the version of
    /// the selected crates rather than every crate in their version group.
    #[arg(long)]
    no_group: bool,
    /// Filter crate names to bump.
    ///
    /// Crates in the same `[[version_group]]` as a selected crate are also
    /// bumped to the same version unless `--no-group` is specified.
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

#[derive(Debug, Clone, PartialEq, Eq)]
struct VersionChange {
    old: Version,
    new: Version,
}

/// A crate which is a candidate for having its version changed.
#[derive(Debug)]
struct Candidate {
    name: String,
    version: PackageVersion,
}

/// The planned version changes.
#[derive(Debug, Default)]
struct Plan {
    /// Version changes for each crate.
    versions: HashMap<String, VersionChange>,
    /// The new `[workspace.package] version`, if it should change.
    workspace: Option<Version>,
}

/// Plan version changes for the given candidates.
fn plan(
    candidates: &[Candidate],
    groups: &ResolvedGroups,
    filter: &HashSet<&str>,
    version_set: &VersionSet,
) -> Result<Plan> {
    let by_name = candidates
        .iter()
        .map(|c| (c.name.as_str(), c))
        .collect::<HashMap<_, _>>();

    let is_selected = |name: &str| {
        if filter.is_empty() || filter.contains(name) {
            return true;
        }

        groups
            .group_of(name)
            .is_some_and(|group| group.iter().any(|m| filter.contains(m.as_str())))
    };

    // Expand overrides to cover every member of a group.
    let mut overrides = HashMap::<&str, (&str, &Version)>::new();

    for (name, version) in &version_set.crates {
        let members = match groups.group_of(name) {
            Some(group) => group.iter().map(String::as_str).collect::<Vec<_>>(),
            None => vec![name.as_str()],
        };

        for member in members {
            if let Some((other, existing)) = overrides.insert(member, (name, version))
                && existing != version
            {
                bail!(
                    "Conflicting version overrides for version group containing `{member}`: {other}={existing} and {name}={version}"
                );
            }
        }
    }

    let mut versions = HashMap::new();

    for candidate in candidates {
        let name = candidate.name.as_str();

        if !is_selected(name) {
            continue;
        }

        let current = candidate.version.version();

        if version_set.is_bump() {
            let base = match groups.group_of(name) {
                Some(group) => group
                    .iter()
                    .flat_map(|m| by_name.get(m.as_str())?.version.version())
                    .max(),
                None => current,
            };

            let Some(base) = base else {
                continue;
            };

            let to = version_set.bump(base);

            tracing::trace!(
                name,
                from = base.to_string(),
                to = to.to_string(),
                "Bump version"
            );

            versions.insert(
                name.to_string(),
                VersionChange {
                    old: current.unwrap_or(&to).clone(),
                    new: to,
                },
            );

            continue;
        }

        let version = overrides
            .get(name)
            .map(|(_, v)| *v)
            .or(version_set.any.as_ref());

        if let Some(version) = version {
            tracing::info!(?name, version = ?version.to_string(), "Set version");

            versions.insert(
                name.to_string(),
                VersionChange {
                    old: version.clone(),
                    new: version.clone(),
                },
            );
        }
    }

    // Crates which inherit their version from the workspace all share one
    // version, so changing one of them changes the workspace version.
    let mut workspace = None::<(&str, Version)>;

    for candidate in candidates {
        if !candidate.version.is_inherited() {
            continue;
        }

        let Some(change) = versions.get(&candidate.name) else {
            continue;
        };

        match &workspace {
            Some((other, version)) if *version != change.new => {
                bail!(
                    "Crates `{other}` and `{}` inherit their version from the workspace but would get different versions: {version} and {}",
                    candidate.name,
                    change.new
                );
            }
            Some(..) => {}
            None => {
                workspace = Some((&candidate.name, change.new.clone()));
            }
        }
    }

    let workspace = workspace.map(|(_, v)| v);

    if let Some(new) = &workspace {
        for candidate in candidates {
            if candidate.version.is_inherited() && !versions.contains_key(&candidate.name) {
                versions.insert(
                    candidate.name.clone(),
                    VersionChange {
                        old: candidate.version.version().unwrap_or(new).clone(),
                        new: new.clone(),
                    },
                );
            }
        }
    }

    Ok(Plan {
        versions,
        workspace,
    })
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
    let workspace_version = version_groups::workspace_version(workspace)?;

    let mut candidates = Vec::new();
    let mut known = BTreeSet::new();

    for manifest in workspace.manifests() {
        let Some(package) = manifest.as_package() else {
            continue;
        };

        let name = package.name()?;
        known.insert(name);

        if !package.is_publish() {
            continue;
        }

        candidates.push(Candidate {
            name: name.to_owned(),
            version: version_groups::package_version(package, workspace_version.as_ref())?,
        });
    }

    let groups = if opts.no_group {
        ResolvedGroups::default()
    } else {
        ResolvedGroups::resolve(cx.config.version_groups(repo), &known)?
    };

    let Plan {
        versions,
        workspace: new_workspace_version,
    } = plan(&candidates, &groups, filter, version_set)?;

    let mut workspace_version_updated = false;

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

                let inherited =
                    version_groups::package_version(package, None).is_ok_and(|v| v.is_inherited());

                if !inherited && package.version() != Some(version_string.as_str()) {
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

            if let Some(new) = &new_workspace_version
                && let Some(version) = workspace
                    .get_mut("package")
                    .and_then(|p| p.as_table_like_mut())
                    .and_then(|p| p.get_mut("version"))
                    .and_then(Item::as_value_mut)
                && version.as_str().is_some()
            {
                workspace_version_updated = true;
                let new = new.to_string();

                if version.as_str() != Some(new.as_str()) {
                    *version = Value::String(Formatted::new(new));
                    changed_manifest = true;
                }
            }
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

    if let Some(new) = &new_workspace_version
        && !workspace_version_updated
    {
        bail!("Cannot set workspace version to {new}: no `[workspace.package] version` found");
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

    /// Bump the given version.
    fn bump(&self, from: &Version) -> Version {
        let mut to = from.clone();

        if self.major {
            to.major += 1;
            to.minor = 0;
            to.patch = 0;
            to.pre = Prerelease::default();
        } else if self.minor {
            to.minor += 1;
            to.patch = 0;
            to.pre = Prerelease::default();
        } else if self.patch {
            to.patch += 1;
            to.pre = Prerelease::default();
        }

        if let Some(pre) = &self.pre {
            to.pre = pre.clone();
        }

        to
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
fn modify_version_req(req: &str, o: &Version, n: &Version) -> Result<String> {
    let mut req = VersionReq::parse(req)?;

    // Special case: we don't want to expand single caret requirements.
    if let &[
        Comparator {
            op: Op::Caret,
            major,
            minor: Some(minor),
            patch: Some(patch),
            ref pre,
        },
    ] = &req.comparators[..]
        && (o.major == major || o.minor == minor || o.patch == patch || o.pre != *pre)
    {
        let mut v = n.clone();
        v.build = Default::default();
        return Ok(v.to_string());
    }

    let mut modified = false;

    for c in req.comparators.iter_mut() {
        if c.major == o.major
            && c.minor.unwrap_or(0) == o.minor
            && c.patch.unwrap_or(0) == o.patch
            && c.pre == o.pre
        {
            c.major = n.major;
            c.minor = Some(n.minor);
            c.patch = Some(n.patch);
            c.pre = n.pre.clone();
            modified = true;
        }
    }

    if modified {
        return Ok(req.to_string());
    }

    // If old requirement matches, no need to modify it.
    if req.matches(n) {
        return Ok(req.to_string());
    }

    // If it doesn't match, it is weird. So just straight up replace it.
    let mut v = n.clone();
    v.build = Default::default();
    Ok(v.to_string())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};

    use semver::Version;

    use super::{Candidate, Plan, VersionChange, VersionSet, plan};
    use crate::version_groups::{PackageVersion, ResolvedGroups, VersionGroup};

    fn v(s: &str) -> Version {
        Version::parse(s).unwrap()
    }

    fn explicit(name: &str, version: &str) -> Candidate {
        Candidate {
            name: name.to_owned(),
            version: PackageVersion::Explicit(v(version)),
        }
    }

    fn inherited(name: &str, version: &str) -> Candidate {
        Candidate {
            name: name.to_owned(),
            version: PackageVersion::Inherited(Some(v(version))),
        }
    }

    fn groups(candidates: &[Candidate], groups: &[&[&str]]) -> ResolvedGroups {
        let known = candidates
            .iter()
            .map(|c| c.name.as_str())
            .collect::<BTreeSet<_>>();

        let groups = groups
            .iter()
            .map(|g| VersionGroup {
                crates: g.iter().map(|s| (*s).to_owned()).collect(),
            })
            .collect::<Vec<_>>();

        ResolvedGroups::resolve(&groups, &known).unwrap()
    }

    fn change(old: &str, new: &str) -> VersionChange {
        VersionChange {
            old: v(old),
            new: v(new),
        }
    }

    fn patch() -> VersionSet {
        VersionSet {
            patch: true,
            ..VersionSet::default()
        }
    }

    #[test]
    fn group_bump_uses_highest_version() {
        let candidates = [
            explicit("foo", "1.0.0"),
            explicit("foo-macros", "1.2.0"),
            explicit("bar", "0.1.0"),
        ];
        let groups = groups(&candidates, &[&["foo", "foo-macros"]]);
        let filter = HashSet::from(["foo"]);

        let Plan {
            versions,
            workspace,
        } = plan(&candidates, &groups, &filter, &patch()).unwrap();

        assert_eq!(versions.len(), 2);
        assert_eq!(versions["foo"], change("1.0.0", "1.2.1"));
        assert_eq!(versions["foo-macros"], change("1.2.0", "1.2.1"));
        assert_eq!(workspace, None);
    }

    #[test]
    fn no_group_bumps_individually() {
        let candidates = [explicit("foo", "1.0.0"), explicit("foo-macros", "1.2.0")];
        let groups = ResolvedGroups::default();
        let filter = HashSet::from(["foo"]);

        let plan = plan(&candidates, &groups, &filter, &patch()).unwrap();
        assert_eq!(plan.versions.len(), 1);
        assert_eq!(plan.versions["foo"], change("1.0.0", "1.0.1"));
    }

    #[test]
    fn group_override_applies_to_group() {
        let candidates = [
            explicit("foo", "1.0.0"),
            explicit("foo-macros", "1.2.0"),
            explicit("bar", "0.1.0"),
        ];
        let groups = groups(&candidates, &[&["foo", "foo-macros"]]);

        let mut set = VersionSet::default();
        set.crates.insert("foo".to_owned(), v("2.0.0"));

        let plan = plan(&candidates, &groups, &HashSet::new(), &set).unwrap();
        assert_eq!(plan.versions.len(), 2);
        assert_eq!(plan.versions["foo"].new, v("2.0.0"));
        assert_eq!(plan.versions["foo-macros"].new, v("2.0.0"));
    }

    #[test]
    fn conflicting_group_overrides() {
        let candidates = [explicit("foo", "1.0.0"), explicit("foo-macros", "1.2.0")];
        let groups = groups(&candidates, &[&["foo", "foo-macros"]]);

        let mut set = VersionSet::default();
        set.crates.insert("foo".to_owned(), v("2.0.0"));
        set.crates.insert("foo-macros".to_owned(), v("2.1.0"));

        let error = plan(&candidates, &groups, &HashSet::new(), &set).unwrap_err();
        assert!(error.to_string().contains("Conflicting"), "{error}");

        // Agreeing overrides are fine.
        set.crates.insert("foo-macros".to_owned(), v("2.0.0"));
        assert!(plan(&candidates, &groups, &HashSet::new(), &set).is_ok());
    }

    #[test]
    fn inherited_members_update_workspace() {
        let candidates = [
            inherited("foo", "1.0.0"),
            explicit("foo-macros", "1.1.0"),
            inherited("bar", "1.0.0"),
        ];
        let groups = groups(&candidates, &[&["foo", "foo-macros"]]);
        let filter = HashSet::from(["foo-macros"]);

        let plan = plan(&candidates, &groups, &filter, &patch()).unwrap();
        assert_eq!(plan.workspace, Some(v("1.1.1")));
        assert_eq!(plan.versions["foo"], change("1.0.0", "1.1.1"));
        assert_eq!(plan.versions["foo-macros"], change("1.1.0", "1.1.1"));
        // `bar` shares the workspace version, so it changes too.
        assert_eq!(plan.versions["bar"], change("1.0.0", "1.1.1"));
    }

    #[test]
    fn inherited_conflict() {
        let candidates = [
            inherited("foo", "1.0.0"),
            explicit("foo-macros", "1.1.0"),
            inherited("bar", "1.0.0"),
        ];
        let groups = groups(&candidates, &[&["foo", "foo-macros"]]);

        let error = plan(&candidates, &groups, &HashSet::new(), &patch()).unwrap_err();
        assert!(error.to_string().contains("inherit"), "{error}");
    }
}
