use std::convert::Infallible;
use std::env;
use std::fmt;
use std::fs;
use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, Result};

#[derive(Debug)]
pub(crate) struct Env {
    pub(crate) kick_version: Option<String>,
    pub(crate) github_event_name: Option<String>,
    pub(crate) github_ref: Option<String>,
    pub(crate) github_sha: Option<String>,
    pub(crate) github_token: Option<SecretString>,
}

impl Env {
    pub(crate) fn new() -> Self {
        let kick_version = env::var("KICK_VERSION").ok().filter(|e| !e.is_empty());
        let github_event_name = env::var("GITHUB_EVENT_NAME").ok().filter(|e| !e.is_empty());
        let github_ref = env::var("GITHUB_REF").ok().filter(|e| !e.is_empty());
        let github_sha = env::var("GITHUB_SHA").ok().filter(|e| !e.is_empty());
        let github_token = env::var("GITHUB_TOKEN")
            .ok()
            .filter(|e| !e.is_empty())
            .map(SecretString::new);

        Self {
            kick_version,
            github_event_name,
            github_ref,
            github_sha,
            github_token,
        }
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
pub(crate) fn read_secret_string<P>(path: P) -> Result<Option<SecretString>>
where
    P: AsRef<Path>,
{
    let path = path.as_ref();

    match fs::read_to_string(path) {
        Ok(auth) => {
            let auth = auth.trim();

            if auth.is_empty() {
                return Ok(None);
            }

            Ok(Some(SecretString::new(auth.to_owned())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => return Err(anyhow::Error::from(e)).with_context(|| path.display().to_string()),
    }
}
