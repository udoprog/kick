use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::str;

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use bstr::ByteSlice;
use reqwest::Url;

use crate::env::SecretString;
use crate::process::{Command, OsArg};

/// The outcome of a merge operation.
pub(crate) struct MergeOutcome {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) success: bool,
}

#[derive(Debug)]
pub(crate) struct Git {
    pub(crate) path: PathBuf,
}

impl Git {
    #[inline]
    pub(crate) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn fetch(
        &self,
        dir: impl AsRef<Path>,
        remote: impl AsRef<str>,
        revspec: impl AsRef<str>,
    ) -> Result<bool> {
        let output = Command::new(&self.path)
            .args(["fetch", remote.as_ref(), revspec.as_ref()])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .output()?;

        if !output.status.success() {
            return Ok(true);
        }

        Ok(!output.stdout.is_empty())
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn force_checkout(&self, dir: impl AsRef<Path>, rev: impl AsRef<str>) -> Result<()> {
        let status = Command::new(&self.path)
            .args(["checkout", "--force", rev.as_ref()])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn merge_fast_forward(
        &self,
        dir: impl AsRef<Path>,
        revspec: impl AsRef<str>,
    ) -> Result<MergeOutcome> {
        let output = Command::new(&self.path)
            .args(["merge", "--ff-only", revspec.as_ref()])
            .stdin(Stdio::null())
            .current_dir(dir)
            .output()?;

        Ok(MergeOutcome {
            stdout: String::from_utf8(output.stdout)?,
            stderr: String::from_utf8(output.stderr)?,
            success: output.status.success(),
        })
    }

    /// Make a commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn add(
        &self,
        dir: impl AsRef<Path>,
        args: impl IntoIterator<Item: Into<OsArg>>,
    ) -> Result<()> {
        let status = Command::new(&self.path)
            .arg("add")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    /// Make a commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn commit(&self, dir: impl AsRef<Path>, message: impl fmt::Display) -> Result<()> {
        let status = Command::new(&self.path)
            .args(["commit", "-m"])
            .arg(message.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    /// Make a tag.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn tag(&self, dir: impl AsRef<Path>, tag: impl fmt::Display) -> Result<()> {
        let status = Command::new(&self.path)
            .args(["tag"])
            .arg(tag.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn is_cached(&self, dir: impl AsRef<Path>) -> Result<bool> {
        let status = Command::new(&self.path)
            .args(["diff", "--cached", "--exit-code", "--quiet"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        Ok(!status.success())
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn is_dirty(&self, dir: impl AsRef<Path>) -> Result<bool> {
        let output = Command::new(&self.path)
            .args(["status", "--short"])
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .output()?;

        if !output.status.success() {
            return Ok(true);
        }

        Ok(!output.stdout.is_empty())
    }

    fn remote_update(&self, dir: impl AsRef<Path>) -> Result<()> {
        tracing::info!("Updating remote");

        let status = Command::new(&self.path)
            .args(["remote", "update"])
            .stdin(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    pub(crate) fn remote_branch(&self, dir: impl AsRef<Path>) -> Result<(String, String)> {
        let output = Command::new(&self.path)
            .args(["status", "-sb"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(dir)
            .output()?;

        ensure!(output.status.success(), output.status);

        let string = std::str::from_utf8(&output.stdout)?.trim();

        // Trim "## " prefix.
        let Some(rest) = string.strip_prefix("## ") else {
            bail!("Unexpected output: {string}");
        };

        // Trim " [ahead N]" suffix.
        let rest = if let Some((head, _)) = rest.split_once(' ') {
            head
        } else {
            rest
        };

        let Some((local, remote)) = rest.split_once("...") else {
            bail!("Unexpected output: {string}");
        };

        Ok((local.to_string(), remote.to_string()))
    }

    /// Test if the local branch is outdated.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path, ?fetch))]
    pub(crate) fn is_outdated(&self, dir: impl AsRef<Path>, fetch: bool) -> Result<bool> {
        let dir = dir.as_ref();

        if fetch {
            self.remote_update(dir)?;
        }

        let (local, remote) = self.remote_branch(dir)?;

        let status = Command::new(&self.path)
            .args(["diff", "--quiet", &local, &remote])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        Ok(!status.success())
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn init(&self, dir: impl AsRef<Path>) -> Result<()> {
        let status = Command::new(&self.path)
            .args(["init"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn remote_add(
        &self,
        dir: impl AsRef<Path>,
        name: impl AsRef<str>,
        url: impl AsRef<str>,
    ) -> Result<()> {
        let status = Command::new(&self.path)
            .args(["remote", "add", name.as_ref(), url.as_ref()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn remote_get_push_url(&self, dir: impl AsRef<Path>, name: &str) -> Result<String> {
        let output = Command::new(&self.path)
            .args(["remote", "get-url", "--push", name])
            .current_dir(dir.as_ref())
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::piped())
            .output()?;

        ensure!(output.status.success(), output.status);
        let url = String::from_utf8(output.stdout)?;
        Ok(url.trim().to_owned())
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn remote_set_push_url(
        &self,
        dir: impl AsRef<Path>,
        name: impl AsRef<str>,
        url: impl AsRef<str>,
    ) -> Result<()> {
        let status = Command::new(&self.path)
            .args(["remote", "set-url", "--push", name.as_ref(), url.as_ref()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    /// Parse a commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn rev_parse(&self, dir: impl AsRef<Path>, rev: impl AsRef<str>) -> Result<String> {
        let output = Command::new(&self.path)
            .args(["rev-parse", rev.as_ref()])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(dir)
            .output()?;

        ensure!(output.status.success(), output.status);
        Ok(str::from_utf8(&output.stdout)?.trim().to_owned())
    }

    /// Get HEAD commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path, ?fetch))]
    pub(crate) fn describe_tags(
        &self,
        dir: impl AsRef<Path>,
        fetch: bool,
    ) -> Result<Option<DescribeTags>> {
        if fetch {
            self.remote_update(dir.as_ref())?;
        }

        let output = Command::new(&self.path)
            .args(["describe", "--tags"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(dir)
            .output()?;

        if !output.status.success() {
            return Ok(None);
        }

        let string = std::str::from_utf8(&output.stdout)?.trim();

        let Some(((tag, offset), _)) = string
            .rsplit_once('-')
            .and_then(|(rest, hash)| Some((rest.rsplit_once('-')?, hash)))
        else {
            return Ok(Some(DescribeTags {
                tag: string.to_string(),
                offset: None,
            }));
        };

        Ok(Some(DescribeTags {
            tag: tag.to_string(),
            offset: Some(offset.parse()?),
        }))
    }

    /// Get remote url.
    pub(crate) fn get_url(&self, dir: impl AsRef<Path>, remote: &str) -> Result<Url> {
        let output = Command::new(&self.path)
            .args(["remote", "get-url", remote])
            .current_dir(dir)
            .stdout(Stdio::piped())
            .output()?;

        ensure!(output.status.success(), output.status);
        let url = String::from_utf8(output.stdout)?;
        Ok(Url::parse(url.trim())?)
    }

    /// Get credentials.
    pub(crate) fn get_credentials(&self, host: &str, protocol: &str) -> Result<Credentials> {
        let mut child = Command::new(&self.path)
            .args(["credential-manager-core", "get"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;

        let mut stdin = child.stdin()?;

        write!(stdin, "host={host}\nprotocol={protocol}\n")?;
        drop(stdin);

        let output = child.wait_with_output()?;
        ensure!(output.status.success(), output.status);

        let mut username = None;
        let mut password = None;

        for line in output.stdout.split_str("\n") {
            if let Some((head, tail)) = line.trim().split_once_str("=") {
                match head {
                    b"protocol" => {}
                    b"host" => {}
                    b"username" => {
                        username = Some(tail.to_vec());
                    }
                    b"password" => {
                        password = Some(tail.to_vec());
                    }
                    _ => {}
                }
            }
        }

        let username = username.context("missing username")?;
        let password = password.context("missing password")?;

        let mut combined = Vec::with_capacity(username.len() + password.len() + 1);
        combined.extend_from_slice(&username);
        combined.push(b':');
        combined.extend_from_slice(&password);

        Ok(Credentials { combined })
    }
}

pub(crate) struct DescribeTags {
    pub(crate) tag: String,
    pub(crate) offset: Option<usize>,
}

/// Git credentials.
pub(crate) struct Credentials {
    combined: Vec<u8>,
}

impl Credentials {
    /// Get secret credentials.
    pub(crate) fn get(&self) -> SecretString {
        let mut string = String::new();
        STANDARD_NO_PAD.encode_string(&self.combined, &mut string);
        SecretString::new(string)
    }
}
