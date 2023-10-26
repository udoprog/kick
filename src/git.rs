use std::env::consts::EXE_EXTENSION;
use std::ffi::OsStr;
use std::fmt::Display;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use crate::process::Command;
use anyhow::{anyhow, Context, Result};
use reqwest::Url;

#[cfg(windows)]
const PATH_SEP: char = ';';
#[cfg(not(windows))]
const PATH_SEP: char = ':';

#[derive(Debug)]
pub(crate) struct Git {
    command: PathBuf,
}

impl Git {
    /// Try to find a working git command.
    pub(crate) fn find() -> Result<Option<Self>> {
        if let Some(path) = std::env::var_os("GIT_PATH").and_then(|path| path.into_string().ok()) {
            if let Some(status) = git_version(&path)? {
                if status.success() {
                    return Ok(Some(Self {
                        command: path.into(),
                    }));
                }
            }
        }

        let Some(path) = std::env::var_os("PATH").and_then(|path| path.into_string().ok()) else {
            return Ok(None);
        };

        // Look for the command in the PATH.
        for path in path.split(PATH_SEP) {
            let command = Path::new(path).join("git").with_extension(EXE_EXTENSION);

            if let Some(status) = git_version(&command)? {
                if status.success() {
                    return Ok(Some(Self { command }));
                }
            }
        }

        Ok(None)
    }

    /// Make a commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn add<P, A>(&self, dir: &P, args: A) -> Result<()>
    where
        P: ?Sized + AsRef<Path>,
        A: IntoIterator,
        A::Item: AsRef<OsStr>,
    {
        let output = Command::new(&self.command)
            .arg("add")
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null())
            .current_dir(dir)
            .output()?;

        if !output.status.success() {
            return Err(anyhow!("status: {}", output.status));
        }

        Ok(())
    }

    /// Make a commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn commit<P, M>(&self, dir: &P, message: M) -> Result<()>
    where
        P: ?Sized + AsRef<Path>,
        M: Display,
    {
        tracing::trace!("git commit -m \"{message}\"");

        let output = Command::new(&self.command)
            .args(["commit", "-m"])
            .arg(message.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null())
            .current_dir(dir)
            .output()?;

        if !output.status.success() {
            return Err(anyhow!("status: {}", output.status));
        }

        Ok(())
    }

    /// Make a tag.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn tag<P, M>(&self, dir: &P, tag: M) -> Result<()>
    where
        P: ?Sized + AsRef<Path>,
        M: Display,
    {
        tracing::trace!("git tag \"{tag}\"");

        let output = Command::new(&self.command)
            .args(["tag"])
            .arg(tag.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::inherit())
            .stderr(Stdio::null())
            .current_dir(dir)
            .output()?;

        if !output.status.success() {
            return Err(anyhow!("status: {}", output.status));
        }

        Ok(())
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn is_cached<P>(&self, dir: &P) -> Result<bool>
    where
        P: ?Sized + AsRef<Path>,
    {
        tracing::trace!("git diff --cached --exit-code --quiet");

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
        tracing::trace!("git diff --quiet");

        let status = Command::new(&self.command)
            .args(["diff", "--quiet"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .current_dir(dir)
            .status()?;

        Ok(!status.success())
    }

    /// Get HEAD commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn rev_parse<P>(&self, dir: &P, rev: &str) -> Result<String>
    where
        P: ?Sized + AsRef<Path>,
    {
        tracing::trace!("git rev-parse {rev}");

        let output = Command::new(&self.command)
            .args(["rev-parse", rev])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .current_dir(dir)
            .output()?;

        if !output.status.success() {
            return Err(anyhow!("status: {}", output.status));
        }

        Ok(String::from_utf8(output.stdout)?)
    }

    /// Get HEAD commit.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.command))]
    pub(crate) fn describe_tags<P>(
        &self,
        dir: &P,
    ) -> Result<Option<(String, Option<(usize, String)>)>>
    where
        P: ?Sized + AsRef<Path>,
    {
        tracing::trace!("git describe --tags");

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

        let Some(((tag, count), hash)) = string
            .rsplit_once('-')
            .and_then(|(rest, hash)| Some((rest.rsplit_once('-')?, hash)))
        else {
            return Ok(Some((string.to_string(), None)));
        };

        Ok(Some((
            tag.to_string(),
            Some((count.parse()?, hash.to_string())),
        )))
    }

    /// Get remote url.
    pub(crate) fn get_url<P>(&self, dir: &P, remote: &str) -> Result<Url>
    where
        P: ?Sized + AsRef<Path>,
    {
        let output = Command::new(&self.command)
            .args(["remote", "get-url", remote])
            .current_dir(dir)
            .stdout(Stdio::piped())
            .output()?;

        anyhow::ensure!(
            output.status.success(),
            "failed to get git remote `{remote}`"
        );

        let url = String::from_utf8(output.stdout)?;
        Ok(Url::parse(url.trim())?)
    }
}

fn git_version<P>(path: &P) -> Result<Option<ExitStatus>>
where
    P: ?Sized + AsRef<Path>,
{
    match Command::new(path.as_ref())
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
