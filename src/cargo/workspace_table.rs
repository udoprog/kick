use relative_path::RelativePath;
use toml_edit::{Item, Table};

use crate::cargo::{DEPENDENCIES, DependenciesTable};

/// Represents the `[workspace]` section of a manifest.
#[repr(transparent)]
pub(crate) struct WorkspaceTable {
    doc: Table,
}

impl WorkspaceTable {
    pub(super) fn new(doc: &Table) -> &Self {
        // SAFETY: type is repr transparent.
        unsafe { &*(doc as *const Table as *const Self) }
    }

    /// Get list of members.
    pub(crate) fn members(&self) -> impl Iterator<Item = &RelativePath> {
        let members = self.doc.get("members").and_then(|v| v.as_array());

        members
            .into_iter()
            .flatten()
            .flat_map(|v| Some(RelativePath::new(v.as_str()?)))
    }

    /// The `[workspace.package] version` field.
    pub(crate) fn package_version(&self) -> Option<&str> {
        self.doc.get("package")?.get("version")?.as_str()
    }

    /// Workspace dependencies.
    pub(super) fn dependencies(&self) -> Option<&DependenciesTable> {
        let doc = self.doc.get(DEPENDENCIES).and_then(Item::as_table)?;
        Some(DependenciesTable::new(doc))
    }
}
