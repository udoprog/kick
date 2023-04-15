use relative_path::RelativePath;
use toml_edit::{Item, Table};

use crate::manifest::{
    ManifestWorkspaceDependencies, BUILD_DEPENDENCIES, DEPENDENCIES, DEV_DEPENDENCIES,
};

/// Represents the `[workspace]` section of a manifest.
pub(crate) struct ManifestWorkspace<'a> {
    doc: &'a Table,
}

impl<'a> ManifestWorkspace<'a> {
    pub(crate) fn new(doc: &'a Table) -> Self {
        Self { doc }
    }

    /// Get list of members.
    pub(crate) fn members(&self) -> impl Iterator<Item = &'a RelativePath> {
        let members = self.doc.get("members").and_then(|v| v.as_array());

        members
            .into_iter()
            .flatten()
            .flat_map(|v| Some(RelativePath::new(v.as_str()?)))
    }

    /// Workspace dependencies.
    pub(crate) fn dependencies(&self) -> Option<ManifestWorkspaceDependencies<'a>> {
        let doc = self.doc.get(DEPENDENCIES).and_then(Item::as_table)?;
        Some(ManifestWorkspaceDependencies::new(doc))
    }

    /// Workspace dev-dependencies.
    pub(crate) fn dev_dependencies(&self) -> Option<ManifestWorkspaceDependencies<'a>> {
        let doc = self.doc.get(DEV_DEPENDENCIES).and_then(Item::as_table)?;
        Some(ManifestWorkspaceDependencies::new(doc))
    }

    /// Workspace dev-dependencies.
    pub(crate) fn build_dependencies(&self) -> Option<ManifestWorkspaceDependencies<'a>> {
        let doc = self.doc.get(BUILD_DEPENDENCIES).and_then(Item::as_table)?;
        Some(ManifestWorkspaceDependencies::new(doc))
    }
}
