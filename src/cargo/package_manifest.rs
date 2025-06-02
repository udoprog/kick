use anyhow::{anyhow, Result};
use toml_edit::{Array, Formatted, Item, Key, Table, Value};

use crate::cargo::RustVersion;
use crate::model::{PackageParams, RepoRef};

macro_rules! insert_field {
    ($insert:ident, $field:literal) => {
        pub(crate) fn $insert<S>(&mut self, value: S) -> Result<()>
        where
            S: AsRef<str>,
        {
            self.doc.insert(
                $field,
                Item::Value(Value::String(Formatted::new(value.as_ref().to_owned()))),
            );
            Ok(())
        }
    };
}

macro_rules! insert_list {
    ($insert:ident, $name:literal) => {
        pub(crate) fn $insert<I>(&mut self, iter: I) -> Result<()>
        where
            I: IntoIterator,
            I::Item: AsRef<str>,
        {
            let mut array = Array::new();

            for keyword in iter {
                array.push(keyword.as_ref().to_owned());
            }

            self.doc.insert($name, Item::Value(Value::Array(array)));
            Ok(())
        }
    };
}

macro_rules! package_field {
    ($($get:ident, $field:literal),* $(,)?) => {
        $(
            pub(crate) fn $get(&self) -> Option<&str> {
                self.doc.get($field).and_then(Item::as_str)
            }
        )*
    };
}

/// Represents the `[package]` section of a manifest.
#[repr(transparent)]
pub(crate) struct Package {
    doc: Table,
}

impl Package {
    #[inline]
    pub(crate) fn new(doc: &Table) -> &Self {
        // SAFETY: Package is repr(transparent) over Table,
        unsafe { &*(doc as *const _ as *const Self) }
    }

    #[inline]
    pub(crate) fn new_mut(doc: &mut Table) -> &mut Self {
        // SAFETY: Package is repr(transparent) over Table,
        unsafe { &mut *(doc as *mut _ as *mut Self) }
    }

    #[inline]
    pub(crate) fn as_table(&self) -> &Table {
        &self.doc
    }

    /// Test if package should or should not be published.
    pub(crate) fn is_publish(&self) -> bool {
        self.doc
            .get("publish")
            .and_then(Item::as_bool)
            .unwrap_or(true)
    }

    /// Get the name of the package.
    pub(crate) fn name(&self) -> Result<&str> {
        let name = self
            .doc
            .get("name")
            .and_then(|item| item.as_str())
            .ok_or_else(|| anyhow!("missing `[package] name`"))?;

        Ok(name)
    }

    /// Get authors.
    pub(crate) fn authors(&self) -> Option<&Array> {
        self.doc.get("authors").and_then(Item::as_array)
    }

    /// Get categories.
    pub(crate) fn categories(&self) -> Option<&Array> {
        self.doc.get("categories").and_then(Item::as_array)
    }

    /// Get keywords.
    pub(crate) fn keywords(&self) -> Option<&Array> {
        self.doc.get("keywords").and_then(Item::as_array)
    }

    /// Get description.
    pub(crate) fn description(&self) -> Option<&str> {
        self.doc.get("description").and_then(Item::as_str)
    }

    /// Rust version.
    pub(crate) fn rust_version(&self) -> Option<RustVersion> {
        RustVersion::parse(self.doc.get("rust-version").and_then(Item::as_str)?)
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

    package_field! {
        version, "version",
        license, "license",
        readme, "readme",
        repository, "repository",
        homepage, "homepage",
        documentation, "documentation",
    }

    insert_field!(insert_version, "version");
    insert_field!(insert_license, "license");
    insert_field!(insert_readme, "readme");
    insert_field!(insert_repository, "repository");
    insert_field!(insert_homepage, "homepage");
    insert_field!(insert_documentation, "documentation");

    insert_list!(insert_keywords, "keywords");
    insert_list!(insert_categories, "categories");

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

    /// Insert authors.
    pub(crate) fn insert_authors(&mut self, authors: Vec<String>) -> Result<()> {
        let mut array = Array::new();

        for author in authors {
            array.push(author);
        }

        self.doc.insert("authors", Item::Value(Value::Array(array)));
        Ok(())
    }

    /// Sort package keys.
    pub(crate) fn sort_package_keys(&mut self) -> Result<()> {
        use crate::cli::check::cargo::CargoKey;

        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
        enum SortKey<'a> {
            CargoKey(CargoKey),
            Other(&'a Key),
        }

        self.doc.sort_values_by(|a, _, b, _| {
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

    /// Remove version.
    pub(crate) fn remove_version(&mut self) -> bool {
        self.doc.remove("version").is_some()
    }

    /// Remove rust-version.
    pub(crate) fn remove_rust_version(&mut self) -> bool {
        self.doc.remove("rust-version").is_some()
    }

    /// Set rust-version of the manifest.
    pub(crate) fn set_rust_version(&mut self, version: &RustVersion) -> bool {
        let version = version.to_string();

        if self.doc.get("rust-version").and_then(Item::as_str) == Some(&version) {
            return false;
        }

        self.doc.insert(
            "rust-version",
            Item::Value(Value::String(Formatted::new(version))),
        );

        true
    }
}
