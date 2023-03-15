use core::fmt;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, Context, Result};
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Serialize, Serializer};
use url::Url;

use crate::gitmodules;
use crate::rust_version::RustVersion;

/// Badge building parameters.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct CrateParams<'a> {
    pub(crate) repo: Option<ModuleRepo<'a>>,
    pub(crate) name: &'a str,
    pub(crate) description: Option<&'a str>,
    pub(crate) rust_version: Option<RustVersion>,
}

impl CrateParams<'_> {
    /// Coerce into owned.
    pub(crate) fn into_owned(self) -> OwnedCrateParams {
        OwnedCrateParams {
            repo: self.repo.map(ModuleRepo::into_owned),
            name: self.name.to_owned(),
            description: self.description.map(str::to_owned),
            rust_version: self.rust_version,
        }
    }
}

/// Owned crate parameters.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct OwnedCrateParams {
    pub(crate) repo: Option<OwnedModuleRepo>,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) rust_version: Option<RustVersion>,
}

/// Update parameters.
pub(crate) struct UpdateParams<'a> {
    pub(crate) license: Option<&'a str>,
    pub(crate) readme: Option<&'a str>,
    pub(crate) repository: Option<&'a str>,
    pub(crate) homepage: Option<&'a str>,
    pub(crate) documentation: Option<&'a str>,
    pub(crate) authors: &'a [String],
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct ModuleRepo<'a> {
    pub(crate) owner: &'a str,
    pub(crate) name: &'a str,
}

impl ModuleRepo<'_> {
    /// Coerce into owned variant.
    pub(crate) fn into_owned(self) -> OwnedModuleRepo {
        OwnedModuleRepo {
            owner: self.owner.into(),
            name: self.name.into(),
        }
    }
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

#[derive(Debug, Clone)]
pub(crate) struct OwnedModuleRepo {
    pub(crate) owner: Box<str>,
    pub(crate) name: Box<str>,
}

impl fmt::Display for OwnedModuleRepo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.owner, self.name)
    }
}

impl Serialize for OwnedModuleRepo {
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
pub(crate) fn load_modules(root: &Path) -> Result<Vec<Module>> {
    let gitmodules_path = root.join(".gitmodules");
    let git_path = root.join(".git");

    let mut modules = Vec::new();

    match std::fs::read(root.join(&gitmodules_path)) {
        Ok(buf) => {
            modules.extend(
                parse_git_modules(&buf)
                    .with_context(|| anyhow!("{}", gitmodules_path.display()))?,
            );
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    if git_path.is_dir() {
        modules.extend(module_from_git(root)?);
    }

    Ok(modules)
}

/// Parse a git module.
pub(crate) fn parse_git_module(parser: &mut gitmodules::Parser<'_>) -> Result<Option<Module>> {
    let mut path = None;
    let mut url = None;

    let mut section = match parser.parse_section()? {
        Some(section) => section,
        None => return Ok(None),
    };

    while let Some((key, value)) = section.next_section()? {
        match key {
            "path" => {
                let string = std::str::from_utf8(value)?;
                path = Some(RelativePath::new(string));
            }
            "url" => {
                let string = std::str::from_utf8(value)?;
                url = Some(str::parse::<Url>(string)?);
            }
            _ => {}
        }
    }

    let (Some(url), Some(path)) = (url, path) else {
        return Ok(None);
    };

    Ok(Some(Module {
        source: ModuleSource::Gitmodules,
        path: path.into(),
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
fn module_from_git(root: &Path) -> Result<Option<Module>> {
    let output = Command::new("git")
        .args(["git", "remote", "get-url", "origin"])
        .current_dir(root)
        .stdout(Stdio::piped())
        .output()?;

    if !output.status.success() {
        tracing::trace!("failed to get git remote `origin`: {}", root.display());
        return Ok(None);
    }

    let remote = String::from_utf8(output.stdout)?;
    let url = Url::parse(remote.trim())?;

    Ok(Some(Module {
        source: ModuleSource::Git,
        path: RelativePathBuf::default().into(),
        url,
    }))
}
