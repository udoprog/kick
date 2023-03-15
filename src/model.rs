use core::fmt;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};
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
pub(crate) fn load_modules(root: &Path, path: &RelativePath) -> Result<Vec<Module>> {
    let gitmodules_path = path.join(".gitmodules");
    let git_path = path.join(".git");

    let mut modules = Vec::new();

    match std::fs::read(gitmodules_path.to_path(root)) {
        Ok(buf) => {
            modules
                .extend(parse_git_modules(path, &buf).with_context(|| gitmodules_path.to_owned())?);
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }

    if git_path.to_path(root).is_dir() {
        modules.extend(module_from_git(path.to_path(root)).with_context(|| path.to_owned())?);
    }

    Ok(modules)
}

/// Parse a git module.
pub(crate) fn parse_git_module(
    path: &RelativePath,
    parser: &mut gitmodules::Parser<'_>,
) -> Result<Option<Module>> {
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
                parsed_path = Some(path.join(string).into());
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
pub(crate) fn parse_git_modules(path: &RelativePath, input: &[u8]) -> Result<Vec<Module>> {
    let mut parser = gitmodules::Parser::new(input);

    let mut modules = Vec::new();

    while let Some(module) = parse_git_module(path, &mut parser)? {
        modules.push(module);
    }

    Ok(modules)
}

/// Process module information from a git repository.
fn module_from_git<P>(root: P) -> Result<Option<Module>>
where
    P: AsRef<Path>,
{
    let output = Command::new("git")
        .args(["git", "remote", "get-url", "origin"])
        .current_dir(root)
        .stdout(Stdio::piped())
        .output()?;

    if !output.status.success() {
        tracing::trace!("failed to get git remote `origin`");
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
