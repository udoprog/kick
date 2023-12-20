use std::path::Path;

use anyhow::{anyhow, Context, Result};

use crate::ctxt::Ctxt;
use crate::{changes, SharedOptions};

pub(crate) fn entry(cx: &mut Ctxt<'_>, shared: &SharedOptions, changes_path: &Path) -> Result<()> {
    let changes = changes::load_changes(changes_path)
        .with_context(|| anyhow!("{}", changes_path.display()))?;

    let Some(changes) = changes else {
        tracing::info!("No changes found: {}", changes_path.display());
        return Ok(());
    };

    if !shared.save {
        tracing::warn!("Not writing changes since `--save` was not specified");
    }

    for change in changes {
        crate::changes::apply(cx, &change, shared.save)?;
    }

    if shared.save {
        tracing::info!("Removing {}", changes_path.display());
        std::fs::remove_file(changes_path)
            .with_context(|| anyhow!("{}", changes_path.display()))?;
    }

    Ok(())
}
