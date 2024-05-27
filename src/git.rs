use std::ffi::OsStr;
use std::fmt::Display;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::str;

use crate::process::Command;
use anyhow::{ensure, Context, Result};
use reqwest::Url;

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
        A::Item: AsRef<OsStr>,
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

        let status = Command::new(&self.command)
            .args(["diff", "--quiet", "main", "origin/main"])
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
}

pub(crate) struct DescribeTags {
    pub(crate) tag: String,
    pub(crate) offset: Option<usize>,
}

pub(crate) fn version(path: &OsStr) -> Result<Option<ExitStatus>> {
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
