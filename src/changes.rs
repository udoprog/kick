use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io;
use std::ops::Range;
use std::path::Path;
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

/// Save changes to the given path.
pub(crate) fn load_changes(path: &Path) -> Result<Option<Vec<Change>>> {
    use std::io::Read;

    use flate2::read::GzDecoder;

    let f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let mut encoder = GzDecoder::new(f);

    let mut buf = Vec::new();
    encoder.read_to_end(&mut buf)?;
    Ok(serde_json::from_slice(&buf)?)
}

/// Save changes to the given path.
pub(crate) fn save_changes(cx: &Ctxt<'_>, path: &Path) -> Result<()> {
    use std::io::Write;

    use flate2::write::GzEncoder;
    use flate2::Compression;

    let f = fs::File::create(path)?;
    let changes = cx.changes().iter().cloned().collect::<Vec<_>>();
    let buf = serde_json::to_vec(&changes)?;
    let mut encoder = GzEncoder::new(f, Compression::default());
    encoder.write_all(&buf)?;
    encoder.flush()?;
    Ok(())
}

/// Report a warning.
pub(crate) fn report(cx: &Ctxt<'_>, warning: &Warning) -> Result<()> {
    match warning {
        Warning::MissingReadme { path } => {
            let path = cx.to_path(path);
            println!("{}: Missing README", path.display());
        }
        Warning::WrongWorkflowName {
            path,
            actual,
            expected,
        } => {
            let path = cx.to_path(path);
            println!(
                "{}: Wrong workflow name: {actual} (actual) != {expected} (expected)",
                path.display()
            );
        }
        Warning::ToplevelHeadings {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(file, range.start, *line_offset)?;
            let path = cx.to_path(path);
            println!(
                "{}:{line}:{column}: doc comment has toplevel headings",
                path.display()
            );
            println!("{string}");
        }
        Warning::MissingPreceedingBr {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(file, range.start, *line_offset)?;
            let path = cx.to_path(path);
            println!(
                "{}:{line}:{column}: missing preceeding <br>",
                path.display()
            );
            println!("{string}");
        }
        Warning::MissingFeature { path, feature } => {
            let path = cx.to_path(path);
            println!("{}: missing features `{feature}`", path.display());
        }
        Warning::NoFeatures { path } => {
            let path = cx.to_path(path);
            println!("{}: trying featured build (--all-features, --no-default-features), but no features present", path.display());
        }
        Warning::MissingEmptyFeatures { path } => {
            let path = cx.to_path(path);
            println!("{}: missing empty features build", path.display());
        }
        Warning::MissingAllFeatures { path } => {
            let path = cx.to_path(path);
            println!("{}: missing all features build", path.display());
        }
        Warning::ActionMissingKey {
            path,
            key,
            expected,
            doc,
            actual,
        } => {
            let path = cx.to_path(path);
            println!(
                "{}: {key}: action missing key, expected {expected}",
                path.display()
            );

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
            let path = cx.to_path(path);
            println!(
                "{}: {key}: action missing branch `{branch}`",
                path.display()
            );
        }
        Warning::ActionExpectedEmptyMapping { path, key } => {
            let path = cx.to_path(path);
            println!("{}: {key}: action expected empty mapping", path.display());
        }
    }

    Ok(())
}

/// Report and apply a asingle change.
pub(crate) fn apply(cx: &Ctxt<'_>, change: &Change, save: bool) -> Result<()> {
    match change {
        Change::MissingWorkflow { id, path, repo } => {
            let path = cx.to_path(path);
            println!("{}: Missing workflow", path.display());

            if save {
                if let Some(parent) = path.parent() {
                    if !parent.is_dir() {
                        std::fs::create_dir_all(parent)?;
                    }
                }

                let workspace = repo.require_workspace(cx)?;
                let primary_package = workspace.primary_package()?;
                let params = cx.repo_params(&primary_package, repo)?;

                let Some(string) = cx.config.workflow(repo, id, params)? else {
                    println!("  workflows.{id}: Missing workflow template!");
                    return Ok(());
                };

                std::fs::write(path, string)?;
            }
        }
        Change::BadWorkflow { path, doc, change } => {
            let path = cx.to_path(path);
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
                        println!("{}: {reason}", path.display());

                        if save {
                            doc.value_mut(*uses).set_string(string);

                            for (id, key) in remove_keys {
                                if let Some(mut m) = doc.value_mut(*id).into_mapping_mut() {
                                    if !m.remove(key) {
                                        bail!("{}: failed to remove key `{key}`", path.display());
                                    }
                                }
                            }

                            for (id, key, value) in set_keys {
                                let mut m = doc.value_mut(*id);

                                for step in key.split('.') {
                                    let Some(next) =
                                        m.into_mapping_mut().and_then(|m| m.get_into_mut(step))
                                    else {
                                        bail!(
                                            "{}: missing step `{step}` in key `{key}`",
                                            path.display()
                                        );
                                    };

                                    m = next;
                                }

                                m.set_string(value);
                            }

                            edited = true;
                        }
                    }
                    WorkflowChange::Error { name, reason } => {
                        println!("{}: {name}: {reason}", path.display());
                    }
                }
            }

            if edited {
                println!("{}: Fixing", path.display());
                std::fs::write(path, doc.to_string())?;
            }
        }
        Change::UpdateLib {
            path,
            lib: new_file,
        } => {
            let path = cx.to_path(path);

            if save {
                println!("{}: Fixing", path.display());
                std::fs::write(path, new_file.as_str())?;
            } else {
                println!("{}: Needs update", path.display());
            }
        }
        Change::UpdateReadme {
            path,
            readme: new_file,
        } => {
            let path = cx.to_path(path);

            if save {
                println!("{}: Fixing", path.display());
                std::fs::write(path, new_file.as_str())?;
            } else {
                println!("{}: Needs update", path.display());
            }
        }
        Change::CargoTomlIssues {
            path,
            cargo: modified_cargo,
            issues,
        } => {
            let path = cx.to_path(path);

            println!("{}:", path.display());

            for issue in issues {
                println!("  {issue}");
            }

            if let Some(modified_cargo) = modified_cargo {
                if save {
                    modified_cargo.save_to(path)?;
                } else {
                    println!("Would save {}", path.display());
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

                if package.rust_version() != Some(*version) {
                    if save {
                        tracing::info!(
                            "Saving {} with rust-version = \"{version}\"",
                            manifest.path()
                        );
                        manifest.set_rust_version(version)?;
                        manifest.sort_package_keys()?;
                        manifest.save_to(cx.to_path(manifest.path()))?;
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
                        manifest.save_to(cx.to_path(manifest.path()))?;
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
                let out = cx.to_path(manifest.path());
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
                let path = cx.to_path(path);
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
                    .current_dir(cx.to_path(manifest_dir));

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
        id: String,
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
