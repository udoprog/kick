use toml_edit::Table;

use crate::manifest::ManifestWorkspaceDependency;

/// Represents the `[dependencies]` section of a manifest.
pub(crate) struct ManifestWorkspaceDependencies<'a> {
    doc: &'a Table,
}

impl<'a> ManifestWorkspaceDependencies<'a> {
    pub(crate) fn new(doc: &'a Table) -> Self {
        Self { doc }
    }

    /// Get a dependency by its key.
    pub fn get(&self, key: &'a str) -> Option<ManifestWorkspaceDependency<'a>> {
        let value = self.doc.get(key)?;
        Some(ManifestWorkspaceDependency::new(value))
    }
}
