//! Support for groups of crates which share a single version.
//!
//! See the `[[version_group]]` configuration.

use std::collections::{BTreeSet, HashMap};

use anyhow::{Result, bail};
use semver::Version;
use toml_edit::Item;

use crate::cargo::Package;
use crate::workspace::Crates;

/// A configured group of crates which should be versioned together.
#[derive(Debug, Clone)]
pub(crate) struct VersionGroup {
    /// Package names of crates in the group.
    pub(crate) crates: Vec<String>,
}

/// Version groups resolved against the crates known in a workspace.
#[derive(Debug, Default)]
pub(crate) struct ResolvedGroups {
    /// Groups of crate names, each sorted and de-duplicated.
    groups: Vec<Vec<String>>,
    /// Map from crate name to the index of the group it belongs to.
    by_crate: HashMap<String, usize>,
}

impl ResolvedGroups {
    /// Resolve groups against the set of known crate names.
    ///
    /// Errors if a group names an unknown crate, or if a crate belongs to more
    /// than one group.
    pub(crate) fn resolve<'a, I>(groups: I, known: &BTreeSet<&str>) -> Result<Self>
    where
        I: IntoIterator<Item = &'a VersionGroup>,
    {
        let mut out = Self::default();

        for group in groups {
            let mut members = BTreeSet::new();

            for name in &group.crates {
                if !known.contains(name.as_str()) {
                    let known = known.iter().copied().collect::<Vec<_>>().join(", ");
                    bail!(
                        "[[version_group]]: unknown crate `{name}` in group [{}], known crates are: {known}",
                        group.crates.join(", ")
                    );
                }

                members.insert(name.clone());
            }

            if members.is_empty() {
                continue;
            }

            let index = out.groups.len();

            for name in &members {
                if let Some(&existing) = out.by_crate.get(name) {
                    bail!(
                        "[[version_group]]: crate `{name}` is in more than one group: [{}] and [{}]",
                        out.groups[existing].join(", "),
                        members.iter().cloned().collect::<Vec<_>>().join(", ")
                    );
                }

                out.by_crate.insert(name.clone(), index);
            }

            out.groups.push(members.into_iter().collect());
        }

        Ok(out)
    }

    /// Get the members of the group the given crate belongs to, if any.
    pub(crate) fn group_of(&self, name: &str) -> Option<&[String]> {
        let index = *self.by_crate.get(name)?;
        Some(&self.groups[index])
    }

    /// Iterate over all groups.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &[String]> {
        self.groups.iter().map(Vec::as_slice)
    }
}

/// The version of a package, accounting for workspace inheritance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PackageVersion {
    /// The version is specified directly in `package.version`.
    Explicit(Version),
    /// The version is inherited through `version.workspace = true`, with the
    /// version of the workspace if it is known.
    Inherited(Option<Version>),
    /// No version is specified.
    Missing,
}

impl PackageVersion {
    /// Get the effective version, if any.
    pub(crate) fn version(&self) -> Option<&Version> {
        match self {
            PackageVersion::Explicit(version) => Some(version),
            PackageVersion::Inherited(version) => version.as_ref(),
            PackageVersion::Missing => None,
        }
    }

    /// Test if the version is inherited from the workspace.
    pub(crate) fn is_inherited(&self) -> bool {
        matches!(self, PackageVersion::Inherited(..))
    }
}

/// Get the `[workspace.package] version` of the workspace, if any.
pub(crate) fn workspace_version(crates: &Crates) -> Result<Option<Version>> {
    for (_, workspace) in crates.workspaces() {
        if let Some(version) = workspace.package_version() {
            return Ok(Some(Version::parse(version)?));
        }
    }

    Ok(None)
}

