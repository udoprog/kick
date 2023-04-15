use toml_edit::Table;

use crate::manifest::WorkspaceDependency;

/// Represents the `[dependencies]` section of a manifest.
pub(crate) struct WorkspaceDependencies<'a> {
    doc: &'a Table,
}

impl<'a> WorkspaceDependencies<'a> {
    pub(crate) fn new(doc: &'a Table) -> Self {
        Self { doc }
    }

    /// Get a dependency by its key.
    pub fn get(&self, key: &'a str) -> Option<WorkspaceDependency<'a>> {
        let value = self.doc.get(key)?;
        Some(WorkspaceDependency::new(value))
    }
}
