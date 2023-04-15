use anyhow::Result;
use toml_edit::{Item, Value};

use crate::manifest::{ManifestDependency, Workspace, WorkspaceDependencies};
use crate::workspace::{Crates, PackageValue};

/// A single declared dependency.
pub(crate) struct Dependency<'a> {
    dependency: &'a str,
    value: &'a Item,
    crates: &'a Crates,
    accessor: fn(&Workspace<'a>) -> Option<WorkspaceDependencies<'a>>,
}

impl<'a> Dependency<'a> {
    pub(crate) fn new(
        dependency: &'a str,
        value: &'a Item,
        crates: &'a Crates,
        accessor: fn(&Workspace<'a>) -> Option<WorkspaceDependencies<'a>>,
    ) -> Self {
        Self {
            dependency,
            value,
            crates,
            accessor,
        }
    }

    /// Get the package name of the dependency.
    pub(crate) fn package(&self) -> Result<PackageValue<&'a str>> {
        let optional = self.crates.lookup_dependency_key(
            self.dependency,
            self.value,
            self.accessor,
            ManifestDependency::package,
            Value::as_str,
            "package",
        )?;

        Ok(PackageValue::from_package(
            optional
                .map(PackageValue::into_value)
                .unwrap_or(self.dependency),
        ))
    }

    /// Get the package name of the dependency.
    pub(crate) fn is_optional(&self) -> Result<PackageValue<bool>> {
        let optional = self.crates.lookup_dependency_key(
            self.dependency,
            self.value,
            self.accessor,
            ManifestDependency::is_optional,
            Value::as_bool,
            "optional",
        )?;

        Ok(PackageValue::from_package(
            optional.map(PackageValue::into_value).unwrap_or(false),
        ))
    }
}
