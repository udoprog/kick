use anyhow::{anyhow, Result};
use toml_edit::{Array, Formatted, Item, Table, Value};

use crate::cargo::Manifest;
use crate::cargo::RustVersion;
use crate::model::{PackageParams, RepoRef};

macro_rules! package_field {
    ($lt:lifetime, $($get:ident, $field:literal),* $(,)?) => {
        $(
            pub(crate) fn $get(&self) -> Option<&$lt str> {
                self.doc.get($field).and_then(Item::as_str)
            }
        )*
    };
}

/// Represents the `[package]` section of a manifest.
pub(crate) struct Package<'a> {
    doc: &'a Table,
    manifest: &'a Manifest,
}

impl<'a> Package<'a> {
    pub(crate) fn new(doc: &'a Table, manifest: &'a Manifest) -> Self {
        Self { doc, manifest }
    }

    /// Get the underlying manifest of the package.
    pub(crate) fn manifest(&self) -> &Manifest {
        self.manifest
    }

    /// Test if package should or should not be published.
    pub(crate) fn is_publish(&self) -> bool {
        self.doc
            .get("publish")
            .and_then(Item::as_bool)
            .unwrap_or(true)
    }

    /// Get the name of the package.
    pub(crate) fn name(&self) -> Result<&'a str> {
        let name = self
            .doc
            .get("name")
            .and_then(|item| item.as_str())
            .ok_or_else(|| anyhow!("missing `[package] name`"))?;

        Ok(name)
    }

    /// Get authors.
    pub(crate) fn authors(&self) -> Option<&'a Array> {
        self.doc.get("authors").and_then(Item::as_array)
    }

    /// Get categories.
    pub(crate) fn categories(&self) -> Option<&'a Array> {
        self.doc.get("categories").and_then(Item::as_array)
    }

    /// Get keywords.
    pub(crate) fn keywords(&self) -> Option<&'a Array> {
        self.doc.get("keywords").and_then(Item::as_array)
    }

    /// Get description.
    pub(crate) fn description(&self) -> Option<&'a str> {
        self.doc.get("description").and_then(Item::as_str)
    }

    /// Rust version.
    pub(crate) fn rust_version(&self) -> Option<RustVersion> {
        RustVersion::parse(self.doc.get("rust-version").and_then(Item::as_str)?)
    }

    package_field! {
        'a,
        version, "version",
        license, "license",
        readme, "readme",
        repository, "repository",
        homepage, "homepage",
        documentation, "documentation",
    }

    /// Construct crate parameters.
    pub(crate) fn package_params<'p>(&'p self, repo: &'p RepoRef) -> Result<PackageParams<'p>> {
        Ok(PackageParams {
            name: self.name()?,
            repo: repo.repo(),
            description: self.description(),
            rust_version: self.rust_version(),
        })
    }
}

/// Represents the `[package]` section of a manifest.
pub(crate) struct PackageMut<'a> {
    pub(crate) doc: &'a mut Table,
}

impl<'a> PackageMut<'a> {
    pub(crate) fn new(doc: &'a mut Table) -> Self {
        Self { doc }
    }

    /// Test if manifest is publish.
    pub(crate) fn is_publish(&self) -> bool {
        self.doc
            .get("publish")
            .and_then(Item::as_bool)
            .unwrap_or(true)
    }

    /// Set version of the manifest.
    ///
    /// Returns `true` if the version string was modified.
    pub(crate) fn set_version(&mut self, version: &str) -> bool {
        if self.doc.get("version").and_then(|item| item.as_str()) == Some(version) {
            return false;
        }

        self.doc.insert(
            "version",
            Item::Value(Value::String(Formatted::new(version.to_owned()))),
        );
        true
    }
}
