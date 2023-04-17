use core::fmt;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::changes::{CargoIssue, Change};
use crate::ctxt::Ctxt;
use crate::manifest::{self, Package};
use crate::model::UpdateParams;
use crate::workspace::Crates;

macro_rules! cargo_keys {
    ($($ident:ident => $name:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(tag = "kind")]
        pub(crate) enum CargoKey {
            $($ident,)*
        }

        impl fmt::Display for CargoKey {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $(CargoKey::$ident { .. } => write!(f, $name),)*
                }
            }
        }

        pub(crate) fn cargo_key(key: &str) -> Option<CargoKey> {
            match key {
                $($name => Some(CargoKey::$ident),)*
                _ => None,
            }
        }
    };
}

// Order from: https://doc.rust-lang.org/cargo/reference/manifest.html
cargo_keys! {
    Name => "name",
    Version => "version",
    Authors => "authors",
    Edition => "edition",
    RustVersion => "rust-version",
    Description => "description",
    Documentation => "documentation",
    Readme => "readme",
    Homepage => "homepage",
    Repository => "repository",
    License => "license",
    Keywords => "keywords",
    Categories => "categories",
    Resolver => "resolver",
}

/// Validate the main `Cargo.toml`.
pub(crate) fn work_cargo_toml(
    cx: &Ctxt<'_>,
    crates: &Crates,
    package: &Package,
    update: &UpdateParams<'_>,
) -> Result<()> {
    let mut modified_manifest = package.manifest().clone();
    let mut issues = Vec::new();
    let mut changed = false;

    macro_rules! check {
        ($get:ident, $insert:ident, $missing:ident, $wrong:ident) => {
            match (package.$get(), &update.$get) {
                (None, Some(update)) => {
                    modified_manifest.$insert(update.clone())?;
                    issues.push(CargoIssue::$missing);
                    changed = true;
                }
                (Some(value), Some(update)) if value != *update => {
                    modified_manifest.$insert(update.clone())?;
                    issues.push(CargoIssue::$wrong);
                    changed = true;
                }
                _ => {}
            }
        };
    }

    check! {
        license,
        insert_license,
        MissingPackageLicense,
        WrongPackageLicense
    };

    check! {
        readme,
        insert_readme,
        MissingPackageReadme,
        WrongPackageReadme
    };

    check! {
        repository,
        insert_repository,
        MissingPackageRepository,
        WrongPackageRepository
    };

    check! {
        homepage,
        insert_homepage,
        MissingPackageHomepage,
        WrongPackageHomepage
    };

    check! {
        documentation,
        insert_documentation,
        MissingPackageDocumentation,
        WrongPackageDocumentation
    };

    if package.description().filter(|d| !d.is_empty()).is_none() {
        issues.push(CargoIssue::PackageDescription);
    }

    if let Some(categories) = package.categories().filter(|value| !value.is_empty()) {
        let categories = categories
            .iter()
            .flat_map(|v| Some(v.as_str()?.to_owned()))
            .collect::<Vec<_>>();
        let mut sorted = categories.clone();
        sorted.sort();

        if categories != sorted {
            issues.push(CargoIssue::PackageCategoriesNotSorted);
            changed = true;
            modified_manifest.insert_categories(sorted)?;
        }
    } else {
        issues.push(CargoIssue::PackageCategories);
    }

    if let Some(keywords) = package.keywords().filter(|value| !value.is_empty()) {
        let keywords = keywords
            .iter()
            .flat_map(|v| Some(v.as_str()?.to_owned()))
            .collect::<Vec<_>>();
        let mut sorted = keywords.clone();
        sorted.sort();

        if keywords != sorted {
            issues.push(CargoIssue::PackageKeywordsNotSorted);
            changed = true;
            modified_manifest.insert_keywords(sorted)?;
        }
    } else {
        issues.push(CargoIssue::PackageKeywords);
    }

    if package
        .authors()
        .filter(|authors| !authors.is_empty())
        .is_none()
    {
        issues.push(CargoIssue::PackageAuthorsEmpty);
        changed = true;
        modified_manifest.insert_authors(update.authors.to_vec())?;
    }

    if matches!(modified_manifest.dependencies(crates), Some(d) if d.is_empty()) {
        issues.push(CargoIssue::PackageDependenciesEmpty);
        changed = true;
        modified_manifest.remove(manifest::DEPENDENCIES);
    }

    if matches!(modified_manifest.dev_dependencies(crates), Some(d) if d.is_empty()) {
        issues.push(CargoIssue::PackageDevDependenciesEmpty);
        changed = true;
        modified_manifest.remove(manifest::DEV_DEPENDENCIES);
    }

    if matches!(modified_manifest.build_dependencies(crates), Some(d) if d.is_empty()) {
        issues.push(CargoIssue::PackageBuildDependenciesEmpty);
        changed = true;
        modified_manifest.remove(manifest::BUILD_DEPENDENCIES);
    }

    {
        let package = modified_manifest.ensure_package()?;
        let mut keys = Vec::new();

        for (key, _) in package.iter() {
            if let Some(key) = cargo_key(key) {
                keys.push(key);
            }
        }

        let mut sorted_keys = keys.clone();
        sorted_keys.sort();

        if keys != sorted_keys {
            issues.push(CargoIssue::KeysNotSorted {
                actual: keys,
                expected: sorted_keys,
            });
            modified_manifest.sort_package_keys()?;
            changed = true;
        }
    }

    if !issues.is_empty() {
        cx.change(Change::CargoTomlIssues {
            path: package.manifest().path().to_owned(),
            cargo: changed.then_some(modified_manifest),
            issues,
        });
    }

    Ok(())
}
