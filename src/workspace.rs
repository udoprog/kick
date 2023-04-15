use std::collections::{HashSet, VecDeque};
use std::ops::Deref;

use anyhow::{anyhow, Context, Result};
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize};
use toml_edit::{Item, Value};

use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::manifest::{
    self, Manifest, ManifestDependencies, ManifestDependency, ManifestWorkspace,
};
use crate::model::{CrateParams, RepoRef};
use crate::rust_version::RustVersion;

/// The default name of a cargo manifest `Cargo.toml`.
pub(crate) const CARGO_TOML: &str = "Cargo.toml";

/// Load a workspace starting at the given path.
#[tracing::instrument(skip_all)]
pub(crate) fn open(cx: &Ctxt<'_>, repo: &RepoRef) -> Result<Option<Workspace>> {
    tracing::trace!("Opening workspace");

    let manifest_path = match cx.config.cargo_toml(repo.path()) {
        Some(cargo_toml) => repo.path().join(cargo_toml),
        None => repo.path().join(CARGO_TOML),
    };

    let primary_crate = cx
        .config
        .crate_for(repo.path())
        .or(repo.repo().map(|repo| repo.name));

    let Some(manifest) = manifest::open(manifest_path.to_path(cx.root))? else {
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

    let mut manifests = Vec::new();
    let mut workspaces = Vec::new();
    let mut packages = Vec::new();

    while let Some(package) = queue.pop_front() {
        if !visited.insert(package.manifest_dir.clone()) {
            tracing::trace!(?package.manifest_path, "Already loaded");
            continue;
        }

        if package.manifest.is_package() {
            packages.push(manifests.len());
        }

        tracing::trace!(?package.manifest_path, name = package.manifest.crate_name()?, "Processing package");

        if let Some(workspace) = package.manifest.as_workspace() {
            let members = expand_members(cx, &package, workspace.members())?;

            for manifest_dir in members {
                let manifest_path = manifest_dir.join(CARGO_TOML);

                let manifest = manifest::open(manifest_path.to_path(cx.root))
                    .with_context(|| anyhow!("{manifest_path}"))?
                    .with_context(|| anyhow!("{manifest_path}: missing file"))?;

                queue.push_back(Package {
                    manifest_dir,
                    manifest_path,
                    manifest,
                });
            }

            workspaces.push(manifests.len());
        }

        manifests.push(package);
    }

    Ok(Some(Workspace {
        primary_crate: primary_crate.map(Box::from),
        manifests,
        packages,
        workspaces,
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub(crate) fn crate_params<'a>(&'a self, repo: &'a RepoRef) -> Result<CrateParams<'a>> {
        Ok(CrateParams {
            name: self.manifest.crate_name()?,
            repo: repo.repo(),
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
    /// List of loaded packages and their associated manifests.
    manifests: Vec<Package>,
    /// Index of manifests which have a [package] declaration in them.
    packages: Vec<usize>,
    /// Index of manifests which have a [workspace] declaration in them.
    workspaces: Vec<usize>,
}

impl Workspace {
    /// Test if this is a single crate workspace.
    pub(crate) fn is_single_crate(&self) -> bool {
        self.packages.len() == 1
    }

    /// Get list of packages.
    pub(crate) fn packages(&self) -> impl Iterator<Item = &Package> {
        self.packages
            .iter()
            .flat_map(|&index| self.manifests.get(index))
    }

    /// Find the primary crate in the workspace.
    pub(crate) fn primary_crate(&self) -> Result<&Package> {
        // Single package, easy to determine primary crate.
        if let &[index] = &self.packages[..] {
            let package = self.manifests.get(index).context("missing package")?;
            return Ok(package);
        }

        // Find a package which matches the name of the project.
        if let Some(name) = &self.primary_crate {
            for &index in &self.packages {
                let package = self.manifests.get(index).context("missing package")?;

                if package.manifest.crate_name()? == name.as_ref() {
                    return Ok(package);
                }
            }
        }

        let mut names = Vec::with_capacity(self.manifests.len());

        for package in &self.manifests {
            names.push(package.manifest.crate_name()?);
        }

        Err(anyhow!(
            "Cannot determine primary crate, candidates are: {candidates}",
            candidates = names.join(", ")
        ))
    }

    /// Lookup a key related to a package.
    ///
    /// This is complicated, because it can be declared in the workplace declaration.
    pub(crate) fn lookup_dependency_key<'a, F, D, V, T>(
        &'a self,
        key: &'a str,
        dep: &'a Item,
        workspace_field: F,
        dep_lookup: D,
        value_lookup: V,
        field: &'static str,
    ) -> Result<Option<PackageValue<T>>>
    where
        F: Fn(ManifestWorkspace<'a>, &'a Self) -> Option<ManifestDependencies<'a>>,
        V: Fn(&'a Value) -> Option<T>,
        D: Fn(&ManifestDependency<'a>) -> Result<PackageValue<T>>,
    {
        if let Some(Item::Value(value)) = dep.get(field) {
            if let Some(value) = value_lookup(value) {
                return Ok(Some(PackageValue {
                    workspace: None,
                    value,
                }));
            }
        }

        // workspace dependency.
        if let Some(true) = dep.get("workspace").and_then(|w| w.as_bool()) {
            for &index in self.workspaces.iter() {
                let Some(workspace) = self.manifests.get(index).and_then(|p| p.manifest.as_workspace()) else {
                    continue;
                };

                let Some(deps) = workspace_field(workspace, self) else {
                    continue;
                };

                let Some(dep) = deps.get(key) else {
                    continue;
                };

                return Ok(Some(dep_lookup(&dep)?));
            }
        }

        Ok(None)
    }
}

/// A value fetched from a package that keeps track of where it is defined.
pub(crate) struct PackageValue<T> {
    /// The workspace package that the name came from. This will be useful once
    /// we start editing things.
    #[allow(unused)]
    workspace: Option<usize>,
    value: T,
}

impl<T> PackageValue<T> {
    /// Construct a value from a package.
    pub(crate) fn from_package(value: T) -> Self {
        Self {
            workspace: None,
            value,
        }
    }

    /// Convert into its inner value.
    pub(crate) fn into_value(value: PackageValue<T>) -> T {
        value.value
    }
}

impl<T> Deref for PackageValue<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}
