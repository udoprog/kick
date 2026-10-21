use std::path::Path;

use anyhow::{Context, Result, anyhow};

use crate::changes;
use crate::ctxt::Ctxt;

pub(crate) fn entry(cx: &Ctxt<'_>, changes_path: &Path) -> Result<()> {
    let changes = changes::load_changes(changes_path)
        .with_context(|| anyhow!("{}", changes_path.display()))?;

    let Some(staged) = changes else {
        return Ok(());
    };

    for expected in staged.expected {
        cx.overlay.expect(expected);
    }

    for change in staged.changes {
        let what = change.change.to_string();
        cx.stage(change)
            .with_context(|| anyhow!("Staging: {what}"))?;
    }

    Ok(())
}
