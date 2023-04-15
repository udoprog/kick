use std::iter::from_fn;

use toml_edit::Table;

use crate::manifest::{ManifestDependency, ManifestWorkspace, ManifestWorkspaceDependencies};
use crate::workspace::Workspace;

/// Represents the `[dependencies]` section of a manifest.
pub(crate) struct ManifestDependencies<'a> {
    doc: &'a Table,
    workspace: &'a Workspace,
    accessor: fn(&ManifestWorkspace<'a>) -> Option<ManifestWorkspaceDependencies<'a>>,
}

impl<'a> ManifestDependencies<'a> {
    pub(crate) fn new(
        doc: &'a Table,
        workspace: &'a Workspace,
        accessor: fn(&ManifestWorkspace<'a>) -> Option<ManifestWorkspaceDependencies<'a>>,
    ) -> Self {
        Self {
            doc,
            workspace,
            accessor,
        }
    }

    /// Test if the dependencies section is empty.
    pub fn is_empty(&self) -> bool {
        self.doc.is_empty()
    }

    /// Iterate over dependencies.
    pub fn iter(&self) -> impl Iterator<Item = ManifestDependency<'a>> + 'a {
        let mut iter = self.doc.iter();
        let workspace = self.workspace;
        let accessor = self.accessor;

        from_fn(move || {
            let (key, value) = iter.next()?;
            Some(ManifestDependency::new(key, value, workspace, accessor))
        })
    }
}
