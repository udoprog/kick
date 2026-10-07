use std::fmt;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::str;

use anyhow::{Context, Result, ensure};
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

    /// Run git with the given arguments in `dir`, capturing its output.
    ///
    /// Errors raised while running the command name the command and the
    /// directory it was run in.
    fn run(&self, dir: &Path, args: &[&str]) -> Result<Output> {
        Command::new(&self.path)
            .args(args.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .current_dir(dir)
            .output()
            .with_context(|| running(dir, args))
    }

    /// Run git with the given arguments in `dir`, capturing its output and
    /// erroring with what git printed to stderr if it fails.
    fn run_checked(&self, dir: &Path, args: &[&str]) -> Result<Output> {
        let output = self.run(dir, args)?;

        if !output.status.success() {
            return Err(failed(&output)).with_context(|| running(dir, args));
        }

        Ok(output)
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn is_cached(&self, dir: impl AsRef<Path>) -> Result<bool> {
        let dir = dir.as_ref();
        let args = ["diff", "--cached", "--exit-code", "--quiet"];
        let output = self.run(dir, &args)?;

        // NB: With `--exit-code`, git exits with 1 if there are differences
        // and with some other non-zero code on errors.
        match output.status.code() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(failed(&output)).with_context(|| running(dir, &args)),
        }
    }

    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn is_dirty(&self, dir: impl AsRef<Path>) -> Result<bool> {
        let dir = dir.as_ref();
        let args = ["status", "--short"];
        let output = self.run(dir, &args)?;

        if !output.status.success() {
            tracing::warn!(
                "{}, treating it as dirty: {}",
                running(dir, &args),
                failed(&output)
            );
            return Ok(true);
        }

        Ok(!output.stdout.is_empty())
    }

    fn remote_update(&self, dir: &Path) -> Result<()> {
        tracing::info!("Updating remote");

        let args = ["remote", "update"];

        let status = Command::new(&self.path)
            .args(args)
            .stdin(Stdio::null())
            .current_dir(dir)
            .status()
            .with_context(|| running(dir, &args))?;

        ensure!(status.success(), "{}: {status}", running(dir, &args));
        Ok(())
    }

    /// Get the current local branch and the upstream branch it tracks.
    ///
    /// Returns `None` if `HEAD` is detached, the branch has no commits yet,
    /// or it doesn't track an upstream branch which exists.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path))]
    pub(crate) fn upstream(&self, dir: impl AsRef<Path>) -> Result<Option<Upstream>> {
        let dir = dir.as_ref();
        let args = [
            "status",
            "--porcelain=v2",
            "--branch",
            "--untracked-files=no",
        ];
        let output = self.run_checked(dir, &args)?;

        let stdout = str::from_utf8(&output.stdout)
            .context("Output is not valid UTF-8")
            .with_context(|| running(dir, &args))?;

        Ok(parse_upstream(stdout))
    }

    /// Test if the local branch is outdated.
    ///
    /// A branch without an upstream branch to compare against is not
    /// outdated.
    #[tracing::instrument(skip_all, fields(dir = ?dir.as_ref(), command = ?self.path, ?fetch))]
    pub(crate) fn is_outdated(&self, dir: impl AsRef<Path>, fetch: bool) -> Result<bool> {
        let dir = dir.as_ref();

        if fetch {
            self.remote_update(dir)?;
        }

        let Some(Upstream { local, remote }) = self.upstream(dir)? else {
            tracing::debug!(
                "{}: No upstream branch to compare against, not outdated",
                absolute(dir).display()
            );
            return Ok(false);
        };

        let args = ["diff", "--quiet", &local, &remote];
        let output = self.run(dir, &args)?;

        match output.status.code() {
            Some(0) => Ok(false),
            Some(1) => Ok(true),
            _ => Err(failed(&output)).with_context(|| running(dir, &args)),
        }
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
        let dir = dir.as_ref();

        if fetch {
            self.remote_update(dir)?;
        }

        let args = ["describe", "--tags"];
        let output = self.run(dir, &args)?;

        if !output.status.success() {
            tracing::trace!("{}: {}", running(dir, &args), failed(&output));
            return Ok(None);
        }

        let string = std::str::from_utf8(&output.stdout)
            .context("Output is not valid UTF-8")
            .with_context(|| running(dir, &args))?
            .trim();

        let Some(((tag, offset), _)) = string
            .rsplit_once('-')
            .and_then(|(rest, hash)| Some((rest.rsplit_once('-')?, hash)))
        else {
            return Ok(Some(DescribeTags {
                tag: string.to_string(),
                offset: None,
            }));
        };

        let offset = offset
            .parse()
            .with_context(|| format!("Parsing offset in `{string}`"))
            .with_context(|| running(dir, &args))?;

        Ok(Some(DescribeTags {
            tag: tag.to_string(),
            offset: Some(offset),
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
        let url = url.trim();
        parse_url(url).with_context(|| format!("Parsing url of remote `{remote}`: {url}"))
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

/// A local branch and the upstream branch it tracks.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Upstream {
    pub(crate) local: String,
    pub(crate) remote: String,
}

/// Parse the upstream branch from the output of `git status --porcelain=v2
/// --branch`.
///
/// The relevant header lines are:
///
/// ```text
/// # branch.oid <commit> | (initial)
/// # branch.head <branch> | (detached)
/// # branch.upstream <upstream>
/// # branch.ab +<ahead> -<behind>
/// ```
///
/// Where `branch.upstream` is only present if an upstream is configured, and
/// `branch.ab` only if the upstream branch also exists.
fn parse_upstream(output: &str) -> Option<Upstream> {
    let mut initial = false;
    let mut head = None;
    let mut upstream = None;
    let mut ab = false;

    for line in output.lines() {
        let Some(header) = line.strip_prefix("# ") else {
            continue;
        };

        let (key, value) = header.split_once(' ').unwrap_or((header, ""));

        match key {
            "branch.oid" => initial = value == "(initial)",
            "branch.head" => head = Some(value),
            "branch.upstream" => upstream = Some(value),
            "branch.ab" => ab = true,
            _ => {}
        }
    }

    let head = head.filter(|h| !h.is_empty() && *h != "(detached)")?;
    let upstream = upstream.filter(|u| !u.is_empty())?;

    if initial || !ab {
        return None;
    }

    Some(Upstream {
        local: head.to_owned(),
        remote: upstream.to_owned(),
    })
}

/// Describe running git with the given arguments in `dir`.
fn running(dir: &Path, args: &[&str]) -> String {
    format!(
        "Running `git {}` in {}",
        args.join(" "),
        absolute(dir).display()
    )
}

/// Make `dir` absolute for diagnostics.
fn absolute(dir: &Path) -> PathBuf {
    std::path::absolute(dir).unwrap_or_else(|_| dir.to_owned())
}

/// Build an error describing a failed git command from its output.
fn failed(output: &Output) -> anyhow::Error {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();

    if stderr.is_empty() {
        anyhow::anyhow!("git failed: {}", output.status)
    } else {
        anyhow::anyhow!("git failed: {}: {stderr}", output.status)
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

/// Parse a git url.
///
/// In addition to regular urls, this supports the scp-like syntax used by git
/// remotes, such as `git@github.com:udoprog/kick`, which is translated into an
/// equivalent `https` url.
pub(crate) fn parse_url(string: &str) -> Result<Url> {
    // Git treats a url as scp-like if the first colon is not preceded by a
    // slash and is not part of a `://` scheme separator.
    if let Some((head, tail)) = string.split_once(':')
        && !head.contains('/')
        && !tail.starts_with("//")
    {
        let host = match head.split_once('@') {
            Some((_, host)) => host,
            None => head,
        };

        ensure!(!host.is_empty(), "Missing host in `[<user>@]<host>:<path>`");

        let path = tail.trim_start_matches('/');
        ensure!(!path.is_empty(), "Missing path in `[<user>@]<host>:<path>`");

        let string = format!("https://{host}/{path}");
        return Url::parse(&string).with_context(|| format!("Parsing scp-like url as `{string}`"));
    }

    Url::parse(string).context(
        "Expected a url such as `https://github.com/udoprog/kick`, or an scp-like remote such as `git@github.com:udoprog/kick`",
    )
}

#[cfg(test)]
mod tests {
    use super::{Git, Upstream, parse_upstream, parse_url};
    use crate::worktree::tests::git;

    fn upstream(local: &str, remote: &str) -> Option<Upstream> {
        Some(Upstream {
            local: local.to_owned(),
            remote: remote.to_owned(),
        })
    }

    #[test]
    fn is_outdated_against_local_upstream() {
        let tmp = tempfile::tempdir().unwrap();
        let base = tmp.path();
        let upstream = base.join("upstream.git");
        let local = base.join("local");
        let other = base.join("other");

        git(base, &["init", "--bare", upstream.to_str().unwrap()]);
        git(base, &["init", local.to_str().unwrap()]);

        let g = Git::new("git".into());

        // No commits yet.
        assert!(!g.is_outdated(&local, false).unwrap());

        git(&local, &["commit", "--allow-empty", "-m", "first"]);

        // No upstream configured.
        assert!(g.upstream(&local).unwrap().is_none());
        assert!(!g.is_outdated(&local, false).unwrap());

        git(
            &local,
            &["remote", "add", "origin", upstream.to_str().unwrap()],
        );
        git(&local, &["push", "-u", "origin", "main"]);
        assert!(!g.is_outdated(&local, false).unwrap());

        git(
            base,
            &["clone", upstream.to_str().unwrap(), other.to_str().unwrap()],
        );
        std::fs::write(other.join("file"), "content").unwrap();
        git(&other, &["add", "file"]);
        git(&other, &["commit", "-m", "second"]);
        git(&other, &["push", "origin", "main"]);
        git(&local, &["fetch", "origin"]);

        assert_eq!(
            g.upstream(&local).unwrap(),
            Some(Upstream {
                local: "main".into(),
                remote: "origin/main".into(),
            })
        );
        assert!(g.is_outdated(&local, false).unwrap());

        // Detached head.
        git(&local, &["checkout", "--detach"]);
        assert!(!g.is_outdated(&local, false).unwrap());

        // A missing directory names the command and the directory.
        let missing = base.join("missing");
        let error = g.is_outdated(&missing, false).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("git status"), "{error}");
        assert!(error.contains(missing.to_str().unwrap()), "{error}");

        // Not a repository includes git's stderr.
        let error = g.is_outdated(base, false).unwrap_err();
        let error = format!("{error:#}");
        assert!(error.contains("not a git repository"), "{error}");
    }

    #[test]
    fn test_parse_upstream() {
        // Tracking an upstream, with changed files.
        assert_eq!(
            parse_upstream(
                "# branch.oid 0a9e5a9bc4efde31e2cd7a84af8ec9e65ea5de4c\n\
                 # branch.head main\n\
                 # branch.upstream origin/main\n\
                 # branch.ab +1 -2\n\
                 1 .M N... 100644 100644 100644 abc abc src/main.rs\n"
            ),
            upstream("main", "origin/main")
        );

        // Branch names may contain slashes.
        assert_eq!(
            parse_upstream(
                "# branch.oid 0a9e5a9bc4efde31e2cd7a84af8ec9e65ea5de4c\n\
                 # branch.head card/5b485ad1\n\
                 # branch.upstream upstream/card/5b485ad1\n\
                 # branch.ab +0 -0\n"
            ),
            upstream("card/5b485ad1", "upstream/card/5b485ad1")
        );

        // No upstream configured (`## main` in `git status -sb`).
        assert_eq!(
            parse_upstream(
                "# branch.oid 0a9e5a9bc4efde31e2cd7a84af8ec9e65ea5de4c\n\
                 # branch.head main\n"
            ),
            None
        );

        // Upstream configured but gone (`## main...origin/main [gone]`).
        assert_eq!(
            parse_upstream(
                "# branch.oid 0a9e5a9bc4efde31e2cd7a84af8ec9e65ea5de4c\n\
                 # branch.head main\n\
                 # branch.upstream origin/main\n"
            ),
            None
        );

        // No commits yet (`## No commits yet on main`).
        assert_eq!(
            parse_upstream("# branch.oid (initial)\n# branch.head main\n"),
            None
        );

        // Detached head (`## HEAD (no branch)`).
        assert_eq!(
            parse_upstream(
                "# branch.oid 0a9e5a9bc4efde31e2cd7a84af8ec9e65ea5de4c\n\
                 # branch.head (detached)\n"
            ),
            None
        );

        assert_eq!(parse_upstream(""), None);
    }

    #[test]
    fn test_parse_url() {
        macro_rules! assert_url {
            ($string:expr, $expected:expr) => {
                assert_eq!(parse_url($string).unwrap().as_str(), $expected);
            };
        }

        assert_url!(
            "https://github.com/udoprog/kick",
            "https://github.com/udoprog/kick"
        );
        assert_url!(
            "ssh://git@github.com/udoprog/kick.git",
            "ssh://git@github.com/udoprog/kick.git"
        );
        assert_url!(
            "git@github.com:udoprog/kick",
            "https://github.com/udoprog/kick"
        );
        assert_url!(
            "git@github.com:udoprog/kick.git",
            "https://github.com/udoprog/kick.git"
        );
        assert_url!("github.com:udoprog/kick", "https://github.com/udoprog/kick");
        assert_url!(
            "git@github.com:/udoprog/kick",
            "https://github.com/udoprog/kick"
        );

        assert_url!(
            "@github.com:udoprog/kick",
            "https://github.com/udoprog/kick"
        );

        assert!(parse_url("git@github.com:").is_err());
        assert!(parse_url("git@:udoprog/kick").is_err());
        assert!(parse_url("/local/path/repo").is_err());
    }
}
