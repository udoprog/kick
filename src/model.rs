use core::fmt;
use std::path::Path;

use anyhow::{Context, Error, Result};
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Serialize, Serializer};
use url::Url;

use crate::git::Git;
use crate::gitmodules;
use crate::rust_version::RustVersion;

/// Parameters particular to a given crate.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct CrateParams<'a> {
    pub(crate) name: &'a str,
    pub(crate) repo: Option<ModuleRepo<'a>>,
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
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct ModuleParams<'a> {
    #[serde(rename = "crate")]
    pub(crate) crate_params: CrateParams<'a>,
    /// Current job name.
    pub(crate) job_name: &'a str,
    /// Globally known rust versions in use.
    pub(crate) rust_versions: RenderRustVersions,
    #[serde(flatten)]
    pub(crate) variables: &'a toml::Table,
}

impl ModuleParams<'_> {
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
pub(crate) struct ModuleRepo<'a> {
    pub(crate) owner: &'a str,
    pub(crate) name: &'a str,
}

impl fmt::Display for ModuleRepo<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl Serialize for ModuleRepo<'_> {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ModuleSource {
    /// Module loaded from a .gitmodules file.
    Gitmodules,
    /// Module loaded from .git
    Git,
}

/// A git module.
#[derive(Debug, Clone)]
pub(crate) struct Module {
    pub(crate) source: ModuleSource,
    pub(crate) path: Box<RelativePath>,
    pub(crate) url: Url,
}

impl Module {
    /// Repo name.
    pub(crate) fn repo(&self) -> Option<ModuleRepo<'_>> {
        let Some("github.com") = self.url.domain() else {
            return None;
        };

        let path = self.url.path().trim_matches('/');
        let (owner, name) = path.split_once('/')?;
        Some(ModuleRepo { owner, name })
    }
}

/// Load git modules.
pub(crate) fn load_modules(root: &Path, git: Option<&Git>) -> Result<Vec<Module>> {
    let gitmodules_path = root.join(".gitmodules");
    let git_path = root.join(".git");

    let mut modules = Vec::new();

    let result = match std::fs::read(&gitmodules_path) {
        Ok(buf) => {
            modules.extend(parse_git_modules(&buf)?);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Error::from(e)),
    };

    result.with_context(|| gitmodules_path.display().to_string())?;

    if let Some(git) = git {
        if git_path.is_dir() {
            tracing::trace!("{}: found .git", git_path.display());
            modules.extend(
                module_from_git(git, &git_path).with_context(|| git_path.display().to_string())?,
            );
        }
    }

    Ok(modules)
}

/// Parse a git module.
pub(crate) fn parse_git_module(parser: &mut gitmodules::Parser<'_>) -> Result<Option<Module>> {
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

    Ok(Some(Module {
        source: ModuleSource::Gitmodules,
        path,
        url,
    }))
}

/// Parse gitmodules from the given input.
pub(crate) fn parse_git_modules(input: &[u8]) -> Result<Vec<Module>> {
    let mut parser = gitmodules::Parser::new(input);

    let mut modules = Vec::new();

    while let Some(module) = parse_git_module(&mut parser)? {
        modules.push(module);
    }

    Ok(modules)
}

/// Process module information from a git repository.
fn module_from_git<P>(git: &Git, root: &P) -> Result<Option<Module>>
where
    P: ?Sized + AsRef<Path>,
{
    let url = match git.get_url(root, "origin") {
        Ok(url) => url,
        Err(error) => {
            tracing::trace!("{error}");
            return Ok(None);
        }
    };

    Ok(Some(Module {
        source: ModuleSource::Git,
        path: RelativePathBuf::default().into(),
        url,
    }))
}
