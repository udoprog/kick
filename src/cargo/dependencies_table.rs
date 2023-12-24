use toml_edit::Table;

use crate::cargo::DependencyItem;

/// Represents the `[dependencies]` section of a manifest.
#[repr(transparent)]
pub(super) struct DependenciesTable {
    doc: Table,
}

impl DependenciesTable {
    pub(super) fn new(doc: &Table) -> &Self {
        // SAFETY: type is repr transparent.
        unsafe { &*(doc as *const Table as *const Self) }
    }

    /// Get a dependency by its key.
    pub(super) fn get(&self, key: &str) -> Option<&DependencyItem> {
        Some(DependencyItem::new(self.doc.get(key)?))
    }
}
