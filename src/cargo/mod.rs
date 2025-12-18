pub(crate) use self::dependencies::Dependencies;
mod dependencies;

pub(crate) use self::dependency::Dependency;
mod dependency;

pub(crate) use self::package_manifest::Package;
mod package_manifest;

pub(crate) use self::workspace_table::WorkspaceTable;
mod workspace_table;

use self::dependencies_table::DependenciesTable;
mod dependencies_table;

pub(crate) use self::dependency_item::DependencyItem;
mod dependency_item;

pub(crate) use self::rust_version::RustVersion;
pub(crate) mod rust_version;

use std::collections::HashSet;
use std::path::Path;
use std::{fs, io};

use anyhow::{Context, Result, anyhow};
use musli::{Decode, Encode};
use relative_path::{RelativePath, RelativePathBuf};
use toml_edit::{DocumentMut, Item, Table, TableLike};

use crate::ctxt::{Ctxt, Paths};
use crate::model::Repo;
use crate::workspace::Crates;

/// The "workspace" field.
pub(crate) const WORKSPACE: &str = "workspace";
/// The "dependencies" field.
pub(crate) const DEPENDENCIES: &str = "dependencies";
/// The "dev-dependencies" field.
pub(crate) const DEV_DEPENDENCIES: &str = "dev-dependencies";
/// The "build-dependencies" field.
pub(crate) const BUILD_DEPENDENCIES: &str = "build-dependencies";
/// Various kinds of dependencies sections.
pub(crate) const DEPS: [&str; 3] = [DEPENDENCIES, DEV_DEPENDENCIES, BUILD_DEPENDENCIES];
/// The "target" field.
pub(crate) const TARGET: &str = "target";

/// Open a `Cargo.toml`.
pub(crate) fn open(paths: Paths<'_>, manifest_path: &RelativePath) -> Result<Option<Manifest>> {
    let Some(input) = paths.read_to_string(manifest_path)? else {
        return Ok(None);
    };

    Ok(Some(Manifest {
        doc: input.parse().with_context(|| anyhow!("{manifest_path}"))?,
        path: manifest_path.into(),
    }))
}

/// A binary defined in a cargo manifest.
pub(crate) enum ManifestBinary {
    /// An autobin directory which should be listed.
    AutoBin(RelativePathBuf),
    /// A single entry binary.
    Entry(String),
}

impl ManifestBinary {
    pub(crate) fn list(&self, cx: &Ctxt<'_>, repo: &Repo, names: &mut Vec<String>) -> Result<()> {
        match self {
            ManifestBinary::AutoBin(dir) => {
                let dir = cx.to_path(repo.path().join(dir));

                let entries = match fs::read_dir(&dir) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        return Ok(());
                    }
                    Err(e) => {
                        return Err(e).with_context(|| dir.display().to_string());
                    }
                    Ok(entries) => entries,
                };

                for entry in entries {
                    let entry = entry.with_context(|| anyhow!("listing: {}", dir.display()))?;

                    let path = entry.path();

                    if !path.is_file() {
                        continue;
                    }

                    if !matches!(path.extension().and_then(|s| s.to_str()), Some("rs")) {
                        continue;
                    }

                    let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                        continue;
                    };

                    names.push(name.to_owned());
                }
            }
            ManifestBinary::Entry(name) => {
                names.push(name.clone());
            }
        }

        Ok(())
    }
}

/// A parsed `Cargo.toml`.
#[derive(Debug, Clone, Encode, Decode)]
pub(crate) struct Manifest {
    #[musli(with = musli::serde)]
    path: Box<RelativePath>,
    #[musli(with = crate::musli::string)]
    doc: DocumentMut,
}

impl Manifest {
    /// Path of the manifest.
    pub(crate) fn path(&self) -> &RelativePath {
        self.path.as_ref()
    }

    /// Directory of manifest.
    pub(crate) fn dir(&self) -> &RelativePath {
        self.path.parent().unwrap_or(RelativePath::new("."))
    }

    /// List of binary candidates.
    pub(crate) fn binaries(&self, binaries: &mut Vec<ManifestBinary>) -> Result<()> {
        if let Some(package) = self.as_package() {
            if package.is_autobin() {
                binaries.push(ManifestBinary::AutoBin(self.dir().join("src").join("bin")));
            }

            let name = package.name()?;
            binaries.push(ManifestBinary::Entry(name.to_owned()));
        }

        Ok(())
    }

    /// Find the location of the entrypoint `lib.rs`.
    pub(crate) fn entries(&self) -> Vec<RelativePathBuf> {
        if let Some(path) = self
            .lib()
            .and_then(|lib| lib.get("path").and_then(Item::as_str))
        {
            vec![self.dir().join(path)]
        } else {
            vec![
                self.dir().join("src").join("lib.rs"),
                self.dir().join("src").join("main.rs"),
            ]
        }
    }

