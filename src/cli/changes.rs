use std::path::Path;

use anyhow::{Context, Result, anyhow};

use crate::changes;
use crate::ctxt::Ctxt;

pub(crate) fn entry(cx: &Ctxt<'_>, changes_path: &Path) -> Result<()> {
    let changes = changes::load_changes(changes_path)
        .with_context(|| anyhow!("{}", changes_path.display()))?;

    if let Some(changes) = changes {
        *cx.changes.borrow_mut() = changes;
    }

    Ok(())
}
