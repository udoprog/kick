use std::fmt;
use std::ops::Range;
use std::sync::Arc;

use anyhow::{bail, Result};
use nondestructive::yaml;
use relative_path::RelativePathBuf;
use semver::Version;

use crate::cli::check::cargo::CargoKey;
use crate::cli::check::ci::ActionExpected;
use crate::config::Replaced;
use crate::ctxt::Ctxt;
use crate::file::{File, LineColumn};
use crate::manifest::Manifest;
use crate::model::Module;
use crate::rust_version::RustVersion;
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

/// A simple workflow validation.
pub(crate) enum WorkflowChange {
    /// Oudated version of an action.
    ReplaceString {
        reason: String,
        string: String,
        value: yaml::Id,
        remove_keys: Vec<(yaml::Id, String)>,
        set_keys: Vec<(yaml::Id, String, String)>,
    },
    /// Deny use of the specific action.
    Error { name: String, reason: String },
}

/// A single validation.
pub(crate) enum Change {
    DeprecatedWorkflow {
        path: RelativePathBuf,
    },
    MissingWorkflow {
        path: RelativePathBuf,
        module: Module,
        candidates: Box<[RelativePathBuf]>,
    },
    WrongWorkflowName {
        path: RelativePathBuf,
        actual: String,
        expected: String,
    },
    BadWorkflow {
        path: RelativePathBuf,
        doc: yaml::Document,
        validation: Vec<WorkflowChange>,
    },
    MissingReadme {
        path: RelativePathBuf,
    },
    UpdateLib {
        path: RelativePathBuf,
        lib: Arc<File>,
    },
    UpdateReadme {
        path: RelativePathBuf,
        readme: Arc<File>,
    },
    ToplevelHeadings {
        path: RelativePathBuf,
        file: Arc<File>,
        range: Range<usize>,
        line_offset: usize,
    },
    MissingPreceedingBr {
        path: RelativePathBuf,
        file: Arc<File>,
        range: Range<usize>,
        line_offset: usize,
    },
    MissingFeature {
        path: RelativePathBuf,
        feature: String,
    },
    NoFeatures {
        path: RelativePathBuf,
    },
    MissingEmptyFeatures {
        path: RelativePathBuf,
    },
    MissingAllFeatures {
        path: RelativePathBuf,
    },
    CargoTomlIssues {
        path: RelativePathBuf,
        cargo: Option<Manifest>,
        issues: Vec<CargoIssue>,
    },
    ActionMissingKey {
        path: RelativePathBuf,
        key: Box<str>,
        expected: ActionExpected,
        doc: yaml::Document,
        actual: Option<yaml::Id>,
    },
    ActionOnMissingBranch {
        path: RelativePathBuf,
        key: Box<str>,
        branch: Box<str>,
    },
    ActionExpectedEmptyMapping {
        path: RelativePathBuf,
        key: Box<str>,
    },
    /// Set rust version for the given module.
    SetRustVersion {
        module: Module,
        version: RustVersion,
    },
    /// Remove rust version from the given module.
    RemoveRustVersion {
        module: Module,
        version: RustVersion,
    },
    /// Save a package.
    SavePackage {
        /// Save the given package.
        package: Package,
    },
    Replace {
        /// A cached replacement.
        replaced: Replaced,
    },
    ReleaseCommit {
        /// Perform a release commit.
        path: RelativePathBuf,
        /// Version to commit.
        version: Version,
    },
}
impl Change {
    /// Check if validation can save something.
    pub(crate) fn has_changes(&self) -> bool {
        match self {
            Change::DeprecatedWorkflow { .. } => false,
            Change::MissingWorkflow { .. } => true,
            Change::WrongWorkflowName { .. } => false,
            Change::BadWorkflow { validation, .. } => !validation.is_empty(),
            Change::MissingReadme { .. } => false,
            Change::UpdateLib { .. } => true,
            Change::UpdateReadme { .. } => true,
            Change::ToplevelHeadings { .. } => false,
            Change::MissingPreceedingBr { .. } => false,
            Change::MissingFeature { .. } => false,
            Change::NoFeatures { .. } => false,
            Change::MissingEmptyFeatures { .. } => false,
            Change::MissingAllFeatures { .. } => false,
            Change::CargoTomlIssues { cargo, .. } => cargo.is_some(),
            Change::ActionMissingKey { .. } => false,
            Change::ActionOnMissingBranch { .. } => false,
            Change::ActionExpectedEmptyMapping { .. } => false,
            Change::SetRustVersion { .. } => true,
            Change::RemoveRustVersion { .. } => true,
            Change::SavePackage { .. } => true,
            Change::Replace { .. } => true,
            Change::ReleaseCommit { .. } => true,
        }
    }
}

