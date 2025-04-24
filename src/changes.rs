use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::ops::Range;
use std::path::Path;
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{bail, Context, Result};
use musli::storage::Encoding;
use musli::{Decode, Encode};
use nondestructive::yaml;
use relative_path::RelativePathBuf;
use semver::Version;
use similar::{ChangeTag, TextDiff};
use termcolor::WriteColor;

use crate::cargo::Manifest;
use crate::cargo::RustVersion;
use crate::cli::check::cargo::CargoKey;
use crate::cli::check::ci::ActionExpected;
use crate::commands::Colors;
use crate::config::Replaced;
use crate::ctxt::Ctxt;
use crate::edits::{self, Edits};
use crate::file::{File, LineColumn};
use crate::model::RepoRef;
use crate::process::Command;

const ENCODING: Encoding = Encoding::new();

/// Save changes to the given path.
pub(crate) fn load_changes(path: &Path) -> Result<Option<Vec<ChangeWrapper>>> {
    use std::io::Read;

    use flate2::read::GzDecoder;

    let f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context(path.display().to_string()),
    };

    let mut encoder = GzDecoder::new(f);

    let mut buf = Vec::new();
    encoder.read_to_end(&mut buf)?;

    let cx = musli::context::new().with_trace().with_type();

    let value: Vec<ChangeWrapper> = match ENCODING.from_slice_with(&cx, &buf) {
        Ok(value) => value,
        Err(..) => {
            bail!("{}", cx.report())
        }
    };

    Ok(Some(value))
}

/// Save changes to the given path.
pub(crate) fn save_changes(changes: &[ChangeWrapper], path: &Path) -> Result<()> {
    use flate2::write::GzEncoder;
    use flate2::Compression;

    let f = fs::File::create(path)?;
    let mut w = GzEncoder::new(f, Compression::default());
    ENCODING.to_writer(&mut w, changes)?;
    let mut f = w.finish()?;
    f.flush()?;
    Ok(())
}

