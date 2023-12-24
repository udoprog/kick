use std::iter::from_fn;

use toml_edit::Table;

use crate::cargo::{DependenciesTable, Dependency, WorkspaceTable};
use crate::workspace::Crates;

use super::DependencyItem;

/// Represents the `[dependencies]` section of a manifest.
pub(crate) struct Dependencies<'a> {
    doc: &'a Table,
    crates: &'a Crates,
    accessor: fn(&'a WorkspaceTable) -> Option<&'a DependenciesTable>,
}

impl<'a> Dependencies<'a> {
    pub(super) fn new(
        doc: &'a Table,
        crates: &'a Crates,
        accessor: fn(&'a WorkspaceTable) -> Option<&'a DependenciesTable>,
    ) -> Self {
        Self {
            doc,
            crates,
            accessor,
        }
    }

    /// Test if the dependencies section is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.doc.is_empty()
    }

    /// Iterate over dependencies.
    pub(crate) fn iter(&self) -> impl Iterator<Item = Dependency<'a>> + 'a {
        let mut iter = self.doc.iter();
        let workspace = self.crates;
        let accessor = self.accessor;

        from_fn(move || {
            let (key, value) = iter.next()?;
            let value = DependencyItem::new(value);
            Some(Dependency::new(key, value, workspace, accessor))
        })
    }
}
