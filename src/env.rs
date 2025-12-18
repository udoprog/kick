use std::convert::Infallible;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader};
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};

#[derive(Debug, Clone)]
pub(crate) enum GithubTokenSource {
    Environment,
    CommandLine,
    Path(Box<Path>),
}

#[derive(Debug, Clone)]
pub(crate) struct GithubToken {
    pub(crate) source: GithubTokenSource,
    pub(crate) secret: SecretString,
}

impl GithubToken {
    #[inline]
    pub(crate) fn env(secret: SecretString) -> Self {
        Self {
            source: GithubTokenSource::Environment,
            secret,
        }
    }

    #[inline]
    pub(crate) fn cli(secret: SecretString) -> Self {
        Self {
            source: GithubTokenSource::CommandLine,
            secret,
        }
    }

    #[inline]
    pub(crate) fn path(path: impl AsRef<Path>, secret: SecretString) -> Self {
        Self {
            source: GithubTokenSource::Path(path.as_ref().into()),
            secret,
        }
    }
}

#[derive(Debug)]
pub(crate) struct Env {
    pub(crate) kick_version: Option<String>,
    pub(crate) github_event_name: Option<String>,
    pub(crate) github_ref: Option<String>,
    pub(crate) github_sha: Option<String>,
    pub(crate) github_tokens: Vec<GithubToken>,
}

impl Env {
    pub(crate) fn new() -> Self {
        Self {
            kick_version: None,
            github_event_name: None,
            github_ref: None,
            github_sha: None,
            github_tokens: Vec::new(),
        }
    }

    pub(crate) fn update_from_env(&mut self) {
        self.kick_version = env::var("KICK_VERSION").ok().filter(|e| !e.is_empty());
        self.github_event_name = env::var("GITHUB_EVENT_NAME").ok().filter(|e| !e.is_empty());
        self.github_ref = env::var("GITHUB_REF").ok().filter(|e| !e.is_empty());
        self.github_sha = env::var("GITHUB_SHA").ok().filter(|e| !e.is_empty());
        self.github_tokens.extend(
            env::var("GITHUB_TOKEN")
                .ok()
                .filter(|e| !e.is_empty())
                .map(|s| GithubToken::env(SecretString::new(s))),
        );
    }

    /// The tag GITHUB_REF refers to.
    pub(crate) fn github_tag(&self) -> Option<&str> {
        self.github_ref.as_ref()?.strip_prefix("refs/tags/")
    }

    /// The head GITHUB_REF refers to.
    pub(crate) fn github_head(&self) -> Option<&str> {
        self.github_ref.as_ref()?.strip_prefix("refs/heads/")
    }
}

/// A string which prevents itself from being accidentally printed.
#[derive(Clone)]
pub(crate) struct SecretString(String);

impl SecretString {
    #[inline]
    pub(crate) const fn new(s: String) -> Self {
        Self(s)
    }

    #[inline]
    pub(crate) fn as_secret(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretString {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl FromStr for SecretString {
    type Err = Infallible;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self::new(s.to_owned()))
    }
}

/// Helper to optionally read a secret string without leaking it.
pub(crate) fn read_secret_string(path: impl AsRef<Path>) -> Result<Option<SecretString>> {
    let path = path.as_ref();
    read_secret_string_inner(path).with_context(|| path.display().to_string())
}

fn read_secret_string_inner(path: &Path) -> Result<Option<SecretString>> {
    let f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let mut line = String::new();

    let mut f = BufReader::new(f);

    loop {
        line.clear();

        let n = f.read_line(&mut line)?;

        if n == 0 {
            break;
        }

        let line = line.trim();

        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        return Ok(Some(SecretString::new(line.to_owned())));
    }

    Ok(None)
}
