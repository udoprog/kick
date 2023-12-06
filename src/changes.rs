use std::ffi::OsString;
use std::fmt;
use std::ops::Range;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{bail, Result};
use nondestructive::yaml;
use relative_path::RelativePathBuf;
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::cli::check::cargo::CargoKey;
use crate::cli::check::ci::ActionExpected;
use crate::config::Replaced;
use crate::ctxt::Ctxt;
use crate::file::{File, LineColumn};
use crate::manifest::Manifest;
use crate::model::RepoRef;
use crate::process::Command;
use crate::rust_version::RustVersion;

/// Report a warning.
pub(crate) fn report(warning: &Warning) -> Result<()> {
    match warning {
        Warning::MissingReadme { path } => {
            println!("{path}: Missing README");
        }
        Warning::DeprecatedWorkflow { path } => {
            println!("{path}: Reprecated Workflow");
        }
        Warning::WrongWorkflowName {
            path,
            actual,
            expected,
        } => {
            println!("{path}: Wrong workflow name: {actual} (actual) != {expected} (expected)");
        }
        Warning::ToplevelHeadings {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(file, range.start, *line_offset)?;
            println!("{path}:{line}:{column}: doc comment has toplevel headings");
            println!("{string}");
        }
        Warning::MissingPreceedingBr {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(file, range.start, *line_offset)?;
            println!("{path}:{line}:{column}: missing preceeding <br>");
            println!("{string}");
        }
        Warning::MissingFeature { path, feature } => {
            println!("{path}: missing features `{feature}`");
        }
        Warning::NoFeatures { path } => {
            println!("{path}: trying featured build (--all-features, --no-default-features), but no features present");
        }
        Warning::MissingEmptyFeatures { path } => {
            println!("{path}: missing empty features build");
        }
        Warning::MissingAllFeatures { path } => {
            println!("{path}: missing all features build");
        }
        Warning::ActionMissingKey {
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
        Warning::ActionOnMissingBranch { path, key, branch } => {
            println!("{path}: {key}: action missing branch `{branch}`");
        }
        Warning::ActionExpectedEmptyMapping { path, key } => {
            println!("{path}: {key}: action expected empty mapping");
        }
    }

    Ok(())
}

/// Report and apply a asingle change.
pub(crate) fn apply(cx: &Ctxt<'_>, change: &Change, save: bool) -> Result<()> {
    match change {
        Change::MissingWorkflow {
            path,
            repo,
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

                    let workspace = repo.require_workspace(cx)?;
                    let primary_package = workspace.primary_package()?;
                    let params = cx.repo_params(&primary_package, repo)?;

                    let Some(string) = cx.config.workflow(repo, params)? else {
                        println!("  Missing default workflow!");
                        return Ok(());
                    };

                    std::fs::write(path, string)?;
                }
            }
        }
        Change::MissingWeeklyBuild { path, repo } => {
            println!("{path}: Missing workflow");

            if save {
                let path = path.to_path(cx.root);

                if let Some(parent) = path.parent() {
                    if !parent.is_dir() {
                        std::fs::create_dir_all(parent)?;
                    }
                }

                let workspace = repo.require_workspace(cx)?;
                let primary_package = workspace.primary_package()?;
                let params = cx.repo_params(&primary_package, repo)?;

                let Some(string) = cx.config.weekly(repo, params)? else {
                    println!("  Missing default weekly build!");
                    return Ok(());
                };

                std::fs::write(path, string)?;
            }
        }
        Change::BadWorkflow { path, doc, change } => {
            let mut doc = doc.clone();
            let mut edited = false;

            for change in change {
                match change {
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
                                    let Some(next) =
                                        m.into_mapping_mut().and_then(|m| m.get_into_mut(step))
                                    else {
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
        Change::SetRustVersion { repo, version } => {
            if save {
                tracing::info!(
                    path = repo.path().as_str(),
                    "Setting rust version: Rust {version}"
                );
            } else {
                tracing::info!(
                    path = repo.path().as_str(),
                    "Would set rust version: Rust {version}"
                );
            }

            let workspace = repo.require_workspace(cx)?;

            for package in workspace.packages() {
                if !package.is_publish() {
                    continue;
                }

                let mut manifest = package.manifest().clone();
                let version = version.to_string();

                if package.rust_version() != Some(version.as_str()) {
                    if save {
                        tracing::info!(
                            "Saving {} with rust-version = \"{version}\"",
                            manifest.path()
                        );
                        manifest.set_rust_version(&version)?;
                        manifest.sort_package_keys()?;
                        manifest.save_to(manifest.path().to_path(cx.root))?;
                    } else {
                        tracing::info!(
                            "Would save {} with rust-version = \"{version}\"",
                            manifest.path()
                        );
                    }
                }
            }
        }
        Change::RemoveRustVersion { repo, version } => {
            if save {
                tracing::info!(
                    path = repo.path().as_str(),
                    "Clearing rust version: Rust {version}"
                );
            } else {
                tracing::info!(
                    path = repo.path().as_str(),
                    "Would clear rust version: Rust {version}"
                );
            }

            let workspace = repo.require_workspace(cx)?;

            for package in workspace.packages() {
                let mut manifest = package.manifest().clone();

                if manifest.remove_rust_version() {
                    if save {
                        tracing::info!(
                            "Saving {} without rust-version (target version outdates rust-version)",
                            manifest.path()
                        );
                        manifest.save_to(manifest.path().to_path(cx.root))?;
                    } else {
                        tracing::info!(
                            "Woudl save {} without rust-version (target version outdates rust-version)",
                            manifest.path()
                        );
                    }
                }
            }
        }
        Change::SavePackage { manifest } => {
            if save {
                tracing::info!("Saving {}", manifest.path());
                let out = manifest.path().to_path(cx.root);
                manifest.save_to(out)?;
            } else {
                tracing::info!("Would save {}", manifest.path());
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
        Change::Publish {
            name,
            dry_run,
            manifest_dir,
            args,
            no_verify,
        } => {
            if save {
                tracing::info!("{}: publishing: {}", manifest_dir, name);

                let mut command = Command::new("cargo");
                command.args(["publish"]);

                if *no_verify {
                    command.arg("--no-verify");
                }

                if *dry_run {
                    command.arg("--dry-run");
                }

                command
                    .args(&args[..])
                    .stdin(Stdio::null())
                    .current_dir(manifest_dir.to_path(cx.root));

                let status = command.status()?;

                if !status.success() {
                    bail!("{}: failed to publish: {status}", manifest_dir);
                }

                tracing::info!("{status}");
            } else {
                tracing::info!("{}: would publish: {} (with --run)", manifest_dir, name);
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

macro_rules! cargo_issues {
    ($f:ident, $($issue:ident $({ $($field:ident: $ty:ty),* $(,)? })? => $description:expr),* $(,)?) => {
        #[derive(Clone, Serialize, Deserialize)]
        #[serde(tag = "kind")]
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

/// A simple workflow change.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
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

pub(crate) enum Warning {
    MissingReadme {
        path: RelativePathBuf,
    },
    DeprecatedWorkflow {
        path: RelativePathBuf,
    },
    WrongWorkflowName {
        path: RelativePathBuf,
        actual: String,
        expected: String,
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
}

/// A single change.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) enum Change {
    MissingWorkflow {
        path: RelativePathBuf,
        repo: RepoRef,
        candidates: Box<[RelativePathBuf]>,
    },
    MissingWeeklyBuild {
        path: RelativePathBuf,
        repo: RepoRef,
    },
    BadWorkflow {
        path: RelativePathBuf,
        doc: yaml::Document,
        change: Vec<WorkflowChange>,
    },
    UpdateLib {
        path: RelativePathBuf,
        lib: Arc<File>,
    },
    UpdateReadme {
        path: RelativePathBuf,
        readme: Arc<File>,
    },
    CargoTomlIssues {
        path: RelativePathBuf,
        cargo: Option<Manifest>,
        issues: Vec<CargoIssue>,
    },
    /// Set rust version for the given repo.
    SetRustVersion { repo: RepoRef, version: RustVersion },
    /// Remove rust version from the given repo.
    RemoveRustVersion { repo: RepoRef, version: RustVersion },
    /// Save a package.
    SavePackage {
        /// Save the given package.
        manifest: Manifest,
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
    /// Perform a publish action somewhere.
    Publish {
        /// Name of the crate being published.
        name: String,
        /// Directory to publish.
        manifest_dir: RelativePathBuf,
        /// Whether we perform a dry run or not.
        dry_run: bool,
        /// Whether `--no-verify` should be passed.
        no_verify: bool,
        /// Extra arguments.
        args: Vec<OsString>,
    },
}
