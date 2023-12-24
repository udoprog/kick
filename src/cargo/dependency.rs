use anyhow::Result;

use crate::cargo::{DependenciesTable, DependencyItem, WorkspaceTable};
use crate::workspace::{Crates, PackageValue};

/// A single declared dependency.
pub(crate) struct Dependency<'a> {
    dependency: &'a str,
    value: &'a DependencyItem,
    crates: &'a Crates,
    accessor: fn(&'a WorkspaceTable) -> Option<&'a DependenciesTable>,
}

impl<'a> Dependency<'a> {
    pub(super) fn new(
        dependency: &'a str,
        value: &'a DependencyItem,
        crates: &'a Crates,
        accessor: fn(&'a WorkspaceTable) -> Option<&'a DependenciesTable>,
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
        let optional = self.lookup(DependencyItem::package)?;

        Ok(PackageValue::new(
            optional
                .map(PackageValue::into_value)
                .unwrap_or(self.dependency),
        ))
    }

    /// Get the package name of the dependency.
    pub(crate) fn is_optional(&self) -> Result<PackageValue<bool>> {
        let optional = self.lookup(DependencyItem::is_optional)?;

        Ok(PackageValue::new(
            optional.map(PackageValue::into_value).unwrap_or(false),
        ))
    }

    /// Lookup a key related to a package.
    ///
    /// This is complicated, because it can be declared in the workplace declaration.
    pub(crate) fn lookup<V, T>(&self, get: V) -> Result<Option<PackageValue<T>>>
    where
        V: Fn(&'a DependencyItem) -> Option<T>,
    {
        if let Some(value) = get(self.value) {
            return Ok(Some(PackageValue::new(value)));
        }

        // Handle workspace dependency.
        if let Some(true) = self.value.is_workspace() {
            for (index, workspace) in self.crates.workspaces() {
                let Some(dep) = (self.accessor)(workspace).and_then(|d| d.get(self.dependency))
                else {
                    continue;
                };

                let Some(value) = get(dep) else {
                    continue;
                };

                return Ok(Some(PackageValue::workspace(index, value)));
            }
        }

        Ok(None)
    }
}