/// Report a warning.
pub(crate) fn report<W>(o: &mut W, cx: &Ctxt<'_>, warning: &Warning) -> Result<()>
where
    W: ?Sized + WriteColor,
{
    match warning {
        Warning::MissingReadme { path } => {
            let path = cx.to_path(path);
            writeln!(o, "{}: Missing README", path.display())?;
        }
        Warning::WrongWorkflowName {
            path,
            actual,
            expected,
        } => {
            let path = cx.to_path(path);
            writeln!(
                o,
                "{}: Wrong workflow name: {actual} (actual) != {expected} (expected)",
                path.display()
            )?;
        }
        Warning::ToplevelHeadings {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(file, range.start, *line_offset)?;
            let path = cx.to_path(path);
            writeln!(
                o,
                "{}:{line}:{column}: doc comment has toplevel headings",
                path.display()
            )?;
            writeln!(o, "{string}")?;
        }
        Warning::MissingPreceedingBr {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(file, range.start, *line_offset)?;
            let path = cx.to_path(path);
            writeln!(
                o,
                "{}:{line}:{column}: missing preceeding <br>",
                path.display()
            )?;
            writeln!(o, "{string}")?;
        }
        Warning::MissingFeature { path, feature } => {
            let path = cx.to_path(path);
            writeln!(o, "{}: missing features `{feature}`", path.display())?;
        }
        Warning::NoFeatures { path } => {
            let path = cx.to_path(path);
            writeln!(o,"{}: trying featured build (--all-features, --no-default-features), but no features present", path.display())?;
        }
        Warning::MissingEmptyFeatures { path } => {
            let path = cx.to_path(path);
            writeln!(o, "{}: missing empty features build", path.display())?;
        }
        Warning::MissingAllFeatures { path } => {
            let path = cx.to_path(path);
            writeln!(o, "{}: missing all features build", path.display())?;
        }
        Warning::ActionMissingKey {
            path,
            key,
            expected,
            doc,
            actual,
        } => {
            let path = cx.to_path(path);
            writeln!(
                o,
                "{}: {key}: action missing key, expected {expected}",
                path.display()
            )?;

            match actual {
                Some(value) => {
                    writeln!(o, "  actual:")?;
                    let value = doc.value(*value);
                    write!(o, "{value}")?;
                }
                None => {
                    writeln!(o, "  actual: *missing value*")?;
                }
            }
        }
        Warning::ActionExpectedEmptyMapping { path, key } => {
            let path = cx.to_path(path);
            writeln!(
                o,
                "{}: {key}: action expected empty mapping",
                path.display()
            )?;
        }
    }

    Ok(())
}

/// Report and apply a asingle change.
pub(crate) fn apply<W>(o: &mut W, cx: &Ctxt<'_>, change: &Change, save: bool) -> Result<()>
where
    W: ?Sized + WriteColor,
{
    let col = Colors::new();

    match change {
        Change::MissingWorkflow { id, path, repo } => {
            let path = cx.to_path(path);
            writeln!(o, "{}: Missing workflow", path.display())?;

            if save {
                if let Some(parent) = path.parent() {
                    if !parent.is_dir() {
                        fs::create_dir_all(parent)?;
                    }
                }

                let workspace = repo.require_workspace(cx)?;
                let primary_package = workspace.primary_package()?;
                let params = cx.repo_params(&primary_package, repo)?;

                let Some(string) = cx.config.workflow(repo, id, params)? else {
                    writeln!(o, "  workflows.{id}: Missing workflow template!")?;
                    return Ok(());
                };

                fs::write(path, string)?;
            }
        }
        Change::BadWorkflow {
            path,
            doc,
            edits,
            errors,
        } => {
            let path = cx.to_path(path);

            if !edits.is_empty() {
                let mut doc = doc.clone();
                let mut edited = false;

                for change in edits.changes() {
                    match change {
                        edits::Change::Insert {
                            at,
                            reason,
                            key,
                            value,
                        } => {
                            writeln!(o, "{}: {reason}", path.display())?;

                            if save {
                                let mut mapping = doc.value_mut(*at);

                                if let Some(mut mapping) = mapping.as_mapping_mut() {
                                    let at = mapping
                                        .insert(key.clone(), yaml::Separator::Auto)
                                        .as_ref()
                                        .id();
                                    value.replace(&mut doc, at);
                                }

                                edited = true;
                            }
                        }
                        edits::Change::Set { at, reason, value } => {
                            writeln!(o, "{}: {reason}", path.display())?;

                            if save {
                                value.replace(&mut doc, *at);
                                edited = true;
                            }
                        }
                        edits::Change::RemoveKey {
                            mapping,
                            reason,
                            key,
                        } => {
                            writeln!(o, "{}: {reason}", path.display())?;

                            if save {
                                if let Some(mut m) = doc.value_mut(*mapping).into_mapping_mut() {
                                    if !m.remove(key) {
                                        bail!("{}: failed to remove key `{key}`", path.display());
                                    }

                                    edited = true;
                                }
                            }
                        }
                    }
                }

                if edited {
                    writeln!(o, "{}: Fixing", path.display())?;
                    fs::write(&path, doc.to_string())?;
                }
            }

            for change in errors {
                match change {
                    WorkflowError::Error { name, reason } => {
                        writeln!(o, "{}: {name}: {reason}", path.display())?;
                    }
                }
            }
        }
        Change::UpdateLib {
            path,
            lib: new_file,
        } => {
            let path = cx.to_path(path);
            write_to(o, &col, &path, new_file, save)?;
        }
        Change::UpdateReadme {
            path,
            readme: new_file,
        } => {
            let path = cx.to_path(path);
            write_to(o, &col, &path, new_file, save)?;
        }
        Change::CargoTomlIssues {
            path,
            cargo: modified_cargo,
            issues,
        } => {
            let path = cx.to_path(path);

            writeln!(o, "{}:", path.display())?;

            for issue in issues {
                writeln!(o, "  {issue}")?;
            }

            if let Some(modified_cargo) = modified_cargo {
                if save {
                    modified_cargo.save_to(path)?;
                } else {
                    writeln!(o, "Would save {}", path.display())?;
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
                        manifest.set_rust_version(version);
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

                if no_verify.is_some() {
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
                let no_verify = match no_verify {
                    Some(NoVerify::Argument) => " with `--no-verify` due to argument",
                    Some(NoVerify::Circular) => " with `--no-verify` due to circular dependency",
                    None => "",
                };

                tracing::info!("{manifest_dir}: would publish: {name}{no_verify}");
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
        #[derive(Clone, Encode, Decode)]
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
    NoPublishVersion => write!(f, "package.version: non-empty while package.publish = false (Supported since Rust 1.75)"),
    KeysNotSorted { expected: Vec<CargoKey>, actual: Vec<CargoKey> } => {
        write!(f, "[package] keys out-of-order, expected: {expected:?}")
    }
}

/// A simple workflow change.
#[derive(Clone, Encode, Decode)]
pub(crate) enum WorkflowError {
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
    ActionExpectedEmptyMapping {
        path: RelativePathBuf,
        key: Box<str>,
    },
}

#[derive(Clone, Encode, Decode)]
pub(crate) enum NoVerify {
    Argument,
    Circular,
}

/// A single change.
#[derive(Clone, Encode, Decode)]
pub(crate) struct ChangeWrapper {
    /// The kind of change.
    pub(crate) change: Change,
    /// Whether the change has been applied or not.
    pub(crate) written: bool,
}

/// A change to be applied.
#[derive(Clone, Encode, Decode)]
pub(crate) enum Change {
    MissingWorkflow {
        id: String,
        #[musli(with = musli::serde)]
        path: RelativePathBuf,
        repo: RepoRef,
    },
    BadWorkflow {
        #[musli(with = musli::serde)]
        path: RelativePathBuf,
        #[musli(with = musli::serde)]
        doc: yaml::Document,
        edits: Edits,
        errors: Vec<WorkflowError>,
    },
    UpdateLib {
        #[musli(with = musli::serde)]
        path: RelativePathBuf,
        lib: Arc<File>,
    },
    UpdateReadme {
        #[musli(with = musli::serde)]
        path: RelativePathBuf,
        readme: Arc<File>,
    },
    CargoTomlIssues {
        #[musli(with = musli::serde)]
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
        #[musli(with = musli::serde)]
        path: RelativePathBuf,
        /// Version to commit.
        #[musli(with = musli::serde)]
        version: Version,
    },
    /// Perform a publish action somewhere.
    Publish {
        /// Name of the crate being published.
        name: String,
        /// Directory to publish.
        #[musli(with = musli::serde)]
        manifest_dir: RelativePathBuf,
        /// Whether we perform a dry run or not.
        dry_run: bool,
        /// Whether `--no-verify` should be passed and the cause fo passing it.
        no_verify: Option<NoVerify>,
        /// Extra arguments.
        args: Vec<OsString>,
    },
}

impl fmt::Display for Change {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Change::MissingWorkflow { id, path, .. } => {
                write!(f, "Missing workflow `{id}` in `{path}`")
            }
            Change::BadWorkflow { path, .. } => {
                write!(f, "Bad workflow `{path}`")
            }
            Change::UpdateLib { path, .. } => {
                write!(f, "Update lib `{path}`")
            }
            Change::UpdateReadme { path, .. } => {
                write!(f, "Update readme `{path}`")
            }
            Change::CargoTomlIssues { path, issues, .. } => {
                write!(f, "{path}: {}", issues.len())
            }
            Change::SetRustVersion { repo, version } => {
                write!(f, "{}: Set rust version to {version}", repo.path())
            }
            Change::RemoveRustVersion { repo, version } => {
                write!(f, "{}: Remove rust version {version}", repo.path())
            }
            Change::SavePackage { manifest } => {
                write!(f, "Save package `{}`", manifest.path())
            }
            Change::Replace { replaced } => {
                write!(f, "Replace `{}`", replaced.path().display())
            }
            Change::ReleaseCommit { path, version } => {
                write!(f, "{path}: Release commit `{version}`")
            }
            Change::Publish { name, .. } => {
                write!(f, "Publish `{name}`")
            }
        }
    }
}

/// Write a file.
pub(crate) fn write_to<W>(
    o: &mut W,
    col: &Colors,
    path: &Path,
    new_file: &File,
    save: bool,
) -> Result<()>
where
    W: ?Sized + WriteColor,
{
    let current = match fs::read_to_string(path) {
        Ok(current) => current,
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).context(path.display().to_string())?,
    };

    writeln!(o, "{}:", path.display())?;
    diff(o, current.as_str(), new_file.as_str(), col)?;

    if save {
        fs::write(path, new_file.as_str()).with_context(|| path.display().to_string())?;
    }

    Ok(())
}

fn diff<W>(o: &mut W, source: &str, val: &str, col: &Colors) -> Result<()>
where
    W: ?Sized + WriteColor,
{
    let diff = TextDiff::from_lines(source, val);

    for (idx, group) in diff.grouped_ops(3).iter().enumerate() {
        if idx > 0 {
            writeln!(o, "{:-^1$}", "-", 80)?;
        }

        for op in group {
            for change in diff.iter_inline_changes(op) {
                let (sign, color) = match change.tag() {
                    ChangeTag::Delete => ("-", &col.red),
                    ChangeTag::Insert => ("+", &col.green),
                    ChangeTag::Equal => (" ", &col.dim),
                };

                o.set_color(color)?;

                write!(o, "{}", Line(change.old_index()))?;
                write!(o, "{sign}")?;

                for (_, value) in change.iter_strings_lossy() {
                    write!(o, "{value}")?;
                }

                o.reset()?;

                if change.missing_newline() {
                    writeln!(o)?;
                }
            }
        }
    }

    Ok(())
}

struct Line(Option<usize>);

impl fmt::Display for Line {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self.0 {
            None => write!(f, "    "),
            Some(idx) => write!(f, "{:<4}", idx + 1),
        }
    }
}
