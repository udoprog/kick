use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};

/// Move a path from one location to another.
pub(crate) fn move_paths(from: &Path, to: &Path) -> Result<()> {
    tracing::debug!("moving {} -> {}", from.display(), to.display());

    if to.exists() {
        match fs::remove_file(to) {
            Ok(()) => {
                tracing::debug!("Removed existing file: {}", to.display());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                tracing::debug!("File not found, nothing to remove: {}", to.display());
            }
            Err(e) => return Err(e).context(anyhow!("{}", to.display())),
        }
    }

    if let Err(e) = fs::rename(from, to) {
        return Err(e).context(anyhow!("{} -> {}", from.display(), to.display()));
    }

    Ok(())
}
