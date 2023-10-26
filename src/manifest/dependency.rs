use anyhow::Result;
use toml_edit::{Item, Value};

use crate::manifest::{Workspace, WorkspaceDependencies, WorkspaceDependency};
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
        let optional = self.lookup(WorkspaceDependency::package, Value::as_str, "package")?;

        Ok(PackageValue::new(
            optional
                .map(PackageValue::into_value)
                .unwrap_or(self.dependency),
        ))
    }

    /// Get the package name of the dependency.
    pub(crate) fn is_optional(&self) -> Result<PackageValue<bool>> {
        let optional = self.lookup(WorkspaceDependency::is_optional, Value::as_bool, "optional")?;

        Ok(PackageValue::new(
            optional.map(PackageValue::into_value).unwrap_or(false),
        ))
    }

    /// Lookup a key related to a package.
    ///
    /// This is complicated, because it can be declared in the workplace declaration.
    pub(crate) fn lookup<D, V, T>(
        &self,
        dep_lookup: D,
        value_map: V,
        field: &'static str,
    ) -> Result<Option<PackageValue<T>>>
    where
        V: Fn(&'a Value) -> Option<T>,
        D: Fn(&WorkspaceDependency<'a>) -> Option<T>,
    {
        if let Some(Item::Value(value)) = self.value.get(field) {
            if let Some(value) = value_map(value) {
                return Ok(Some(PackageValue::new(value)));
            }
        }

        // workspace dependency.
        if let Some(true) = self.value.get("workspace").and_then(|w| w.as_bool()) {
            for (index, workspace) in self.crates.workspaces() {
                let Some(dep) = (self.accessor)(&workspace).and_then(|d| d.get(self.dependency))
                else {
                    continue;
                };

                let Some(value) = dep_lookup(&dep) else {
                    continue;
                };

                return Ok(Some(PackageValue::workspace(index, value)));
            }
        }

        Ok(None)
    }
}
