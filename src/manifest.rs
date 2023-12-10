mod dependencies;
mod dependency;
mod package;
mod workspace;
mod workspace_dependencies;
mod workspace_dependency;

use std::borrow::Cow;
use std::collections::HashSet;
use std::path::Path;

use anyhow::{anyhow, Result};
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize};
use toml_edit::{Array, Document, Formatted, Item, Table, Value};

use crate::rust_version::RustVersion;
use crate::workspace::Crates;

pub(crate) use self::dependencies::Dependencies;
pub(crate) use self::dependency::Dependency;
pub(crate) use self::package::Package;
pub(crate) use self::workspace::Workspace;
pub(crate) use self::workspace_dependencies::WorkspaceDependencies;
pub(crate) use self::workspace_dependency::WorkspaceDependency;

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

/// Open a `Cargo.toml`.
pub(crate) fn open<P>(
    path: P,
    manifest_dir: &RelativePath,
    manifest_path: &RelativePath,
) -> Result<Option<Manifest>>
where
    P: AsRef<Path>,
{
    let input = match std::fs::read_to_string(path) {
        Ok(input) => input,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let doc = input.parse()?;

    Ok(Some(Manifest {
        doc,
        dir: manifest_dir.to_owned(),
        path: manifest_path.to_owned(),
    }))
}

macro_rules! manifest_package_field {
    ($insert:ident, $field:literal) => {
        pub(crate) fn $insert(&mut self, value: &str) -> Result<()> {
            let package = self.ensure_package_mut()?;
            package.insert(
                $field,
                Item::Value(Value::String(Formatted::new(String::from(value)))),
            );
            Ok(())
        }
    };
}

macro_rules! insert_package_list {
    ($insert:ident, $name:literal) => {
        pub(crate) fn $insert<I>(&mut self, iter: I) -> Result<()>
        where
            I: IntoIterator<Item = String>,
        {
            let package = self.ensure_package_mut()?;

            let mut array = Array::new();

            for keyword in iter {
                array.push(keyword);
            }

            package.insert($name, Item::Value(Value::Array(array)));
            Ok(())
        }
    };
}

/// A parsed `Cargo.toml`.
#[derive(Debug, Clone)]
pub(crate) struct Manifest {
    dir: RelativePathBuf,
    path: RelativePathBuf,
    doc: Document,
}

impl Manifest {
    /// Path of the manifest.
    pub(crate) fn path(&self) -> &RelativePath {
        &self.path
    }

    /// Directory of manifest.
    pub(crate) fn dir(&self) -> &RelativePath {
        &self.dir
    }

    /// Find the location of the entrypoint `lib.rs`.
    pub(crate) fn entries(&self) -> Vec<RelativePathBuf> {
        if let Some(path) = self
            .lib()
            .and_then(|lib| lib.get("path").and_then(toml_edit::Item::as_str))
        {
            vec![self.dir.join(path)]
        } else {
            vec![
                self.dir.join("src").join("lib.rs"),
                self.dir.join("src").join("main.rs"),
            ]
        }
    }

    /// Test if toml defines a package.
    pub(crate) fn is_package(&self) -> bool {
        self.doc.contains_key("package")
    }

    /// Get workspace configuration.
    pub(crate) fn as_workspace(&self) -> Option<Workspace<'_>> {
        let doc = self.doc.get("workspace")?.as_table()?;
        Some(Workspace::new(doc))
    }

    /// Get package configuration.
    pub(crate) fn as_package(&self) -> Option<Package<'_>> {
        let doc = self.doc.get("package")?.as_table()?;
        Some(Package::new(doc, self))
    }

    /// Insert authors.
    pub(crate) fn insert_authors(&mut self, authors: Vec<String>) -> Result<()> {
        let package = self.ensure_package_mut()?;
        let mut array = Array::new();

        for author in authors {
            array.push(author);
        }

        package.insert("authors", Item::Value(Value::Array(array)));
        Ok(())
    }

    /// Remove rust-version.
    pub(crate) fn remove_rust_version(&mut self) -> bool {
        if let Some(package) = self.doc.get_mut("package") {
            if let Some(table) = package.as_table_like_mut() {
                return table.remove("rust-version").is_some();
            }
        }

        false
    }

    /// Set rust-version to the desirable value.
    pub(crate) fn set_rust_version(&mut self, version: &RustVersion) -> Result<()> {
        let package = self.ensure_package_mut()?;
        package.insert(
            "rust-version",
            Item::Value(Value::String(Formatted::new(version.to_string()))),
        );
        Ok(())
    }

    /// Sort package keys.
    pub(crate) fn sort_package_keys(&mut self) -> Result<()> {
        use crate::cli::check::cargo::CargoKey;

        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum SortKey<'a> {
            CargoKey(CargoKey),
            Other(&'a toml_edit::Key),
        }

        let package = self.ensure_package_mut()?;

        package.sort_values_by(|a, _, b, _| {
            let a = crate::cli::check::cargo::cargo_key(a.to_string().trim())
                .map(SortKey::CargoKey)
                .unwrap_or(SortKey::Other(a));
            let b = crate::cli::check::cargo::cargo_key(b.to_string().trim())
                .map(SortKey::CargoKey)
                .unwrap_or(SortKey::Other(b));
            a.cmp(&b)
        });

        Ok(())
    }

    /// Save to the given path.
    pub(crate) fn save_to<P>(&self, path: P) -> Result<()>
    where
        P: AsRef<Path>,
    {
        let string = self.doc.to_string();
        std::fs::write(path, string.as_bytes())?;
        Ok(())
    }

    /// List of features.
    pub(crate) fn features(&self, workspace: &Crates) -> Result<HashSet<String>> {
        let mut new_features = HashSet::new();

        // Get explicit features.
        if let Some(table) = self.doc.get("features").and_then(|v| v.as_table()) {
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
    pub(crate) fn ensure_package(&self) -> Result<&Table> {
        self.doc
            .get("package")
            .and_then(|table| table.as_table())
            .ok_or_else(|| anyhow!("missing `[package]`"))
    }

    /// Access `[lib]` section.
    pub(crate) fn lib(&self) -> Option<&Table> {
        self.doc.get("lib").and_then(|table| table.as_table())
    }

    /// Access `[package]` section mutably.
    fn ensure_package_mut(&mut self) -> Result<&mut Table> {
        self.doc
            .get_mut("package")
            .and_then(|table| table.as_table_mut())
            .ok_or_else(|| anyhow!("missing `[package]`"))
    }

    manifest_package_field!(insert_version, "version");
    manifest_package_field!(insert_license, "license");
    manifest_package_field!(insert_readme, "readme");
    manifest_package_field!(insert_repository, "repository");
    manifest_package_field!(insert_homepage, "homepage");
    manifest_package_field!(insert_documentation, "documentation");

    /// Access dependencies.
    pub(crate) fn dependencies<'a>(&'a self, crates: &'a Crates) -> Option<Dependencies<'a>> {
        let doc = self
            .doc
            .get(DEPENDENCIES)
            .and_then(|table| table.as_table())?;

        Some(Dependencies::new(doc, crates, Workspace::dependencies))
    }

    /// Access dev-dependencies.
    pub(crate) fn dev_dependencies<'a>(&'a self, crates: &'a Crates) -> Option<Dependencies<'a>> {
        let doc = self
            .doc
            .get(DEV_DEPENDENCIES)
            .and_then(|table| table.as_table())?;

        Some(Dependencies::new(doc, crates, Workspace::dev_dependencies))
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
            Workspace::build_dependencies,
        ))
    }

    /// Get the given key.
    pub(crate) fn get_mut(&mut self, key: &str) -> Option<&mut Item> {
        self.doc.get_mut(key)
    }

    /// Remove the given key.
    pub(crate) fn remove(&mut self, key: &str) -> bool {
        self.doc.remove(key).is_some()
    }

    insert_package_list!(insert_keywords, "keywords");
    insert_package_list!(insert_categories, "categories");
}

impl Serialize for Manifest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        #[derive(Serialize, Deserialize)]
        struct DocumentRef<'a> {
            doc: &'a str,
            manifest_dir: &'a RelativePath,
            manifest_path: &'a RelativePath,
        }

        let doc = self.doc.to_string();

        let doc_ref = DocumentRef {
            doc: &doc,
            manifest_dir: &self.dir,
            manifest_path: &self.path,
        };

        doc_ref.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Manifest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Serialize, Deserialize)]
        struct DocumentRef<'a> {
            doc: Cow<'a, str>,
            manifest_dir: RelativePathBuf,
            manifest_path: RelativePathBuf,
        }

        let doc_ref = DocumentRef::deserialize(deserializer)?;

        let doc = doc_ref
            .doc
            .parse()
            .map_err(<D::Error as serde::de::Error>::custom)?;

        Ok(Self {
            doc,
            dir: doc_ref.manifest_dir,
            path: doc_ref.manifest_path,
        })
    }
}
