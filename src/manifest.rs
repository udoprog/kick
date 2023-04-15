use std::borrow::Cow;
use std::collections::HashSet;
use std::iter::from_fn;
use std::path::Path;

use anyhow::{anyhow, Result};
use relative_path::{RelativePath, RelativePathBuf};
use serde::{Deserialize, Serialize};
use toml_edit::{Array, Document, Formatted, Item, Table, Value};

use crate::model::{CrateParams, RepoRef};
use crate::rust_version::RustVersion;
use crate::workspace::{PackageValue, Workspace};

/// The "dependencies" field.
const DEPENDENCIES: &str = "dependencies";

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
        manifest_dir: manifest_dir.to_owned(),
        manifest_path: manifest_path.to_owned(),
    }))
}

macro_rules! field {
    ($get:ident, $insert:ident, $field:literal) => {
        pub(crate) fn $get(&self) -> Result<Option<&str>> {
            self.package_value($field, Item::as_str)
        }

        pub(crate) fn $insert(&mut self, $get: &str) -> Result<()> {
            let package = self.ensure_package_mut()?;
            package.insert(
                $field,
                Item::Value(Value::String(Formatted::new(String::from($get)))),
            );
            Ok(())
        }
    };
}

macro_rules! table_ref {
    ($get:ident, $field:literal) => {
        pub(crate) fn $get(&self) -> Option<&Table> {
            self.doc.get($field).and_then(|table| table.as_table())
        }
    };
}

macro_rules! table_mut {
    ($get_mut:ident, $remove:ident, $field:expr) => {
        pub(crate) fn $get_mut(&mut self) -> Option<&mut Table> {
            self.doc
                .get_mut($field)
                .and_then(|table| table.as_table_mut())
        }

        pub(crate) fn $remove(&mut self) -> bool {
            self.doc.remove($field).is_some()
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
    pub(crate) manifest_dir: RelativePathBuf,
    pub(crate) manifest_path: RelativePathBuf,
    doc: Document,
}

impl Manifest {
    /// Find the location of the entrypoint `lib.rs`.
    pub(crate) fn entries(&self) -> Vec<RelativePathBuf> {
        if let Some(path) = self
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
            name: self.crate_name()?,
            repo: repo.repo(),
            description: self.description()?,
            rust_version: match self.rust_version()? {
                Some(rust_version) => RustVersion::parse(rust_version),
                None => None,
            },
        })
    }

    /// Test if toml defines a package.
    pub(crate) fn is_package(&self) -> bool {
        self.doc.contains_key("package")
    }

    /// Test if package should or should not be published.
    pub(crate) fn is_publish(&self) -> Result<bool> {
        Ok(self
            .ensure_package()?
            .get("publish")
            .and_then(Item::as_bool)
            .unwrap_or(true))
    }

    /// Get workspace configuration.
    pub(crate) fn as_workspace(&self) -> Option<ManifestWorkspace<'_>> {
        let table = self.doc.get("workspace")?.as_table()?;
        Some(ManifestWorkspace { doc: table })
    }

    /// Get authors.
    pub(crate) fn authors(&self) -> Result<Option<&Array>> {
        self.package_value("authors", Item::as_array)
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

    /// Get categories.
    pub(crate) fn categories(&self) -> Result<Option<&Array>> {
        self.package_value("categories", Item::as_array)
    }

    /// Get keywords.
    pub(crate) fn keywords(&self) -> Result<Option<&Array>> {
        self.package_value("keywords", Item::as_array)
    }

    /// Get description.
    pub(crate) fn description(&self) -> Result<Option<&str>> {
        self.package_value("description", Item::as_str)
    }

    /// Rust version.
    pub(crate) fn rust_version(&self) -> Result<Option<&str>> {
        self.package_value("rust-version", Item::as_str)
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
    pub(crate) fn set_rust_version(&mut self, version: &str) -> Result<()> {
        let package = self.ensure_package_mut()?;
        package.insert(
            "rust-version",
            Item::Value(Value::String(Formatted::new(String::from(version)))),
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

    /// Get the name of the crate.
    pub(crate) fn crate_name(&self) -> Result<&str> {
        let package = self.ensure_package()?;

        let name = package
            .get("name")
            .and_then(|item| item.as_str())
            .ok_or_else(|| anyhow!("missing `[package] name`"))?;

        Ok(name)
    }

    /// List of features.
    pub(crate) fn features(&self, workspace: &Workspace) -> Result<HashSet<String>> {
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
                let package = dep.package_name()?;

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

    /// Access a package value.
    fn package_value<T, O: ?Sized>(&self, name: &str, map: T) -> Result<Option<&O>>
    where
        T: FnOnce(&Item) -> Option<&O>,
    {
        Ok(self.ensure_package()?.get(name).and_then(map))
    }

    field!(version, insert_version, "version");
    field!(license, insert_license, "license");
    field!(readme, insert_readme, "readme");
    field!(repository, insert_repository, "repository");
    field!(homepage, insert_homepage, "homepage");
    field!(documentation, insert_documentation, "documentation");

    pub(crate) fn dependencies<'a>(
        &'a self,
        workspace: &'a Workspace,
    ) -> Option<ManifestDependencies<'a>> {
        let doc = self
            .doc
            .get(DEPENDENCIES)
            .and_then(|table| table.as_table())?;

        Some(ManifestDependencies { doc, workspace })
    }

    table_mut!(dependencies_mut, remove_dependencies, DEPENDENCIES);
    table_ref!(dev_dependencies, "dev-dependencies");
    table_mut!(
        dev_dependencies_mut,
        remove_dev_dependencies,
        "dev-dependencies"
    );
    table_ref!(build_dependencies, "build-dependencies");
    table_mut!(
        build_dependencies_mut,
        remove_build_dependencies,
        "build-dependencies"
    );

    insert_package_list!(insert_keywords, "keywords");
    insert_package_list!(insert_categories, "categories");
}

/// Represents the `[workspace]` section of a manifest.
pub(crate) struct ManifestWorkspace<'a> {
    doc: &'a Table,
}

impl<'a> ManifestWorkspace<'a> {
    /// Get list of members.
    pub(crate) fn members(&self) -> impl Iterator<Item = &'a RelativePath> {
        let members = self.doc.get("members").and_then(|v| v.as_array());
        members
            .into_iter()
            .flatten()
            .flat_map(|v| Some(RelativePath::new(v.as_str()?)))
    }

    /// Get dependencies table, if it exists.
    pub(crate) fn dependencies(self, workspace: &'a Workspace) -> Option<ManifestDependencies<'a>> {
        let doc = self
            .doc
            .get(DEPENDENCIES)
            .and_then(|table| table.as_table())?;

        Some(ManifestDependencies { doc, workspace })
    }
}

/// A single declared dependency.
pub(crate) struct ManifestDependency<'a> {
    key: &'a str,
    value: &'a Item,
    workspace: &'a Workspace,
    accessor: fn(ManifestWorkspace<'a>, &'a Workspace) -> Option<ManifestDependencies<'a>>,
}