/// Determine the version of a package.
pub(crate) fn package_version(
    package: &Package,
    workspace: Option<&Version>,
) -> Result<PackageVersion> {
    if let Some(version) = package.version() {
        return Ok(PackageVersion::Explicit(Version::parse(version)?));
    }

    let inherited = package
        .as_table()
        .get("version")
        .and_then(|item| item.get("workspace"))
        .and_then(Item::as_bool)
        .unwrap_or(false);

    if inherited {
        return Ok(PackageVersion::Inherited(workspace.cloned()));
    }

    Ok(PackageVersion::Missing)
}

/// Find crates whose version disagrees with the other members of their group.
///
/// Returns a map from the name of each disagreeing crate to the version it
/// is expected to have, which is the highest version in its group. Crates
/// without a known version are ignored.
pub(crate) fn mismatches<'a, I>(groups: &ResolvedGroups, versions: I) -> HashMap<String, Version>
where
    I: IntoIterator<Item = (&'a str, Option<&'a Version>)>,
{
    let versions = versions
        .into_iter()
        .flat_map(|(name, version)| Some((name, version?)))
        .collect::<HashMap<_, _>>();

    let mut out = HashMap::new();

    for group in groups.iter() {
        let Some(max) = group
            .iter()
            .flat_map(|m| versions.get(m.as_str()).copied())
            .max()
        else {
            continue;
        };

        for member in group {
            if let Some(&version) = versions.get(member.as_str())
                && version != max
            {
                out.insert(member.clone(), max.clone());
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use semver::Version;

    use super::{ResolvedGroups, VersionGroup};

    fn group(crates: &[&str]) -> VersionGroup {
        VersionGroup {
            crates: crates.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    #[test]
    fn resolve_groups() {
        let known = BTreeSet::from(["a", "b", "c", "d"]);
        let groups = [group(&["b", "a"]), group(&["c"])];
        let resolved = ResolvedGroups::resolve(&groups, &known).unwrap();

        assert_eq!(
            resolved.group_of("a"),
            Some(&["a".to_owned(), "b".to_owned()][..])
        );
        assert_eq!(resolved.group_of("b"), resolved.group_of("a"));
        assert_eq!(resolved.group_of("c"), Some(&["c".to_owned()][..]));
        assert_eq!(resolved.group_of("d"), None);
        assert_eq!(resolved.iter().count(), 2);
    }

    #[test]
    fn resolve_unknown_crate() {
        let known = BTreeSet::from(["a"]);
        let groups = [group(&["a", "nope"])];
        let error = ResolvedGroups::resolve(&groups, &known).unwrap_err();
        assert!(
            error.to_string().contains("unknown crate `nope`"),
            "{error}"
        );
    }

    #[test]
    fn resolve_crate_in_two_groups() {
        let known = BTreeSet::from(["a", "b", "c"]);
        let groups = [group(&["a", "b"]), group(&["b", "c"])];
        let error = ResolvedGroups::resolve(&groups, &known).unwrap_err();
        assert!(error.to_string().contains("more than one group"), "{error}");
    }

    #[test]
    fn duplicate_within_group_is_fine() {
        let known = BTreeSet::from(["a", "b"]);
        let groups = [group(&["a", "b", "a"])];
        let resolved = ResolvedGroups::resolve(&groups, &known).unwrap();
        assert_eq!(resolved.group_of("a").unwrap().len(), 2);
    }

    #[test]
    fn mismatches() {
        let known = BTreeSet::from(["a", "b", "c", "d", "e"]);
        let groups = [group(&["a", "b", "c"]), group(&["d", "e"])];
        let resolved = ResolvedGroups::resolve(&groups, &known).unwrap();

        let v1 = Version::new(1, 0, 0);
        let v2 = Version::new(1, 2, 0);

        let out = super::mismatches(
            &resolved,
            [
                ("a", Some(&v1)),
                ("b", Some(&v2)),
                ("c", None),
                ("d", Some(&v1)),
                ("e", Some(&v1)),
            ],
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out.get("a"), Some(&v2));
    }
}
