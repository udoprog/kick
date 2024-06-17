use std::ffi::OsStr;
use std::fmt::Display;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::str;

use anyhow::{bail, ensure, Context, Result};
use base64::engine::general_purpose::STANDARD_NO_PAD;
use base64::Engine;
use bstr::ByteSlice;
use reqwest::Url;

use crate::env::SecretString;
use crate::process::{Command, OsArg};

#[derive(Debug)]
pub(crate) struct Git {
    command: PathBuf,
}

impl Git {
    #[inline]
    pub(crate) fn new(command: PathBuf) -> Self {
        Self { command }
    }

    /// Make a commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn add<P, A>(&self, dir: &P, args: A) -> Result<()>
    where
        P: ?Sized + AsRef<Path>,
        A: IntoIterator,
        A::Item: Into<OsArg>,
    {
        let status = Command::new(&self.command)
            .arg("add")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    /// Make a commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn commit<P, M>(&self, dir: &P, message: M) -> Result<()>
    where
        P: ?Sized + AsRef<Path>,
        M: Display,
    {
        let status = Command::new(&self.command)
            .args(["commit", "-m"])
            .arg(message.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    /// Make a tag.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn tag<P, M>(&self, dir: &P, tag: M) -> Result<()>
    where
        P: ?Sized + AsRef<Path>,
        M: Display,
    {
        let status = Command::new(&self.command)
            .args(["tag"])
            .arg(tag.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn is_cached<P>(&self, dir: &P) -> Result<bool>
    where
        P: ?Sized + AsRef<Path>,
    {
        let status = Command::new(&self.command)
            .args(["diff", "--cached", "--exit-code", "--quiet"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        Ok(!status.success())
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn is_dirty<P>(&self, dir: &P) -> Result<bool>
    where
        P: ?Sized + AsRef<Path>,
    {
        let output = Command::new(&self.command)
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

    fn remote_update<P>(&self, dir: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        tracing::info!("Updating remote");

        let status = Command::new(&self.command)
            .args(["remote", "update"])
            .stdin(Stdio::null())
            .current_dir(dir)
            .status()?;

        ensure!(status.success(), status);
        Ok(())
    }

    pub(crate) fn remote_branch<P>(&self, dir: P) -> Result<(String, String)>
    where
        P: AsRef<Path>,
    {
        let output = Command::new(&self.command)
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
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command, ?fetch))]
    pub(crate) fn is_outdated<P>(&self, dir: P, fetch: bool) -> Result<bool>
    where
        P: AsRef<Path>,
    {
        let dir = dir.as_ref();

        if fetch {
            self.remote_update(dir)?;
        }

        let (local, remote) = self.remote_branch(dir)?;

        let status = Command::new(&self.command)
            .args(["diff", "--quiet", &local, &remote])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        Ok(!status.success())
    }

    /// Parse a commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn rev_parse<P>(&self, dir: P, rev: &str) -> Result<String>
    where
        P: AsRef<Path>,
    {
        let output = Command::new(&self.command)
            .args(["rev-parse", rev])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(dir)
            .output()?;

        ensure!(output.status.success(), output.status);
        Ok(str::from_utf8(&output.stdout)?.trim().to_owned())
    }

    /// Get HEAD commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command, ?fetch))]
    pub(crate) fn describe_tags<P>(&self, dir: &P, fetch: bool) -> Result<Option<DescribeTags>>
    where
        P: ?Sized + AsRef<Path>,
    {
        if fetch {
            self.remote_update(dir)?;
        }

        let output = Command::new(&self.command)
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
    pub(crate) fn get_url<P>(&self, dir: P, remote: &str) -> Result<Url>
    where
        P: AsRef<Path>,
    {
        let output = Command::new(&self.command)
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
        let mut child = Command::new(&self.command)
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

pub(crate) fn test(path: &OsStr) -> Result<Option<ExitStatus>> {
    match std::process::Command::new(path)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) => Ok(Some(status)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context("git --version"),
    }
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
