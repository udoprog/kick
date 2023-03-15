use core::fmt;

use anyhow::Result;

use crate::model::UpdateParams;
use crate::validation::Validation;
use crate::workspace::Package;

macro_rules! cargo_issues {
    ($f:ident, $($issue:ident $({ $($field:ident: $ty:ty),* $(,)? })? => $description:expr),* $(,)?) => {
        pub(crate) enum CargoIssue {
            $($issue $({$($field: $ty),*})?,)*
        }

        impl fmt::Display for CargoIssue {
            fn fmt(&self, $f: &mut fmt::Formatter<'_>) -> fmt::Result {
                match self {
                    $(#[allow(unused_variables)] CargoIssue::$issue $({ $($field),* })? => $description,)*
                }
            }
        }
    }
}

cargo_issues! {
    f,
    MissingPackageLicense => write!(f, "package.license: missing"),
    WrongPackageLicense => write!(f, "package.license: wrong"),
    MissingPackageReadme => write!(f, "package.readme: missing"),
    WrongPackageReadme => write!(f, "package.readme: wrong"),
    MissingPackageRepository => write!(f, "package.repository: missing"),
    WrongPackageRepository => write!(f, "package.repository: wrong"),
    MissingPackageHomepage => write!(f, "package.homepage: missing"),
    WrongPackageHomepage => write!(f, "package.homepage: wrong"),
    MissingPackageDocumentation => write!(f, "package.documentation: missing"),
    WrongPackageDocumentation => write!(f, "package.documentation: wrong"),
    PackageDescription => write!(f, "package.description: missing"),
    PackageCategories => write!(f, "package.categories: missing"),
    PackageCategoriesNotSorted => write!(f, "package.categories: not sorted"),
    PackageKeywords => write!(f, "package.keywords: missing"),
    PackageKeywordsNotSorted => write!(f, "package.keywords: not sorted"),
    PackageAuthorsEmpty => write!(f, "authors: empty"),
    PackageDependenciesEmpty => write!(f, "dependencies: empty"),
    PackageDevDependenciesEmpty => write!(f, "dev-dependencies: empty"),
    PackageBuildDependenciesEmpty => write!(f, "build-dependencies: empty"),
    KeysNotSorted { expected: Vec<CargoKey>, actual: Vec<CargoKey> } => {
        write!(f, "[package] keys out-of-order, expected: {expected:?}")
    }
}

macro_rules! cargo_keys {
    ($($ident:ident => $name:literal),* $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
    package: &Package,
    validation: &mut Vec<Validation>,
    update: &UpdateParams<'_>,
) -> Result<()> {
    let mut modified_manifest = package.manifest.clone();
    let mut issues = Vec::new();
    let mut changed = false;

    macro_rules! check {
        ($get:ident, $insert:ident, $missing:ident, $wrong:ident) => {
            match (package.manifest.$get()?, &update.$get) {
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

    if package.manifest.description()?.is_none() {
        issues.push(CargoIssue::PackageDescription);
    }

    if let Some(categories) = package
        .manifest
        .categories()?
        .filter(|value| !value.is_empty())
    {
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

    if let Some(keywords) = package
        .manifest
        .keywords()?
        .filter(|value| !value.is_empty())
    {
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
        .manifest
        .authors()?
        .filter(|authors| !authors.is_empty())
        .is_none()
    {
        issues.push(CargoIssue::PackageAuthorsEmpty);
        changed = true;
        modified_manifest.insert_authors(update.authors.to_vec())?;
    }

    if matches!(package.manifest.dependencies(), Some(d) if d.is_empty()) {
        issues.push(CargoIssue::PackageDependenciesEmpty);
        changed = true;
        modified_manifest.remove_dependencies();
    }

    if matches!(package.manifest.dev_dependencies(), Some(d) if d.is_empty()) {
        issues.push(CargoIssue::PackageDevDependenciesEmpty);
        changed = true;
        modified_manifest.remove_dev_dependencies();
    }

    if matches!(package.manifest.build_dependencies(), Some(d) if d.is_empty()) {
        issues.push(CargoIssue::PackageBuildDependenciesEmpty);
        changed = true;
        modified_manifest.remove_build_dependencies();
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
        validation.push(Validation::CargoTomlIssues {
            path: package.manifest_path.clone(),
            cargo: changed.then_some(modified_manifest),
            issues,
        });
    }

    Ok(())
}
