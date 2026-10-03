use std::cell::{Cell, OnceCell};
use std::collections::BTreeSet;
use std::env;
use std::fmt;
use std::ops::Deref;
use std::path::Path;
use std::rc::Rc;

use anyhow::{Context, Result, anyhow, bail};
use musli::{Decode, Encode};
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize, Serializer};
use url::Url;

use crate::cargo::RustVersion;
use crate::ctxt::Ctxt;
use crate::system::Git;
use crate::workspace::Crates;

/// Parameters particular to a given package.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct PackageParams<'a> {
    pub(crate) name: &'a str,
    pub(crate) repo: Option<RepoPath<'a>>,
    pub(crate) description: Option<&'a str>,
    pub(crate) rust_version: Option<RustVersion>,
}

/// Global version parameters.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct RenderRustVersions {
    pub(crate) rustc: Option<RustVersion>,
    pub(crate) edition_2018: RustVersion,
    pub(crate) edition_2021: RustVersion,
}

/// Global version parameters.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct Random {
    /// A random minute.
    pub(crate) minute: u8,
    /// A random hour, ranging from 0 to 23.
    pub(crate) hour: u8,
    /// A random day of the week, ranging from 0 to 6.
    pub(crate) day: u8,
}

/// Parameters particular to a specific module.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RepoParams<'a> {
    #[serde(rename = "package")]
    pub(crate) package_params: PackageParams<'a>,
    /// Globally known rust versions in use.
    pub(crate) rust_versions: RenderRustVersions,
    /// Some pseudo-random variables.
    pub(crate) random: Random,
    #[serde(flatten)]
    pub(crate) variables: toml::Table,
}

impl RepoParams<'_> {
    /// Get the current crate name.
    pub(crate) fn name(&self) -> &str {
        self.package_params.name
    }
}

/// Update parameters.
pub(crate) struct UpdateParams<'a> {
    pub(crate) license: Option<&'a str>,
    pub(crate) readme: Option<&'a str>,
    pub(crate) repository: Option<&'a str>,
    pub(crate) homepage: Option<&'a str>,
    pub(crate) documentation: Option<&'a str>,
    pub(crate) authors: Vec<String>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct RepoPath<'a> {
    pub(crate) owner: &'a str,
    pub(crate) name: &'a str,
}

impl fmt::Display for RepoPath<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl Serialize for RepoPath<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Encode, Decode)]
pub(crate) enum RepoSource {
    /// Module loaded from local .git
    Git,
    /// Module loaded from configuration.
    Config(#[musli(with = musli::serde)] RelativePathBuf),
}

impl fmt::Display for RepoSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RepoSource::Git => write!(f, "git repo"),
            RepoSource::Config(path) => write!(f, "{path}"),
        }
    }
}

#[derive(Debug, Clone, Encode, Decode, Serialize, Deserialize)]
pub(crate) struct RepoRef {
    /// Path to module.
    #[musli(with = musli::serde)]
    path: RelativePathBuf,
    /// URL of module.
    #[musli(with = musli::serde)]
    url: Url,
}

impl RepoRef {
    pub(crate) fn path(&self) -> &RelativePath {
        &self.path
    }

    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    pub(crate) fn push_url(&self) -> Option<String> {
        match self.url.host_str() {
            Some("github.com") => self.github_push_url(),
            _ => None,
        }
    }

    fn github_push_url(&self) -> Option<String> {
        if !matches!(self.url.scheme(), "https" | "http") {
            return None;
        }

        Some(format!(
            "git@github.com:{}.git",
            self.url.path().trim_matches('/')
        ))
    }

    pub(crate) fn repo(&self) -> Option<RepoPath<'_>> {
        let Some("github.com") = self.url.domain() else {
            return None;
        };

        let path = self.url.path().trim_matches('/');
        let (owner, name) = path.split_once('/')?;
        Some(RepoPath { owner, name })
    }

    /// Require that the workspace exists and can be opened.
    pub(crate) fn require_workspace(&self, cx: &Ctxt<'_>) -> Result<Crates> {
        let Some(workspace) = self.inner_workspace(cx)? else {
            bail!("{}: missing workspace", self.path);
        };

        Ok(workspace)
    }

    /// Generate random variables which are consistent for a given repo name.
    pub(crate) fn random(&self) -> Random {
        use rand::prelude::*;

        let mut state = 0u64;

        for c in self.path().as_str().chars() {
            state = state.wrapping_shl(11);
            state ^= c as u64;
        }

        let mut rng = rand::rngs::StdRng::seed_from_u64(state);

        Random {
            minute: rng.random_range(0..60),
            hour: rng.random_range(0..24),
            day: rng.random_range(0..7),
        }
    }

    /// Open the workspace to this symbolic module.
    fn inner_workspace(&self, cx: &Ctxt<'_>) -> Result<Option<Crates>> {
        crate::workspace::open(cx, self)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum State {
    /// The repository is pending.
    Pending,
    /// The repository operation exited successfully.
    Success,
    /// An error occured while processing the repository.
    Error,
    /// The repository is disabled.
    Disabled,
}

struct RepoInner {
    /// Sources of module.
    sources: BTreeSet<RepoSource>,
    /// Interior module stuff.
    symbolic: RepoRef,
    /// Running the repo operation errored.
    state: Cell<State>,
    /// Initialized workspace, set once we've successfully tried to open it.
    crates: OnceCell<Option<Crates>>,
}

/// A git module.
#[derive(Clone)]
pub(crate) struct Repo {
    inner: Rc<RepoInner>,
}

impl fmt::Debug for Repo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Repo")
            .field("sources", &self.inner.sources)
            .field("symbolic", &self.inner.symbolic)
            .field("state", &self.inner.state)
            .field("init", &self.inner.crates.get().is_some())
            .field("workspace", &self.inner.crates.get().map(Option::is_some))
            .finish()
    }
}

