use std::path::{Path, PathBuf};

use anyhow::{Result, ensure};

use crate::config::Distribution;
use crate::process::Command;

#[derive(Debug)]
pub(crate) struct Wsl {
    pub(crate) path: PathBuf,
}

impl Wsl {
    #[inline]
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Set up a WSL shell command.
    pub(crate) fn shell<D>(&self, dir: D, dist: Distribution) -> Command
    where
        D: AsRef<Path>,
    {
        let mut command = Command::new(&self.path);
        command.args(["--shell-type", "login"]);

        if dist != Distribution::Other {
            let dist = dist.to_string();
            command.args(["-d", &dist]);
        }

        command.current_dir(dir);
        command
    }

    /// List all WSL distributions.
    pub(crate) async fn list(&self) -> Result<Vec<String>> {
        let output = Command::new(&self.path)
            .args(["--list", "--quiet"])
            .output()
            .await?;

        ensure!(output.status.success(), output.status);

        let string = decode_utf16(&output.stdout)?;
        Ok(string
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

fn decode_utf16(bytes: &[u8]) -> Result<String> {
    let it = bytes.chunks_exact(2).flat_map(|s| match s {
        &[a, b] => Some(u16::from_ne_bytes([a, b])),
        _ => None,
    });

    let mut string = String::with_capacity(bytes.len() / 2);

    for c in char::decode_utf16(it) {
        string.push(c?);
    }

    Ok(string)
}
