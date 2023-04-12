use std::collections::{HashSet, VecDeque};

use anyhow::{anyhow, Context, Result};
use relative_path::{RelativePath, RelativePathBuf};

use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::manifest::{self, Manifest};
use crate::model::{CrateParams, Module};
use crate::rust_version::RustVersion;

/// The default name of a cargo manifest `Cargo.toml`.
pub(crate) const CARGO_TOML: &str = "Cargo.toml";

/// Load a workspace starting at the given path.
#[tracing::instrument(skip_all)]
pub(crate) fn open(cx: &Ctxt<'_>, module: &Module) -> Result<Option<Workspace>> {
    tracing::trace!("Opening workspace");

    let manifest_path = match cx.config.cargo_toml(module.path()) {
        Some(cargo_toml) => module.path().join(cargo_toml),
        None => module.path().join(CARGO_TOML),
    };

    let primary_crate = cx
        .config
        .crate_for(module.path())
        .or(module.repo().map(|repo| repo.name));

    let Some(manifest) = manifest::open(crate::utils::to_path(&manifest_path, cx.root))? else {
        return Ok(None);
    };

    let manifest_dir = manifest_path
        .parent()
        .ok_or_else(|| anyhow!("missing parent directory"))?;

    let mut queue = VecDeque::new();

    let mut visited = HashSet::new();

    queue.push_back(Package {
        manifest_dir: manifest_dir.into(),
        manifest_path: manifest_path.clone(),
        manifest,
    });

    let mut packages = Vec::new();

    while let Some(package) = queue.pop_front() {
        if !visited.insert(package.manifest_dir.clone()) {
            tracing::trace!(?package.manifest_path, "Already loaded");
            continue;
        }

        if package.manifest.is_package() {
            tracing::trace!(?package.manifest_path, name = package.manifest.crate_name()?, "Processing package");
        } else {
            tracing::trace!(?package.manifest_path, "Processing workspace manifest");
        }

        if let Some(workspace) = package.manifest.as_workspace() {
            let members = expand_members(cx, &package, workspace.members())?;

            for manifest_dir in members {
                let manifest_path = manifest_dir.join(CARGO_TOML);

                let manifest = manifest::open(crate::utils::to_path(&manifest_path, cx.root))
                    .with_context(|| anyhow!("{manifest_path}"))?
                    .with_context(|| anyhow!("{manifest_path}: missing file"))?;

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

    Ok(Some(Workspace {
        primary_crate: primary_crate.map(Box::from),
        packages,
    }))
}

fn expand_members<'a>(
    cx: &Ctxt<'_>,
    package: &Package,
    iter: impl Iterator<Item = &'a RelativePath>,
) -> Result<Vec<RelativePathBuf>> {
    let mut output = Vec::new();

    for path in iter {
        let manifest_dir = package.manifest_dir.join(path);
        let glob = Glob::new(cx.root, &manifest_dir);

        for path in glob.matcher() {
            output.push(path?);
        }
    }

    Ok(output)
}

/// A single package in the workspace.
#[derive(Debug, Clone)]
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
            .and_then(|lib| lib.get("path").and_then(toml_edit::Item::as_str))
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
    pub(crate) fn crate_params<'a>(&'a self, module: &'a Module) -> Result<CrateParams<'a>> {
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

#[derive(Debug)]
pub(crate) struct Workspace {
    primary_crate: Option<Box<str>>,
    packages: Vec<Package>,
}

impl Workspace {
    /// Test if this is a single crate workspace.
    pub(crate) fn is_single_crate(&self) -> bool {
        self.packages.len() == 1
    }

    /// Get list of packages.
    pub(crate) fn packages(&self) -> impl Iterator<Item = &Package> {
        self.packages.iter()
    }

    /// Find the primary crate in the workspace.
    pub(crate) fn primary_crate(&self) -> Result<&Package> {
        // Single package, easy to determine primary crate.
        if let [package] = &self.packages[..] {
            return Ok(package);
        }

        // Find a package which matches the name of the project.
        if let Some(name) = &self.primary_crate {
            for package in &self.packages {
                if package.manifest.crate_name()? == name.as_ref() {
                    return Ok(package);
                }
            }
        }

        let mut names = Vec::with_capacity(self.packages.len());

        for package in &self.packages {
            names.push(package.manifest.crate_name()?);
        }

        Err(anyhow!(
            "Cannot determine primary crate, candidates are: {candidates}",
            candidates = names.join(", ")
        ))
    }
}