impl<'a> ManifestDependency<'a> {
    /// Get the package name of the dependency.
    pub(crate) fn package_name(&self) -> Result<PackageValue<&'a str>> {
        let optional = self.workspace.lookup_dependency_key(
            self.key,
            self.value,
            self.accessor,
            ManifestDependency::package_name,
            Value::as_str,
            "package",
        )?;

        Ok(PackageValue::from_package(
            optional.map(PackageValue::into_value).unwrap_or(self.key),
        ))
    }

    /// Get the package name of the dependency.
    pub(crate) fn is_optional(&self) -> Result<PackageValue<bool>> {
        let optional = self.workspace.lookup_dependency_key(
            self.key,
            self.value,
            self.accessor,
            ManifestDependency::is_optional,
            Value::as_bool,
            "optional",
        )?;

        Ok(PackageValue::from_package(
            optional.map(PackageValue::into_value).unwrap_or(false),
        ))
    }
}

/// Represents the `[dependencies]` section of a manifest.
pub(crate) struct ManifestDependencies<'a> {
    doc: &'a Table,
    workspace: &'a Workspace,
}

impl<'a> ManifestDependencies<'a> {
    /// Test if the dependencies section is empty.
    pub fn is_empty(&self) -> bool {
        self.doc.is_empty()
    }

    /// Get a dependency by its key.
    pub fn get(&self, key: &'a str) -> Option<ManifestDependency<'a>> {
        let value = self.doc.get(key)?;

        Some(ManifestDependency {
            key,
            value,
            workspace: self.workspace,
            accessor: ManifestWorkspace::dependencies,
        })
    }

    /// Iterate over dependencies.
    pub fn iter(&self) -> impl Iterator<Item = ManifestDependency<'a>> + 'a {
        let mut iter = self.doc.iter();
        let workspace = self.workspace;

        from_fn(move || {
            let (key, value) = iter.next()?;

            Some(ManifestDependency {
                key,
                value,
                workspace,
                accessor: ManifestWorkspace::dependencies,
            })
        })
    }
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
            manifest_dir: &self.manifest_dir,
            manifest_path: &self.manifest_path,
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
            manifest_dir: doc_ref.manifest_dir,
            manifest_path: doc_ref.manifest_path,
        })
    }
}
