use std::iter::from_fn;

use toml_edit::Table;

use crate::manifest::{Dependency, Workspace, WorkspaceDependencies};
use crate::workspace::Crates;

/// Represents the `[dependencies]` section of a manifest.
pub(crate) struct Dependencies<'a> {
    doc: &'a Table,
    crates: &'a Crates,
    accessor: fn(&Workspace<'a>) -> Option<WorkspaceDependencies<'a>>,
}

impl<'a> Dependencies<'a> {
    pub(crate) fn new(
        doc: &'a Table,
        crates: &'a Crates,
        accessor: fn(&Workspace<'a>) -> Option<WorkspaceDependencies<'a>>,
    ) -> Self {
        Self {
            doc,
            crates,
            accessor,
        }
    }

    /// Test if the dependencies section is empty.
    pub fn is_empty(&self) -> bool {
        self.doc.is_empty()
    }

    /// Iterate over dependencies.
    pub fn iter(&self) -> impl Iterator<Item = Dependency<'a>> + 'a {
        let mut iter = self.doc.iter();
        let workspace = self.crates;
        let accessor = self.accessor;

        from_fn(move || {
            let (key, value) = iter.next()?;
            Some(Dependency::new(key, value, workspace, accessor))
        })
    }
}
