use std::path::Path;

use anyhow::{anyhow, Context, Result};
use relative_path::RelativePath;
use serde::Serialize;
use url::Url;

use crate::gitmodules;
use crate::rust_version::RustVersion;

/// Badge building parameters.
#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct CrateParams<'a> {
    pub(crate) repo: Option<&'a str>,
    pub(crate) name: &'a str,
    pub(crate) description: Option<&'a str>,
    pub(crate) rust_version: Option<RustVersion>,
}

impl CrateParams<'_> {
    /// Coerce into owned.
    pub(crate) fn into_owned(self) -> OwnedCrateParams {
        OwnedCrateParams {
            repo: self.repo.map(str::to_owned),
            name: self.name.to_owned(),
            description: self.description.map(str::to_owned),
            rust_version: self.rust_version,
        }
    }
}

/// Owned crate parameters.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct OwnedCrateParams {
    pub(crate) repo: Option<String>,
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

/// A git module.
#[derive(Debug, Clone)]
pub(crate) struct Module<'a> {
    pub(crate) name: &'a str,
    pub(crate) path: Option<&'a RelativePath>,
    pub(crate) url: Option<Url>,
}

impl Module<'_> {
    /// Repo name.
    pub(crate) fn repo(&self) -> Option<&str> {
        let url = self.url.as_ref()?;

        let Some("github.com") = url.domain() else {
            return None;
        };

        Some(url.path().trim_matches('/'))
    }
}

/// Load git modules.
pub(crate) fn load_gitmodules<'a>(root: &Path, buf: &'a mut Vec<u8>) -> Result<Vec<Module<'a>>> {
    /// Parse a git module.
    pub(crate) fn parse_git_module<'a>(
        parser: &mut gitmodules::Parser<'a>,
    ) -> Result<Option<Module<'a>>> {
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

        Ok(Some(Module {
            name: section.name(),
            path,
            url,
        }))
    }

    /// Parse gitmodules from the given input.
    pub(crate) fn parse_git_modules(input: &[u8]) -> Result<Vec<Module<'_>>> {
        let mut parser = gitmodules::Parser::new(input);

        let mut modules = Vec::new();

        while let Some(module) = parse_git_module(&mut parser)? {
            modules.push(module);
        }

        Ok(modules)
    }

    /// Process module information from a git repository.
    fn module_from_git(_: &Path) -> Result<Module<'static>> {
        Err(anyhow!("cannot get a module from .git repo"))
    }

    let gitmodules_path = root.join(".gitmodules");
    let mut modules = Vec::new();

    if gitmodules_path.is_file() {
        *buf = std::fs::read(root.join(".gitmodules"))?;

        modules.extend(
            parse_git_modules(buf).with_context(|| anyhow!("{}", gitmodules_path.display()))?,
        );
    } else {
        modules.push(module_from_git(&root)?);
    }

    Ok(modules)
}