/// Report and apply a asingle validation.
pub(crate) fn apply(cx: &Ctxt<'_>, validation: &Change, save: bool) -> Result<()> {
    if cx.can_save() && !save {
        tracing::warn!("Not writing changes since `--save` is not specified");
    }

    match validation {
        Change::MissingWorkflow {
            path,
            module,
            candidates,
        } => {
            println!("{path}: Missing workflow");

            for candidate in candidates.iter() {
                println!("  Candidate: {candidate}");
            }

            if save {
                if let [from] = candidates.as_ref() {
                    println!("{path}: Rename from {from}",);
                    std::fs::rename(from.to_path(cx.root), path.to_path(cx.root))?;
                } else {
                    let path = path.to_path(cx.root);

                    if let Some(parent) = path.parent() {
                        if !parent.is_dir() {
                            std::fs::create_dir_all(parent)?;
                        }
                    }

                    let workspace = module.workspace(cx)?;
                    let primary_crate = workspace.primary_crate()?;
                    let params = cx.module_params(primary_crate, module)?;

                    let Some(string) = cx.config.workflow(module, params)? else {
                        println!("  Missing default workflow!");
                        return Ok(());
                    };

                    std::fs::write(path, string)?;
                }
            }
        }
        Change::DeprecatedWorkflow { path } => {
            println!("{path}: Reprecated Workflow");
        }
        Change::WrongWorkflowName {
            path,
            actual,
            expected,
        } => {
            println!("{path}: Wrong workflow name: {actual} (actual) != {expected} (expected)");
        }
        Change::BadWorkflow {
            path,
            doc,
            validation,
        } => {
            let mut doc = doc.clone();
            let mut edited = false;

            for validation in validation {
                match validation {
                    WorkflowChange::ReplaceString {
                        reason,
                        string,
                        value: uses,
                        remove_keys,
                        set_keys,
                    } => {
                        println!("{path}: {reason}");

                        if save {
                            doc.value_mut(*uses).set_string(string);

                            for (id, key) in remove_keys {
                                if let Some(mut m) = doc.value_mut(*id).into_mapping_mut() {
                                    if !m.remove(key) {
                                        bail!("{path}: failed to remove key `{key}`");
                                    }
                                }
                            }

                            for (id, key, value) in set_keys {
                                let mut m = doc.value_mut(*id);

                                for step in key.split('.') {
                                    let Some(next) = m.into_mapping_mut().and_then(|m| m.get_into_mut(step)) else {
                                        bail!("{path}: missing step `{step}` in key `{key}`");
                                    };

                                    m = next;
                                }

                                m.set_string(value);
                            }

                            edited = true;
                        }
                    }
                    WorkflowChange::Error { name, reason } => {
                        println!("{path}: {name}: {reason}");
                    }
                }
            }

            if edited {
                println!("{path}: Fixing");
                std::fs::write(path.to_path(cx.root), doc.to_string())?;
            }
        }
        Change::MissingReadme { path } => {
            println!("{path}: Missing README");
        }
        Change::UpdateLib {
            path,
            lib: new_file,
        } => {
            if save {
                println!("{path}: Fixing");
                std::fs::write(path.to_path(cx.root), new_file.as_str())?;
            } else {
                println!("{path}: Needs update");
            }
        }
        Change::UpdateReadme {
            path,
            readme: new_file,
        } => {
            if save {
                println!("{path}: Fixing");
                std::fs::write(path.to_path(cx.root), new_file.as_str())?;
            } else {
                println!("{path}: Needs update");
            }
        }
        Change::ToplevelHeadings {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(file, range.start, *line_offset)?;
            println!("{path}:{line}:{column}: doc comment has toplevel headings");
            println!("{string}");
        }
        Change::MissingPreceedingBr {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(file, range.start, *line_offset)?;
            println!("{path}:{line}:{column}: missing preceeding <br>");
            println!("{string}");
        }
        Change::MissingFeature { path, feature } => {
            println!("{path}: missing features `{feature}`");
        }
        Change::NoFeatures { path } => {
            println!("{path}: trying featured build (--all-features, --no-default-features), but no features present");
        }
        Change::MissingEmptyFeatures { path } => {
            println!("{path}: missing empty features build");
        }
        Change::MissingAllFeatures { path } => {
            println!("{path}: missing all features build");
        }
        Change::CargoTomlIssues {
            path,
            cargo: modified_cargo,
            issues,
        } => {
            println!("{path}:");

            for issue in issues {
                println!("  {issue}");
            }

            if let Some(modified_cargo) = modified_cargo {
                if save {
                    modified_cargo.save_to(path.to_path(cx.root))?;
                } else {
                    println!("Would save {path}");
                }
            }
        }
        Change::ActionMissingKey {
            path,
            key,
            expected,
            doc,
            actual,
        } => {
            println!("{path}: {key}: action missing key, expected {expected}");

            match actual {
                Some(value) => {
                    println!("  actual:");
                    let value = doc.value(*value);
                    print!("{value}");
                }
                None => {
                    println!("  actual: *missing value*");
                }
            }
        }
        Change::ActionOnMissingBranch { path, key, branch } => {
            println!("{path}: {key}: action missing branch `{branch}`");
        }
        Change::ActionExpectedEmptyMapping { path, key } => {
            println!("{path}: {key}: action expected empty mapping");
        }
        Change::SetRustVersion { module, version } => {
            if save {
                tracing::info!("Setting rust version: Rust {version}");
            } else {
                tracing::info!("Would set rust version: Rust {version}");
            }

            let workspace = module.workspace(cx)?;

            for p in workspace.packages() {
                if p.manifest.is_publish()? {
                    let mut p = p.clone();
                    let version = version.to_string();

                    if p.manifest.rust_version()? != Some(version.as_str()) {
                        if save {
                            tracing::info!(
                                "Saving {} with rust-version = \"{version}\"",
                                p.manifest_path
                            );
                            p.manifest.set_rust_version(&version)?;
                            p.manifest.sort_package_keys()?;
                            p.manifest.save_to(p.manifest_path.to_path(cx.root))?;
                        } else {
                            tracing::info!(
                                "Would save {} with rust-version = \"{version}\"",
                                p.manifest_path
                            );
                        }
                    }
                }
            }
        }
        Change::RemoveRustVersion { module, version } => {
            if save {
                tracing::info!("Clearing rust version: Rust {version}");
            } else {
                tracing::info!("Would clear rust version: Rust {version}");
            }

            let workspace = module.workspace(cx)?;

            for p in workspace.packages() {
                let mut p = p.clone();

                if p.manifest.remove_rust_version() {
                    if save {
                        tracing::info!(
                            "Saving {} without rust-version (target version outdates rust-version)",
                            p.manifest_path
                        );
                        p.manifest.save_to(p.manifest_path.to_path(cx.root))?;
                    } else {
                        tracing::info!(
                            "Woudl save {} without rust-version (target version outdates rust-version)",
                            p.manifest_path
                        );
                    }
                }
            }
        }
        Change::SavePackage { package } => {
            if save {
                tracing::info!("Saving {}", package.manifest_path);
                let out = package.manifest_path.to_path(cx.root);
                package.manifest.save_to(out)?;
            } else {
                tracing::info!("Would save {}", package.manifest_path);
            }
        }
        Change::Replace { replaced } => {
            if save {
                tracing::info!(
                    "Saving {} (replacement: {})",
                    replaced.path().display(),
                    replaced.replacement()
                );

                replaced.save()?;
            } else {
                tracing::info!(
                    "Would save {} (replacement: {})",
                    replaced.path().display(),
                    replaced.replacement()
                );
            }
        }
        Change::ReleaseCommit { path, version } => {
            if save {
                let git = cx.require_git()?;
                let version = version.to_string();
                let path = path.to_path(cx.root);
                tracing::info!("Making commit `Release {version}`");
                git.add(&path, ["-u"])?;
                git.commit(&path, format_args!("Release {version}"))?;
                tracing::info!("Tagging `{version}`");
                git.tag(&path, version)?;
            } else {
                tracing::info!("Would make commit `Release {version}`");
                tracing::info!("Would make tag `{version}`");
            }
        }
    };

    Ok(())
}

/// Temporary line comment fix which adjusts the line and column.
pub(crate) fn temporary_line_fix(
    file: &File,
    pos: usize,
    line_offset: usize,
) -> Result<(usize, usize, &str)> {
    let (LineColumn { line, column }, string) = file.line_column(pos)?;
    let line = line_offset + line;
    let column = column + 4;
    Ok((line, column, string))
}
