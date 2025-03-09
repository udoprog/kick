use std::cell::{Cell, UnsafeCell};
use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::ops::Deref;
use std::path::Path;
use std::rc::Rc;

use anyhow::{anyhow, bail, Context, Result};
use musli::{Decode, Encode};
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize, Serializer};
use url::Url;

use crate::cargo::RustVersion;
use crate::ctxt::Ctxt;
use crate::gitmodules;
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

#[derive(Debug, Clone, Copy)]
pub(crate) enum RepoSource {
    /// Module loaded from a .gitmodules file.
    Gitmodules,
    /// Module loaded from .git
    Git,
}

impl fmt::Display for RepoSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RepoSource::Gitmodules => write!(f, ".gitmodules"),
            RepoSource::Git => write!(f, "git repo"),
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
    /// Get path of module.
    pub(crate) fn path(&self) -> &RelativePath {
        &self.path
    }

    /// Get URL of module.
    pub(crate) fn url(&self) -> &Url {
        &self.url
    }

    /// Repo name.
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
            minute: rng.gen_range(0..60),
            hour: rng.gen_range(0..24),
            day: rng.gen_range(0..7),
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
    /// Source of module.
    source: RepoSource,
    /// Interior module stuff.
    symbolic: RepoRef,
    /// Running the repo operation errored.
    state: Cell<State>,
    /// Whether we've tried to initialize the workspace.
    init: Cell<bool>,
    /// Initialized workspace.
    crates: UnsafeCell<Option<Crates>>,
}

/// A git module.
#[derive(Clone)]
pub(crate) struct Repo {
    inner: Rc<RepoInner>,
}

impl fmt::Debug for Repo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Repo")
            .field("source", &self.inner.source)
            .field("symbolic", &self.inner.symbolic)
            .field("state", &self.inner.state)
            .field("init", &self.inner.init)
            .field("workspace", &self.inner.crates)
            .finish()
    }
}

impl Repo {
    pub(crate) fn new(source: RepoSource, path: RelativePathBuf, url: Url) -> Self {
        Self {
            inner: Rc::new(RepoInner {
                source,
                symbolic: RepoRef { path, url },
                state: Cell::new(State::Pending),
                init: Cell::new(false),
                crates: UnsafeCell::new(None),
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

    /// Get the source of a module.
    pub(crate) fn source(&self) -> &RepoSource {
        &self.inner.source
    }

    /// Get the inner workspace.
    fn get_workspace(&self) -> Option<&Crates> {
        // SAFETY: This is the only way to access this interior value.
        unsafe { (*self.inner.crates.get().cast_const()).as_ref() }
    }

    /// Try to get a workspace, if one is present in the module.
    #[tracing::instrument(skip_all, fields(source = ?self.inner.source, module = self.path().as_str()))]
    pub(crate) fn try_workspace(&self, cx: &Ctxt<'_>) -> Result<Option<&'_ Crates>> {
        self.init_workspace(cx)?;
        Ok(self.get_workspace())
    }

    /// Try to get a workspace, if one is present in the module.
    #[tracing::instrument(skip_all, fields(source = ?self.inner.source, module = self.path().as_str()))]
    pub(crate) fn workspace(&self, cx: &Ctxt<'_>) -> Result<&'_ Crates> {
        self.init_workspace(cx)?;

        let Some(workspace) = self.get_workspace() else {
            bail!("missing workspace")
        };

        Ok(workspace)
    }

    #[tracing::instrument(skip_all)]
    fn init_workspace(&self, cx: &Ctxt<'_>) -> Result<()> {
        if !self.inner.init.get() {
            if let Some(workspace) = self.inner_workspace(cx)? {
                // This is the only place where this is initialized.
                unsafe {
                    *self.inner.crates.get() = Some(workspace);
                }
            } else {
                tracing::warn!("Missing workspace for module");
            };

            self.inner.init.set(true);
        }

        Ok(())
    }
}

impl Deref for Repo {
    type Target = RepoRef;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner.symbolic
    }
}

/// Load git modules.
pub(crate) fn load_gitmodules(root: &Path) -> Result<Option<Vec<Repo>>> {
    let path = root.join(".gitmodules");

    match fs::read(&path) {
        Ok(buf) => Ok(Some(
            parse_git_modules(&buf).with_context(|| path.display().to_string())?,
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context(path.display().to_string()),
    }
}

#[tracing::instrument(skip_all, fields(root = ?root.display()))]
pub(crate) fn load_from_git(root: &Path, git: Option<&Git>) -> Result<Vec<Repo>> {
    tracing::trace!("Trying to load from git");

    let mut out = Vec::new();

    let Some(git) = git else {
        bail!("No working git command available");
    };

    let git_path = root.join(".git");

    if git_path.exists() {
        tracing::trace!("Using repository: {}", root.display());
        out.push(from_git(git, root).with_context(|| root.display().to_string())?);
        return Ok(out);
    }

    match url_from_github_action() {
        Ok(url) => {
            tracing::trace!("Using GitHub Actions URL: {url}");
            out.push(Repo::new(RepoSource::Git, RelativePathBuf::from("."), url));
            return Ok(out);
        }
        Err(error) => {
            tracing::trace!("Could not build repo from GitHub Actions");

            for error in error.chain() {
                tracing::trace!("Caused by: {error}");
            }
        }
    }

    Ok(out)
}

/// Parse a git module.
pub(crate) fn parse_git_module(parser: &mut gitmodules::Parser<'_>) -> Result<Option<Repo>> {
    let mut parsed_path = None;
    let mut parsed_url = None;

    let mut section = match parser.parse_section()? {
        Some(section) => section,
        None => return Ok(None),
    };

    while let Some((key, value)) = section.next_section()? {
        match key {
            "path" => {
                let string = std::str::from_utf8(value)?;
                parsed_path = Some(RelativePath::new(string).into());
            }
            "url" => {
                let string = std::str::from_utf8(value)?;

                let out = 'out: {
                    if let Ok(url) = str::parse::<Url>(string) {
                        break 'out url;
                    }

                    parse_remote(string)?
                };

                parsed_url = Some(out);
            }
            _ => {}
        }
    }

    let (Some(url), Some(path)) = (parsed_url, parsed_path) else {
        return Ok(None);
    };

    Ok(Some(Repo::new(RepoSource::Gitmodules, path, url)))
}

fn parse_remote(remote: &str) -> Result<Url> {
    let remote = match remote.split_once('@') {
        Some((_, remote)) => remote,
        None => remote,
    };

    let Some((domain, path)) = remote.split_once(':') else {
        bail!("Missing domain:path")
    };

    let string = format!("https://{domain}/{path}");
    Ok(Url::parse(&string)?)
}

/// Parse gitmodules from the given input.
pub(crate) fn parse_git_modules(input: &[u8]) -> Result<Vec<Repo>> {
    let mut parser = gitmodules::Parser::new(input);

    let mut modules = Vec::new();

    while let Some(module) = parse_git_module(&mut parser)? {
        modules.push(module);
    }

    Ok(modules)
}

/// Process module information from a git repository.
fn from_git<P>(git: &Git, root: P) -> Result<Repo>
where
    P: AsRef<Path>,
{
    let url = git.get_url(root, "origin")?;
    Ok(Repo::new(RepoSource::Git, RelativePathBuf::from("."), url))
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
