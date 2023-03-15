use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Result};

/// Get HEAD commit.
pub(crate) fn rev_parse(current_dir: &Path, rev: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", rev])
        .stdout(Stdio::piped())
        .current_dir(&current_dir)
        .output()?;

    if !output.status.success() {
        return Err(anyhow!("status: {}", output.status));
    }

    Ok(String::from_utf8(output.stdout)?)
}
