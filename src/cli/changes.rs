use anyhow::{Context, Result, anyhow};

use crate::changes::ChangesFile;
use crate::ctxt::Ctxt;

pub(crate) fn entry(cx: &Ctxt<'_>, changes: &ChangesFile) -> Result<()> {
    let Some(staged) = changes.load()? else {
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
