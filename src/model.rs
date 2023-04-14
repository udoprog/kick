use core::fmt;
use std::cell::{Cell, Ref, RefCell};
use std::ops::Deref;
use std::path::Path;
use std::rc::Rc;

use anyhow::{bail, Context, Error, Result};
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize, Serializer};
use url::Url;

use crate::ctxt::Ctxt;
use crate::git::Git;
use crate::gitmodules;
use crate::rust_version::RustVersion;
use crate::workspace::Workspace;

/// Parameters particular to a given crate.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct CrateParams<'a> {
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

/// Parameters particular to a specific module.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct RepoParams<'a> {
    #[serde(rename = "crate")]
    pub(crate) crate_params: CrateParams<'a>,
    /// Current job name.
    pub(crate) job_name: &'a str,
    /// Globally known rust versions in use.
    pub(crate) rust_versions: RenderRustVersions,
    #[serde(flatten)]
    pub(crate) variables: toml::Table,
}

impl RepoParams<'_> {
    /// Get the current crate name.
    pub(crate) fn crate_name(&self) -> &str {
        self.crate_params.name
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RepoRef {
    /// Path to module.
    path: RelativePathBuf,
    /// URL of module.
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
    pub(crate) fn require_workspace(&self, cx: &Ctxt<'_>) -> Result<Workspace> {
        let Some(workspace) = self.inner_workspace(cx)? else {
            bail!("{}: missing workspace", self.path);
        };

        Ok(workspace)
    }

    /// Open the workspace to this symbolic module.
    fn inner_workspace(&self, cx: &Ctxt<'_>) -> Result<Option<Workspace>> {
        crate::workspace::open(cx, self)
    }
}

struct RepoInner {
    /// Source of module.
    source: RepoSource,
    /// Interior module stuff.
    symbolic: RepoRef,
    /// If the module has been disabled for some reason.
    disabled: Cell<bool>,
    /// Whether we've tried to initialize the workspace.
    init: Cell<bool>,
    /// Initialized workspace.
    workspace: RefCell<Option<Workspace>>,
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
            .field("disabled", &self.inner.disabled)
            .field("init", &self.inner.init)
            .field("workspace", &self.inner.workspace)
            .finish()
    }
}

impl Repo {
    pub(crate) fn new(source: RepoSource, path: RelativePathBuf, url: Url) -> Self {
        Self {
            inner: Rc::new(RepoInner {
                source,
                symbolic: RepoRef { path, url },
                disabled: Cell::new(false),
                init: Cell::new(false),
                workspace: RefCell::new(None),
            }),
        }
    }

    /// Test if module is disabled.
    pub(crate) fn is_disabled(&self) -> bool {
        self.inner.disabled.get()
    }

    /// Set if module is disabled.
    pub(crate) fn set_disabled(&self, disabled: bool) {
        self.inner.disabled.set(disabled);
    }

    /// Get the source of a module.
    pub(crate) fn source(&self) -> &RepoSource {
        &self.inner.source
    }

    /// Try to get a workspace, if one is present in the module.
    #[tracing::instrument(skip_all, fields(source = ?self.inner.source, module = self.path().as_str()))]
    pub(crate) fn try_workspace(&self, cx: &Ctxt<'_>) -> Result<Option<Ref<'_, Workspace>>> {
        self.init_workspace(cx)?;
        Ok(Ref::filter_map(self.inner.workspace.borrow(), Option::as_ref).ok())
    }

    /// Try to get a workspace, if one is present in the module.
    #[tracing::instrument(skip_all, fields(source = ?self.inner.source, module = self.path().as_str()))]
    pub(crate) fn workspace(&self, cx: &Ctxt<'_>) -> Result<Ref<'_, Workspace>> {
        self.init_workspace(cx)?;

        if let Ok(workspace) = Ref::filter_map(self.inner.workspace.borrow(), Option::as_ref) {
            Ok(workspace)
        } else {
            bail!("missing workspace")
        }
    }

    #[tracing::instrument(skip_all)]
    fn init_workspace(&self, cx: &Ctxt<'_>) -> Result<()> {
        if !self.inner.init.get() {
            if let Some(workspace) = self.inner_workspace(cx)? {
                *self.inner.workspace.borrow_mut() = Some(workspace);
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
pub(crate) fn load_modules(root: &Path, git: Option<&Git>) -> Result<Vec<Repo>> {
    let gitmodules_path = root.join(".gitmodules");

    match std::fs::read(&gitmodules_path) {
        Ok(buf) => {
            return parse_git_modules(&buf).with_context(|| gitmodules_path.display().to_string());
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(Error::from(e)).context(gitmodules_path.display().to_string()),
    };

    let Some(git) = git else {
        return Ok(Vec::new());
    };

    let git_path = root.join(".git");

    if git_path.is_dir() {
        tracing::trace!("{}: using .git", git_path.display());

        return Ok(vec![
            module_from_git(git, &git_path).with_context(|| git_path.display().to_string())?
        ]);
    }

    Ok(Vec::new())
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
                parsed_url = Some(str::parse::<Url>(string)?);
            }
            _ => {}
        }
    }

    let (Some(url), Some(path)) = (parsed_url, parsed_path) else {
        return Ok(None);
    };

    Ok(Some(Repo::new(RepoSource::Gitmodules, path, url)))
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
fn module_from_git<P>(git: &Git, root: &P) -> Result<Repo>
where
    P: ?Sized + AsRef<Path>,
{
    let url = git.get_url(root, "origin")?;

    Ok(Repo::new(RepoSource::Git, RelativePathBuf::from("."), url))
}
