use std::collections::VecDeque;

use crate::ctxt::Ctxt;
use crate::manifest::{self, Manifest};
use crate::model::{CrateParams, Module};
use crate::rust_version::RustVersion;
use anyhow::{anyhow, Context, Result};
use relative_path::{RelativePath, RelativePathBuf};

/// The default name of a cargo manifest `Cargo.toml`.
pub(crate) const CARGO_TOML: &str = "Cargo.toml";

/// Load a workspace starting at the given path.
pub(crate) fn open(cx: &Ctxt<'_>, module: &Module<'_>) -> Result<Workspace> {
    let path = module.path.ok_or_else(|| anyhow!("missing module path"))?;

    let manifest_path = match cx.config.cargo_toml(module.name) {
        Some(cargo_toml) => path.join(cargo_toml),
        None => path.join(CARGO_TOML),
    };

    let primary_crate = cx
        .config
        .crate_for(module.name)
        .or(module.repo().and_then(|repo| repo.split('/').next_back()));

    let manifest = manifest::open(manifest_path.to_path(cx.root))?;

    let manifest_dir = manifest_path
        .parent()
        .ok_or_else(|| anyhow!("missing parent directory"))?;

    let mut queue = VecDeque::new();

    queue.push_back(Package {
        manifest_dir: manifest_dir.to_owned(),
        manifest_path: manifest_path.to_owned(),
        manifest,
    });

    let mut packages = Vec::new();

    while let Some(package) = queue.pop_front() {
        if let Some(workspace) = package.manifest.as_workspace() {
            let members = expand_members(cx, &package, workspace.members())?;

            for manifest_dir in members {
                let manifest_path = manifest_dir.join(CARGO_TOML);

                let manifest = manifest::open(manifest_path.to_path(cx.root))
                    .with_context(|| anyhow!("{manifest_path}"))?;

                queue.push_back(Package {
                    manifest_dir,
                    manifest_path,
                    manifest,
                });
            }
        }

        if package.manifest.is_package() {
            packages.push(package);
        }
    }

    Ok(Workspace {
        path: path.into(),
        primary_crate: primary_crate.map(Box::from),
        packages,
    })
}

fn expand_members<'a>(
    cx: &Ctxt<'_>,
    package: &Package,
    iter: impl Iterator<Item = &'a RelativePath>,
) -> Result<Vec<RelativePathBuf>> {
    let mut output = Vec::new();
    let mut queue = VecDeque::new();

    for path in iter {
        queue.push_back(package.manifest_dir.join(path));
    }

    'outer: while let Some(p) = queue.pop_front() {
        let mut current = RelativePathBuf::new();
        let mut it = p.components();

        while let Some(c) = it.next() {
            if c.as_str() == "*" {
                let dirs = std::fs::read_dir(current.to_path(cx.root))?;

                for e in dirs {
                    let e = e?;

                    if let Some(c) = e.file_name().to_str() {
                        let mut new = current.clone();
                        new.push(c);
                        new.push(it.as_relative_path());
                        queue.push_back(new);
                    }
                }

                continue 'outer;
            }

            current.push(c.as_str());
        }

        output.push(current);
    }

    Ok(output)
}

/// A single package in the workspace.
#[derive(Clone)]
pub(crate) struct Package {
    pub(crate) manifest_dir: RelativePathBuf,
    pub(crate) manifest_path: RelativePathBuf,
    pub(crate) manifest: Manifest,
}

impl Package {
    /// Find the location of the entrypoint `lib.rs`.
    pub(crate) fn entries(&self) -> Vec<RelativePathBuf> {
        if let Some(path) = self
            .manifest
            .lib()
            .and_then(|lib| lib.get("path").and_then(|p| p.as_str()))
        {
            vec![self.manifest_dir.join(path)]
        } else {
            vec![
                self.manifest_dir.join("src").join("lib.rs"),
                self.manifest_dir.join("src").join("main.rs"),
            ]
        }
    }

    /// Construct crate parameters.
    pub(crate) fn crate_params<'a>(&'a self, module: &'a Module<'_>) -> Result<CrateParams<'a>> {
        Ok(CrateParams {
            repo: module.repo(),
            name: self.manifest.crate_name()?,
            description: self.manifest.description()?,
            rust_version: self.rust_version()?,
        })
    }

    /// Rust versions for a specific manifest.
    pub(crate) fn rust_version(&self) -> Result<Option<RustVersion>> {
        Ok(match self.manifest.rust_version()? {
            Some(rust_version) => RustVersion::parse(rust_version),
            None => None,
        })
    }
}

pub(crate) struct Workspace {
    path: Box<RelativePath>,
    primary_crate: Option<Box<str>>,
    packages: Vec<Package>,
}

impl Workspace {
    /// Get the workspace path.
    pub(crate) fn path(&self) -> &RelativePath {
        &self.path
    }

    /// Test if this is a single crate workspace.
    pub(crate) fn is_single_crate(&self) -> bool {
        self.packages.len() == 1
    }

    /// Get list of packages.
    pub(crate) fn packages(&self) -> impl Iterator<Item = &Package> {
        self.packages.iter()
    }

    /// Mutable list of packages.
    pub(crate) fn packages_mut(&mut self) -> impl Iterator<Item = &mut Package> {
        self.packages.iter_mut()
    }

    /// Find the primary crate in the workspace.
    pub(crate) fn primary_crate(&self) -> Result<Option<&Package>> {
        // Single package, easy to determine primary crate.
        if let [package] = &self.packages[..] {
            return Ok(Some(package));
        }

        // Find a package which matches the name of the project.
        if let Some(name) = &self.primary_crate {
            for package in &self.packages {
                if package.manifest.crate_name()? == name.as_ref() {
                    return Ok(Some(package));
                }
            }
        }

        Ok(None)
    }
}
