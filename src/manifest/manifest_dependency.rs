use anyhow::Result;
use toml_edit::{Item, Value};

use crate::manifest::{
    ManifestWorkspace, ManifestWorkspaceDependencies, ManifestWorkspaceDependency,
};
use crate::workspace::{PackageValue, Workspace};

/// A single declared dependency.
pub(crate) struct ManifestDependency<'a> {
    dependency: &'a str,
    value: &'a Item,
    workspace: &'a Workspace,
    accessor: fn(&ManifestWorkspace<'a>) -> Option<ManifestWorkspaceDependencies<'a>>,
}

impl<'a> ManifestDependency<'a> {
    pub(crate) fn new(
        dependency: &'a str,
        value: &'a Item,
        workspace: &'a Workspace,
        accessor: fn(&ManifestWorkspace<'a>) -> Option<ManifestWorkspaceDependencies<'a>>,
    ) -> Self {
        Self {
            dependency,
            value,
            workspace,
            accessor,
        }
    }

    /// Get the package name of the dependency.
    pub(crate) fn package(&self) -> Result<PackageValue<&'a str>> {
        let optional = self.workspace.lookup_dependency_key(
            self.dependency,
            self.value,
            self.accessor,
            ManifestWorkspaceDependency::package,
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
        let optional = self.workspace.lookup_dependency_key(
            self.dependency,
            self.value,
            self.accessor,
            ManifestWorkspaceDependency::is_optional,
            Value::as_bool,
            "optional",
        )?;

        Ok(PackageValue::from_package(
            optional.map(PackageValue::into_value).unwrap_or(false),
        ))
    }
}