    /// Test if toml defines a package.
    pub(crate) fn is_package(&self) -> bool {
        self.doc.contains_key("package")
    }

    /// Get workspace configuration.
    pub(crate) fn as_workspace(&self) -> Option<&WorkspaceTable> {
        let doc = self.doc.get("workspace")?.as_table()?;
        Some(WorkspaceTable::new(doc))
    }

    /// Access `[package]` section.
    pub(crate) fn as_package(&self) -> Option<&Package> {
        let doc = self.doc.get("package")?.as_table()?;
        Some(Package::new(doc))
    }

    /// Access `[package]` section mutably.
    pub(crate) fn as_package_mut(&mut self) -> Option<&mut Package> {
        let doc = self.doc.get_mut("package")?.as_table_mut()?;
        Some(Package::new_mut(doc))
    }

    /// Save to the given path.
    pub(crate) fn save_to<P>(&self, path: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        let string = self.doc.to_string();
        fs::write(path, string.as_bytes())?;
        Ok(())
    }

    /// List of features.
    pub(crate) fn features(&self, workspace: &Crates) -> Result<HashSet<String>> {
        let mut new_features = HashSet::new();

        // Get explicit features.
        if let Some(table) = self.doc.get("features").and_then(Item::as_table) {
            new_features.extend(
                table
                    .iter()
                    .filter(|(key, _)| *key != "default")
                    .map(|(key, _)| String::from(key)),
            );
        }

        // Get features from optional dependencies.
        if let Some(dependencies) = self.dependencies(workspace) {
            for dep in dependencies.iter() {
                let package = dep.package()?;

                if *dep.is_optional()? {
                    new_features.insert((*package).to_owned());
                }
            }
        }

        Ok(new_features)
    }

    /// Access `[package]` section.
    pub(crate) fn ensure_package(&self) -> Result<&Package> {
        self.as_package().context("missing `[package]`")
    }

    /// Access `[lib]` section.
    pub(crate) fn lib(&self) -> Option<&Table> {
        self.doc.get("lib").and_then(|table| table.as_table())
    }

    /// Access `[package]` section mutably.
    pub(crate) fn ensure_package_mut(&mut self) -> Result<&mut Package> {
        self.as_package_mut().context("missing `[package]`")
    }

    /// Access dependencies.
    pub(crate) fn dependencies<'a>(&'a self, crates: &'a Crates) -> Option<Dependencies<'a>> {
        let doc = self
            .doc
            .get(DEPENDENCIES)
            .and_then(|table| table.as_table())?;

        Some(Dependencies::new(doc, crates, WorkspaceTable::dependencies))
    }

    /// Access dev-dependencies.
    pub(crate) fn dev_dependencies<'a>(&'a self, crates: &'a Crates) -> Option<Dependencies<'a>> {
        let doc = self
            .doc
            .get(DEV_DEPENDENCIES)
            .and_then(|table| table.as_table())?;

        Some(Dependencies::new(
            doc,
            crates,
            WorkspaceTable::dev_dependencies,
        ))
    }

    /// Access build-dependencies.
    pub(crate) fn build_dependencies<'a>(&'a self, crates: &'a Crates) -> Option<Dependencies<'a>> {
        let doc = self
            .doc
            .get(BUILD_DEPENDENCIES)
            .and_then(|table| table.as_table())?;

        Some(Dependencies::new(
            doc,
            crates,
            WorkspaceTable::build_dependencies,
        ))
    }

    /// Get the document as a [`TableLike`].
    pub(crate) fn as_table_like_mut(&mut self) -> &mut dyn TableLike {
        self.doc.as_table_mut()
    }

    /// Get the given key.
    pub(crate) fn get_mut(&mut self, key: &str) -> Option<&mut Item> {
        self.doc.get_mut(key)
    }

    pub(crate) fn remove(&mut self, key: &str) -> bool {
        self.doc.remove(key).is_some()
    }

    /// Remove everything related to the given key, including target keys.
    pub(crate) fn remove_all(&mut self, key: &str) -> bool {
        let mut removed = self.remove(key);

        self.for_each_target_mut(|table| {
            removed |= table.remove(key).is_some();
        });

        removed
    }

    fn for_each_target_mut(&mut self, mut f: impl FnMut(&mut dyn TableLike)) {
        let Some(target) = self.doc.get_mut("target") else {
            return;
        };

        let Some(table) = target.as_table_like_mut() else {
            return;
        };

        for (_, value) in table.iter_mut() {
            let Some(table) = value.as_table_like_mut() else {
                continue;
            };

            f(table);
        }
    }
}
