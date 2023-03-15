use std::collections::HashSet;
use std::path::Path;

use anyhow::{anyhow, Result};
use relative_path::RelativePath;
use toml_edit::{Array, Document, Formatted, Item, Table, Value};

/// A parsed `Cargo.toml`.
#[derive(Debug, Clone)]
pub(crate) struct Manifest {
    doc: Document,
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

macro_rules! dependencies {
    ($get:ident, $remove:ident, $field:literal) => {
        pub(crate) fn $get(&self) -> Option<&Table> {
            self.doc.get($field).and_then(|table| table.as_table())
        }

        pub(crate) fn $remove(&mut self) {
            self.doc.remove($field);
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

/// A cargo workspace.
pub(crate) struct Workspace<'a> {
    table: &'a Table,
}

impl<'a> Workspace<'a> {
    /// Get list of members.
    pub(crate) fn members(&self) -> impl Iterator<Item = &'a RelativePath> {
        let members = self.table.get("members").and_then(|v| v.as_array());
        members
            .into_iter()
            .flatten()
            .flat_map(|v| Some(RelativePath::new(v.as_str()?)))
    }
}

impl Manifest {
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
    pub(crate) fn as_workspace(&self) -> Option<Workspace<'_>> {
        let table = self.doc.get("workspace")?.as_table()?;
        Some(Workspace { table })
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
        return Ok(());
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
        use crate::validation::cargo::CargoKey;

        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum SortKey<'a> {
            CargoKey(CargoKey),
            Other(&'a toml_edit::Key),
        }

        let package = self.ensure_package_mut()?;

        package.sort_values_by(|a, _, b, _| {
            let a = crate::validation::cargo::cargo_key(a.to_string().trim())
                .map(SortKey::CargoKey)
                .unwrap_or(SortKey::Other(a));
            let b = crate::validation::cargo::cargo_key(b.to_string().trim())
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
    pub(crate) fn features(&self) -> HashSet<String> {
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
        if let Some(table) = self.dependencies() {
            for (key, value) in table.iter() {
                let package = if let Some(package) = value.get("package").and_then(|v| v.as_str()) {
                    package
                } else {
                    key
                };

                if value
                    .get("optional")
                    .and_then(|v| v.as_bool())
                    .filter(|v| *v)
                    .is_some()
                {
                    new_features.insert(package.to_owned());
                }
            }
        }

        new_features
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

    field!(license, insert_license, "license");
    field!(readme, insert_readme, "readme");
    field!(repository, insert_repository, "repository");
    field!(homepage, insert_homepage, "homepage");
    field!(documentation, insert_documentation, "documentation");
    dependencies!(dependencies, remove_dependencies, "dependencies");
    dependencies!(
        dev_dependencies,
        remove_dev_dependencies,
        "dev-dependencies"
    );
    dependencies!(
        build_dependencies,
        remove_build_dependencies,
        "build-dependencies"
    );
    insert_package_list!(insert_keywords, "keywords");
    insert_package_list!(insert_categories, "categories");
}

/// Open a `Cargo.toml`.
pub(crate) fn open<P>(path: P) -> Result<Manifest>
where
    P: AsRef<Path>,
{
    let input = std::fs::read_to_string(path)?;
    let doc = input.parse()?;
    Ok(Manifest { doc })
}
