use std::env;

#[derive(Debug)]
pub(crate) struct Env {
    pub(crate) kick_version: Option<String>,
    pub(crate) github_event_name: Option<String>,
    pub(crate) github_ref: Option<String>,
    pub(crate) github_sha: Option<String>,
}

impl Env {
    pub(crate) fn new() -> Self {
        let kick_version = env::var("KICK_VERSION").ok().filter(|e| !e.is_empty());
        let github_event_name = env::var("GITHUB_EVENT_NAME").ok().filter(|e| !e.is_empty());
        let github_ref = env::var("GITHUB_REF").ok().filter(|e| !e.is_empty());
        let github_sha = env::var("GITHUB_SHA").ok().filter(|e| !e.is_empty());

        Self {
            kick_version,
            github_event_name,
            github_ref,
            github_sha,
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