impl Repo {
    pub(crate) fn new(
        sources: impl IntoIterator<Item = RepoSource>,
        path: RelativePathBuf,
        url: Url,
    ) -> Self {
        Self {
            inner: Rc::new(RepoInner {
                sources: sources.into_iter().collect(),
                symbolic: RepoRef { path, url },
                state: Cell::new(State::Pending),
                crates: OnceCell::new(),
            }),
        }
    }

    /// Test if module is disabled.
    pub(crate) fn is_disabled(&self) -> bool {
        matches!(self.inner.state.get(), State::Disabled)
    }

    /// Set if module is disabled.
    pub(crate) fn disable(&self) {
        self.inner.state.set(State::Disabled);
    }

    /// Set the repo as errored.
    pub(crate) fn set_error(&self) {
        self.inner.state.set(State::Error);
    }

    /// Set the repo as succeeded.
    pub(crate) fn set_success(&self) {
        self.inner.state.set(State::Success);
    }

    /// Test if module errored.
    pub(crate) fn state(&self) -> State {
        self.inner.state.get()
    }

    /// Iterate over the sources of a module.
    pub(crate) fn source_list(&self) -> impl Iterator<Item = &RepoSource> {
        self.inner.sources.iter()
    }

    /// Get the sources of a module.
    pub(crate) fn sources(&self) -> impl fmt::Display + '_ {
        struct DisplaySources<'a>(&'a BTreeSet<RepoSource>);

        impl fmt::Display for DisplaySources<'_> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                let mut first = true;

                for source in self.0 {
                    if !first {
                        write!(f, ", ")?;
                    }

                    write!(f, "{source}")?;
                    first = false;
                }

                Ok(())
            }
        }

        DisplaySources(&self.inner.sources)
    }

    /// Try to get a workspace, if one is present in the module.
    #[tracing::instrument(skip_all, fields(sources = ?self.inner.sources, module = self.path().as_str()))]
    pub(crate) fn try_workspace(&self, cx: &Ctxt<'_>) -> Result<Option<&'_ Crates>> {
        self.init_workspace(cx)
    }

    /// Try to get a workspace, if one is present in the module.
    #[tracing::instrument(skip_all, fields(sources = ?self.inner.sources, module = self.path().as_str()))]
    pub(crate) fn workspace(&self, cx: &Ctxt<'_>) -> Result<&'_ Crates> {
        let Some(workspace) = self.init_workspace(cx)? else {
            bail!("missing workspace")
        };

        Ok(workspace)
    }

    #[tracing::instrument(skip_all)]
    fn init_workspace(&self, cx: &Ctxt<'_>) -> Result<Option<&Crates>> {
        if let Some(crates) = self.inner.crates.get() {
            return Ok(crates.as_ref());
        }

        // Errors leave the cell unset so that opening is retried.
        let crates = self.inner_workspace(cx)?;

        if crates.is_none() {
            tracing::warn!("Missing workspace for module");
        }

        Ok(self.inner.crates.get_or_init(|| crates).as_ref())
    }
}

impl Deref for Repo {
    type Target = RepoRef;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner.symbolic
    }
}

#[tracing::instrument(skip_all, fields(root = ?root.display()))]
pub(crate) fn load_from_git(
    root: &Path,
    git: Option<&Git>,
) -> Result<Option<(RelativePathBuf, Url)>> {
    tracing::trace!("Trying to load from git");

    let git_path = root.join(".git");

    if git_path.exists() {
        let git = git.context("no working git command available")?;
        tracing::trace!("Using repository: {}", root.display());
        return Ok(Some(from_git(git, root).with_context(|| {
            format!("Loading repository from git: {}", root.display())
        })?));
    }

    match url_from_github_action() {
        Ok(url) => {
            tracing::trace!("Using GitHub Actions URL: {url}");
            return Ok(Some((RelativePathBuf::from("."), url)));
        }
        Err(error) => {
            tracing::trace!("Could not build repo from GitHub Actions");

            for error in error.chain() {
                tracing::trace!("Caused by: {error}");
            }
        }
    }

    Ok(None)
}

/// Process module information from a git repository.
fn from_git<P>(git: &Git, root: P) -> Result<(RelativePathBuf, Url)>
where
    P: AsRef<Path>,
{
    let url = git.get_url(root, "origin")?;
    Ok((RelativePathBuf::from("."), url))
}

fn url_from_github_action() -> Result<Url> {
    let server_url = env::var_os("GITHUB_SERVER_URL").context("Missing GITHUB_SERVER_URL")?;
    let server_url = server_url
        .to_str()
        .context("GITHUB_SERVER_URL is not a legal string")?;

    let repo = env::var_os("GITHUB_REPOSITORY").context("Missing GITHUB_REPOSITORY")?;
    let repo = repo
        .to_str()
        .context("GITHUB_REPOSITORY is not a legal string")?;

    let mut url = Url::parse(server_url).context("Parsing URL")?;

    {
        let mut path = url
            .path_segments_mut()
            .map_err(|_| anyhow!("Not a legal URL"))?;
        path.extend(repo.split('/'));
    }

    Ok(url)
}
