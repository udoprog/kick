use anyhow::{anyhow, Result};
use toml_edit::{Array, Item, Table};

use crate::cargo::Manifest;
use crate::cargo::RustVersion;
use crate::model::{PackageParams, RepoRef};

macro_rules! package_field {
    ($get:ident, $field:literal) => {
        pub(crate) fn $get(&self) -> Option<&str> {
            self.doc.get($field).and_then(Item::as_str)
        }
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

    package_field!(version, "version");
    package_field!(license, "license");
    package_field!(readme, "readme");
    package_field!(repository, "repository");
    package_field!(homepage, "homepage");
    package_field!(documentation, "documentation");

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
