use std::collections::{HashSet, VecDeque};
use std::ops::Deref;

use anyhow::{anyhow, Context, Result};
use relative_path::{RelativePath, RelativePathBuf};
use toml_edit::{Item, Value};

use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::manifest::{
    self, Manifest, ManifestDependency, Package, Workspace, WorkspaceDependencies,
};
use crate::model::RepoRef;

/// The default name of a cargo manifest `Cargo.toml`.
pub(crate) const CARGO_TOML: &str = "Cargo.toml";

/// Load a workspace starting at the given path.
#[tracing::instrument(skip_all)]
pub(crate) fn open(cx: &Ctxt<'_>, repo: &RepoRef) -> Result<Option<Crates>> {
    tracing::trace!("Opening workspace");

    let manifest_path = match cx.config.cargo_toml(repo.path()) {
        Some(cargo_toml) => repo.path().join(cargo_toml),
        None => repo.path().join(CARGO_TOML),
    };

    let primary_package = cx
        .config
        .crate_for(repo.path())
        .or(repo.repo().map(|repo| repo.name));

    let manifest_dir = manifest_path
        .parent()
        .ok_or_else(|| anyhow!("missing parent directory"))?;

    let Some(manifest) = manifest::open(manifest_path.to_path(cx.root), manifest_dir, &manifest_path)? else {
        return Ok(None);
    };

    let mut queue = VecDeque::new();
    queue.push_back(manifest);

    let mut visited = HashSet::new();

    let mut manifests = Vec::new();
    let mut workspaces = Vec::new();
    let mut packages = Vec::new();

    while let Some(manifest) = queue.pop_front() {
        if !visited.insert(manifest.dir().to_owned()) {
            tracing::trace!(?manifest, "Already loaded");
            continue;
        }

        if manifest.is_package() {
            packages.push(manifests.len());
        }

        tracing::trace!(path = ?manifest.path(), "Processing manifest");

        if let Some(workspace) = manifest.as_workspace() {
            let members = expand_members(cx, &manifest, workspace.members())?;

            for manifest_dir in members {
                let manifest_path = manifest_dir.join(CARGO_TOML);

                let manifest = manifest::open(
                    manifest_path.to_path(cx.root),
                    &manifest_dir,
                    &manifest_path,
                )
                .with_context(|| anyhow!("{manifest_path}"))?
                .with_context(|| anyhow!("{manifest_path}: missing file"))?;

                queue.push_back(manifest);
            }

            workspaces.push(manifests.len());
        }

        manifests.push(manifest);
    }

    Ok(Some(Crates {
        primary_package: primary_package.map(Box::from),
        manifests,
        packages,
        workspaces,
    }))
}

fn expand_members<'a>(
    cx: &Ctxt<'_>,
    manifest: &Manifest,
    iter: impl Iterator<Item = &'a RelativePath>,
) -> Result<Vec<RelativePathBuf>> {
    let mut output = Vec::new();

    for path in iter {
        let manifest_dir = manifest.dir().join(path);
        let glob = Glob::new(cx.root, &manifest_dir);

        for path in glob.matcher() {
            output.push(path?);
        }
    }

    Ok(output)
}

#[derive(Debug)]
pub(crate) struct Crates {
    primary_package: Option<Box<str>>,
    /// List of loaded packages and their associated manifests.
    manifests: Vec<Manifest>,
    /// Index of manifests which have a [package] declaration in them.
    packages: Vec<usize>,
    /// Index of manifests which have a [workspace] declaration in them.
    workspaces: Vec<usize>,
}

impl Crates {
    /// Test if this is a single crate workspace.
    pub(crate) fn is_single_crate(&self) -> bool {
        self.packages.len() == 1
    }

    /// Get list of packages.
    pub(crate) fn packages(&self) -> impl Iterator<Item = Package<'_>> {
        self.packages
            .iter()
            .flat_map(|&index| self.manifests.get(index)?.as_package())
    }

    /// Find the primary crate in the workspace.
    pub(crate) fn primary_package(&self) -> Result<Package<'_>> {
        // Single package, easy to determine primary package.
        if let &[index] = &self.packages[..] {
            let package = self
                .manifests
                .get(index)
                .context("missing package")?
                .as_package()
                .context("not a package")?;

            return Ok(package);
        }

        // Find a package which matches the name of the project.
        if let Some(name) = &self.primary_package {
            for &index in &self.packages {
                let package = self
                    .manifests
                    .get(index)
                    .context("missing package")?
                    .as_package()
                    .context("not a package")?;

                if package.name()? == name.as_ref() {
                    return Ok(package);
                }
            }
        }

        let mut names = Vec::with_capacity(self.manifests.len());

        for manifest in &self.manifests {
            if let Some(package) = manifest.as_package() {
                names.push(package.name()?);
            }
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
        dependency: &'a str,
        dep: &'a Item,
        workspace_field: F,
        dep_lookup: D,
        value_map: V,
        field: &'static str,
    ) -> Result<Option<PackageValue<T>>>
    where
        F: Fn(&Workspace<'a>) -> Option<WorkspaceDependencies<'a>>,
        V: Fn(&'a Value) -> Option<T>,
        D: Fn(&ManifestDependency<'a>) -> Option<T>,
    {
        if let Some(Item::Value(value)) = dep.get(field) {
            if let Some(value) = value_map(value) {
                return Ok(Some(PackageValue {
                    workspace: None,
                    value,
                }));
            }
        }

        // workspace dependency.
        if let Some(true) = dep.get("workspace").and_then(|w| w.as_bool()) {
            for &index in self.workspaces.iter() {
                let Some(workspace) = self.manifests.get(index).and_then(Manifest::as_workspace) else {
                    continue;
                };

                let Some(dep) = workspace_field(&workspace).and_then(|d| d.get(dependency)) else {
                    continue;
                };

                let Some(value) = dep_lookup(&dep) else {
                    continue;
                };

                return Ok(Some(PackageValue {
                    workspace: Some(index),
                    value,
                }));
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
