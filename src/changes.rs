use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use anyhow::{Context, Result, anyhow, bail};
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
use crate::cli::publish::{self, CircularDev, ManifestBackup};
use crate::commands::Colors;
use crate::config::Replaced;
use crate::ctxt::Ctxt;
use crate::edits::{self, Edits};
use crate::file::{File, LineColumn};
use crate::model::RepoRef;
use crate::overlay::{self, Expected, PendingWrite};
use crate::process::Command;

const ENCODING: Encoding = Encoding::new();

/// Prefix of staged changes which record the files they expect, which can
/// never start a file holding only a list of changes as written by older
/// versions of kick.
const MAGIC: &[u8] = b"\0kick\x02";

/// Changes staged in `changes.gz`.
#[derive(Default, Encode, Decode)]
pub(crate) struct Staged {
    /// The staged changes.
    pub(crate) changes: Vec<ChangeWrapper>,
    /// The content each file written by the changes is expected to have.
    pub(crate) expected: Vec<Expected>,
}

/// Load staged changes from the given path.
pub(crate) fn load_changes(path: &Path) -> Result<Option<Staged>> {
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

    if let Some(rest) = buf.strip_prefix(MAGIC) {
        return Ok(Some(ENCODING.from_slice_traced(rest)?));
    }

    let changes: Vec<ChangeWrapper> = ENCODING.from_slice_traced(&buf)?;

    tracing::warn!(
        "{}: staged by an older version of kick, files are not verified to be unchanged since then",
        path.display()
    );

    Ok(Some(Staged {
        changes,
        expected: Vec::new(),
    }))
}

/// Save staged changes to the given path.
pub(crate) fn save_changes(
    changes: &[ChangeWrapper],
    expected: &[Expected],
    path: &Path,
) -> Result<()> {
    use flate2::Compression;
    use flate2::write::GzEncoder;

    #[derive(Encode)]
    struct StagedRef<'a> {
        changes: &'a [ChangeWrapper],
        expected: &'a [Expected],
    }

    let f = fs::File::create(path)?;
    let mut w = GzEncoder::new(f, Compression::default());
    w.write_all(MAGIC)?;
    ENCODING.to_writer(&mut w, &StagedRef { changes, expected })?;
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
            writeln!(
                o,
                "{}: trying featured build (--all-features, --no-default-features), but no features present",
                path.display()
            )?;
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

/// Apply every action which has not been written yet using `apply`.
///
/// Changes which are not actions are skipped, since their files are saved by
/// [`apply_changes`] before any action runs.
///
/// Actions which apply successfully are marked as written when `save` is set.
/// A failing action is logged and the remaining actions are still applied, so
/// the ones which failed stay unwritten and are kept for `kick changes`.
///
/// A crate is not published if a crate in the same workspace which it
/// depends on failed to publish, since that could only fail too or pick up an
/// older version from the registry. It counts as failed.
///
/// Returns the number of actions which failed to apply.
pub(crate) fn apply_all(
    changes: &mut [ChangeWrapper],
    save: bool,
    mut apply: impl FnMut(&Change) -> Result<()>,
) -> usize {
    let mut failed = 0;
    let mut failed_crates = Vec::<&str>::new();

    for change in changes.iter_mut() {
        if change.written || !change.change.is_action() {
            continue;
        }

        if let Change::Publish {
            name, depends_on, ..
        } = &change.change
            && let Some(dep) = depends_on
                .iter()
                .find(|dep| failed_crates.contains(&dep.as_str()))
        {
            failed += 1;
            failed_crates.push(name);
            tracing::error!("Not publishing `{name}` since `{dep}` failed to publish");
            continue;
        }

        match apply(&change.change) {
            Ok(()) => {
                if save {
                    change.written = true;
                }
            }
            Err(error) => {
                failed += 1;

                if let Change::Publish { name, .. } = &change.change {
                    failed_crates.push(name);
                }

                log_error("Failed to apply change", &error);
            }
        }
    }

    failed
}

fn log_error(what: impl fmt::Display, error: &anyhow::Error) {
    tracing::error!("{what}: {error}");

    for cause in error.chain().skip(1) {
        tracing::error!("Caused by: {cause}");
    }
}

/// Report, save and apply every staged change which has not been written yet.
///
/// Every pending file write is shown as a diff. When `save` is set, the files
/// are saved with [`overlay::commit`] before any action, like a release commit
/// or publish, is applied. Unless `partial` is set, nothing is saved and no
/// action is applied if any file fails to be saved.
///
/// Returns the number of changes which failed to apply.
pub(crate) fn apply_changes<W>(o: &mut W, cx: &Ctxt<'_>, save: bool, partial: bool) -> Result<usize>
where
    W: ?Sized + WriteColor,
{
    let col = Colors::new();
    let mut changes = cx.changes_mut();
    let mut failed = 0;

    for change in changes.iter() {
        if !change.written && !change.change.is_action() {
            apply(o, cx, &change.change, save)?;
        }
    }

    let writes = cx.overlay.pending();

    for write in &writes {
        if !write.is_unchanged() {
            write_diff(o, &col, write)?;
        }
    }

    let mut files_saved = true;

    if save {
        let commit = overlay::commit(&writes, partial);

        for conflict in &commit.conflicts {
            tracing::error!("{conflict}");
        }

        for (path, error) in &commit.errors {
            log_error(format_args!("{}: Failed to save", path.display()), error);
        }

        for (path, error) in &commit.rollback_errors {
            log_error(
                format_args!("{}: Failed to restore after a failed save", path.display()),
                error,
            );
        }

        files_saved = commit.is_complete();

        if !files_saved && !partial {
            tracing::error!(
                "Not saving any changes since not every file could be saved, use `--partial` to save what can be saved"
            );
        }

        for path in &commit.written {
            if let Some(w) = writes.iter().find(|w| w.path == *path)
                && !w.is_unchanged()
            {
                writeln!(o, "{}: Saved", w.display.display())?;
            }

            cx.overlay.saved(path);
        }

        let unsaved = cx
            .overlay
            .pending()
            .into_iter()
            .flat_map(|w| w.changes)
            .collect::<std::collections::BTreeSet<_>>();

        for (index, change) in changes.iter_mut().enumerate() {
            if change.written || change.change.is_action() {
                continue;
            }

            if unsaved.contains(&index) {
                failed += 1;
            } else {
                change.written = true;
            }
        }
    }

    if !files_saved && !partial {
        let actions = changes
            .iter()
            .filter(|c| !c.written && c.change.is_action())
            .count();

        if actions > 0 {
            tracing::error!("Not applying {actions} action(s) since files failed to save");
            failed += actions;
        }
    } else {
        failed += apply_all(&mut changes, save, |change| apply(o, cx, change, save));
    }

    Ok(failed)
}

/// Compute the file writes the given change results in.
///
/// Files are read through the overlay of the context, so a change sees the
/// pending writes of changes staged before it.
pub(crate) fn stage(cx: &Ctxt<'_>, change: &Change) -> Result<Vec<(PathBuf, Vec<u8>)>> {
    let mut writes = Vec::new();

    match change {
        Change::MissingWorkflow { id, path, repo } => {
            let path = cx.to_path(path);
            let workspace = repo.require_workspace(cx)?;
            let primary_package = workspace.primary_package()?.ensure_package()?;
            let params = cx.repo_params(primary_package, repo)?;

            let Some(string) = cx.config.workflow(repo, id, params)? else {
                tracing::warn!(
                    "{}: workflows.{id}: Missing workflow template",
                    path.display()
                );
                return Ok(writes);
            };

            writes.push((path, string.into_bytes()));
        }
        Change::BadWorkflow {
            path, doc, edits, ..
        } => {
            if !edits.is_empty() {
                let path = cx.to_path(path);
                let doc = edited_workflow(&path, doc, edits)?;
                writes.push((path, doc.to_string().into_bytes()));
            }
        }
        Change::UpdateLib { path, lib: file } | Change::UpdateReadme { path, readme: file } => {
            writes.push((cx.to_path(path), file.as_str().as_bytes().to_vec()));
        }
        Change::CargoTomlIssues {
            path,
            cargo: Some(manifest),
            ..
        } => {
            writes.push((cx.to_path(path), manifest.to_toml_string().into_bytes()));
        }
        Change::CargoTomlIssues { cargo: None, .. } => {}
        Change::SetRustVersion { repo, version } => {
            let workspace = repo.require_workspace(cx)?;

            for manifest in workspace.packages() {
                let Some(package) = manifest.as_package() else {
                    continue;
                };

                if !package.is_publish() || package.rust_version() == Some(*version) {
                    continue;
                }

                let mut manifest = manifest.clone();
                let package = manifest.ensure_package_mut()?;
                package.sort_package_keys()?;
                package.set_rust_version(version);
                let path = cx.to_path(manifest.path());
                writes.push((path, manifest.to_toml_string().into_bytes()));
            }
        }
        Change::RemoveRustVersion { repo, .. } => {
            let workspace = repo.require_workspace(cx)?;

            for manifest in workspace.packages() {
                let mut manifest = manifest.clone();

                let Some(package) = manifest.as_package_mut() else {
                    continue;
                };

                if package.remove_rust_version() {
                    let path = cx.to_path(manifest.path());
                    writes.push((path, manifest.to_toml_string().into_bytes()));
                }
            }
        }
        Change::SavePackage { manifest } => {
            let path = cx.to_path(manifest.path());
            writes.push((path, manifest.to_toml_string().into_bytes()));
        }
        Change::Replace { replaced } => {
            writes.push((replaced.path().to_owned(), replaced.to_bytes()));
        }
        Change::Upgrade { files, .. } => {
            for file in files {
                writes.push((cx.to_path(&file.path), file.content.clone()));
            }
        }
        Change::ReleaseCommit { .. } | Change::Publish { .. } => {}
    }

    Ok(writes)
}

/// Apply the edits of a bad workflow to its document.
fn edited_workflow(path: &Path, doc: &yaml::Document, edits: &Edits) -> Result<yaml::Document> {
    let mut doc = doc.clone();

    for change in edits.changes() {
        match change {
            edits::Change::Insert { at, key, value, .. } => {
                let mut mapping = doc.value_mut(*at);

                if let Some(mut mapping) = mapping.as_mapping_mut() {
                    let at = mapping
                        .insert(key.clone(), yaml::Separator::Auto)
                        .as_ref()
                        .id();
                    value.replace(&mut doc, at);
                }
            }
            edits::Change::Set { at, value, .. } => {
                value.replace(&mut doc, *at);
            }
            edits::Change::RemoveKey { mapping, key, .. } => {
                if let Some(mut m) = doc.value_mut(*mapping).into_mapping_mut()
                    && !m.remove(key)
                {
                    bail!("{}: failed to remove key `{key}`", path.display());
                }
            }
        }
    }

    Ok(doc)
}

/// Report a single change, and apply it if it is an action.
///
/// The files changes write are saved by [`apply_changes`], so for those this
/// only reports what they are about.
pub(crate) fn apply<W>(o: &mut W, cx: &Ctxt<'_>, change: &Change, save: bool) -> Result<()>
where
    W: ?Sized + WriteColor,
{
    match change {
        Change::MissingWorkflow { path, .. } => {
            let path = cx.to_path(path);
            writeln!(o, "{}: Missing workflow", path.display())?;
        }
        Change::BadWorkflow {
            path,
            edits,
            errors,
            ..
        } => {
            let path = cx.to_path(path);

            for change in edits.changes() {
                let (edits::Change::Insert { reason, .. }
                | edits::Change::Set { reason, .. }
                | edits::Change::RemoveKey { reason, .. }) = change;

                writeln!(o, "{}: {reason}", path.display())?;
            }

            for change in errors {
                match change {
                    WorkflowError::Error { name, reason } => {
                        writeln!(o, "{}: {name}: {reason}", path.display())?;
                    }
                }
            }
        }
        Change::UpdateLib { .. } | Change::UpdateReadme { .. } => {}
        Change::CargoTomlIssues { path, issues, .. } => {
            let path = cx.to_path(path);

            writeln!(o, "{}:", path.display())?;

            for issue in issues {
                writeln!(o, "  {issue}")?;
            }
        }
        Change::SetRustVersion { repo, version } => {
            if save {
                tracing::info!(
                    path = %cx.display(repo.path()),
                    "Setting rust version: Rust {version}"
                );
            } else {
                tracing::info!(
                    path = %cx.display(repo.path()),
                    "Would set rust version: Rust {version}"
                );
            }
        }
        Change::RemoveRustVersion { repo, version } => {
            if save {
                tracing::info!(
                    path = %cx.display(repo.path()),
                    "Clearing rust version: Rust {version}"
                );
            } else {
                tracing::info!(
                    path = %cx.display(repo.path()),
                    "Would clear rust version: Rust {version}"
                );
            }
        }
        Change::SavePackage { .. } => {}
        Change::Upgrade { path, .. } => {
            if save {
                tracing::info!(path = %cx.display(path), "Saving upgraded dependencies");
            } else {
                tracing::info!(path = %cx.display(path), "Would save upgraded dependencies");
            }
        }
        Change::Replace { replaced } => {
            tracing::info!(
                "{}: version replacement: {}",
                replaced.path().display(),
                replaced.replacement()
            );
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
            allow_dirty,
            remove_dev,
            circular_dev,
            depends_on: _,
        } => {
            let dir = cx.display(manifest_dir);

            if save {
                if cx.term.load(std::sync::atomic::Ordering::SeqCst) {
                    bail!("{dir}: not publishing `{name}` since kick was interrupted");
                }

                tracing::info!("{dir}: publishing: {name}");

                let mut command = Command::new("cargo");
                command.args(["publish"]);

                if no_verify.is_some() {
                    command.arg("--no-verify");
                }

                if allow_dirty.is_some() {
                    command.arg("--allow-dirty");
                }

                if *dry_run {
                    command.arg("--dry-run");
                }

                let path = cx.to_path(manifest_dir);

                publish::check_leftover_backup(&path)?;

                let mut backup = None;

                if *remove_dev || !circular_dev.is_empty() {
                    let cargo_toml = manifest_dir.join("Cargo.toml");

                    let mut manifest = crate::cargo::open(cx, &cargo_toml)?
                        .with_context(|| anyhow!("Missing {}", cx.display(&cargo_toml)))?;

                    let removed = publish::strip_dev_dependencies(
                        cx.paths,
                        &mut manifest,
                        *remove_dev,
                        circular_dev,
                    );

                    for removed in &removed {
                        tracing::info!("{dir}: Removing {removed}");
                    }

                    if !removed.is_empty() {
                        backup = Some(ManifestBackup::replace(&path, &manifest)?);
                    }
                }

                command
                    .args(&args[..])
                    .stdin(Stdio::null())
                    .current_dir(&path);

                let result = publish::run_cargo_publish(&mut command, name);

                if let Some(mut backup) = backup {
                    backup
                        .restore()
                        .with_context(|| anyhow!("{dir}: failed to restore Cargo.toml"))?;
                }

                let outcome = result?;

                // A real publish stops with an error when the version is
                // already published, which we treat as success so that a
                // partially failed publish can be run again. A dry run only
                // warns about it and still packages the crate.
                if outcome.already_exists {
                    if !*dry_run {
                        tracing::warn!("{dir}: `{name}` is already published, skipping");
                        return Ok(());
                    }

                    tracing::warn!(
                        "{dir}: `{name}` is already published, a real publish would skip it"
                    );
                }

                if !outcome.status.success() {
                    bail!("{dir}: failed to publish: {}", outcome.status);
                }

                tracing::info!("{}", outcome.status);
            } else {
                tracing::info!("{dir}: would publish: {name}");

                if let Some(no_verify) = no_verify {
                    match no_verify {
                        NoVerify::Argument => {
                            tracing::info!("{dir}: With `--no-verify` due to argument.");
                        }
                        NoVerify::Circular => {
                            tracing::info!("{dir}: With `--no-verify` due to circular dependency.");
                        }
                    };
                }

                if let Some(allow_dirty) = allow_dirty {
                    match allow_dirty {
                        AllowDirty::DevDependency => {
                            tracing::info!("{dir}: With `--allow-dirty` due to `--remove-dev`.");
                        }
                        AllowDirty::CircularDevDependency => {
                            tracing::info!(
                                "{dir}: With `--allow-dirty` since circular dev-dependencies are removed."
                            );
                        }
                        AllowDirty::Argument => {
                            tracing::info!("{dir}: With `--allow-dirty` due to argument.");
                        }
                    }
                }

                if *remove_dev {
                    tracing::info!(
                        "{dir}: Would remove all dev-dependencies due to `--remove-dev`."
                    );
                }

                for dev in circular_dev {
                    tracing::info!(
                        "{dir}: Would remove {dev}, since `{}` is published later.",
                        dev.package
                    );
                }
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
    RedundantPackageHomepage => write!(f, "package.homepage: redundant with package.repository"),
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
    VersionGroupMismatch { actual: String, expected: String } => {
        write!(f, "package.version: {actual} does not match version group, expected {expected}")
    },
    VersionGroupInheritedMismatch { actual: String, expected: String } => {
        write!(f, "package.version: inherited workspace version {actual} does not match version group, expected {expected} (update `[workspace.package] version`)")
    },
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

#[derive(Clone, Encode, Decode)]
pub(crate) enum AllowDirty {
    DevDependency,
    Argument,
    CircularDevDependency,
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
        path: RelativePathBuf,
        repo: RepoRef,
    },
    BadWorkflow {
        path: RelativePathBuf,
        doc: yaml::Document,
        edits: Edits,
        errors: Vec<WorkflowError>,
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
        #[musli(with = musli::serde)]
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
        /// Whether --no-verify should be passed and the cause fo passing it.
        no_verify: Option<NoVerify>,
        /// Whether --allow-dirty should be passed.
        allow_dirty: Option<AllowDirty>,
        /// Whether dev dependencies should be removed.
        remove_dev: bool,
        /// Extra arguments.
        args: Vec<OsString>,
        /// Dev-dependencies to remove since they refer to crates which are
        /// published later.
        circular_dev: Vec<CircularDev>,
        /// Crates in the same workspace which this crate depends on. If
        /// publishing any of them fails, this crate is not published.
        depends_on: Vec<String>,
    },
    /// Files changed by `cargo upgrade` in a repo.
    Upgrade {
        /// The path of the repo.
        path: RelativePathBuf,
        /// The files changed by the upgrade.
        files: Vec<UpgradedFile>,
    },
}

/// A file with the content an upgrade gave it.
#[derive(Clone, Encode, Decode)]
pub(crate) struct UpgradedFile {
    pub(crate) path: RelativePathBuf,
    pub(crate) content: Vec<u8>,
}

impl Change {
    /// Test if the change is an action which runs after files are saved,
    /// rather than one which writes files.
    pub(crate) fn is_action(&self) -> bool {
        matches!(self, Change::ReleaseCommit { .. } | Change::Publish { .. })
    }
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
            Change::Upgrade { path, files } => {
                write!(f, "{path}: Upgrade dependencies in {} file(s)", files.len())
            }
        }
    }
}

/// Write the diff of a pending file write.
fn write_diff<W>(o: &mut W, col: &Colors, write: &PendingWrite) -> Result<()>
where
    W: ?Sized + WriteColor,
{
    let original = write.original.as_deref().unwrap_or_default();

    if write.original.is_none() {
        writeln!(o, "{}: New file", write.display.display())?;
    } else {
        writeln!(o, "{}:", write.display.display())?;
    }

    let original = String::from_utf8_lossy(original);
    let content = String::from_utf8_lossy(&write.content);
    diff(o, &original, &content, col)?;
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

#[cfg(test)]
mod staging_tests;

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io::Write;

    use anyhow::bail;
    use relative_path::RelativePathBuf;
    use semver::Version;

    use super::{
        AllowDirty, Change, ChangeWrapper, ENCODING, Expected, NoVerify, apply_all, load_changes,
        save_changes,
    };
    use crate::cargo::Manifest;

    fn change(path: &str, written: bool) -> ChangeWrapper {
        ChangeWrapper {
            change: Change::ReleaseCommit {
                path: RelativePathBuf::from(path),
                version: Version::new(1, 0, 0),
            },
            written,
        }
    }

    fn path(change: &Change) -> &str {
        match change {
            Change::ReleaseCommit { path, .. } => path.as_str(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn apply_all_counts_failures_and_continues() {
        let mut changes = [
            change("ok", false),
            change("readonly", false),
            change("already", true),
            change("after", false),
        ];

        let mut seen = Vec::new();

        let failed = apply_all(&mut changes, true, |change| {
            let path = path(change);
            seen.push(path.to_owned());

            if path == "readonly" {
                bail!("{path}: Permission denied");
            }

            Ok(())
        });

        assert_eq!(failed, 1);
        // Already written changes are skipped, and a failure does not stop the
        // remaining changes from being applied.
        assert_eq!(seen, ["ok", "readonly", "after"]);
        let written = changes.iter().map(|c| c.written).collect::<Vec<_>>();
        // The failed change stays unwritten so it is saved for later.
        assert_eq!(written, [true, false, true, true]);
    }

    fn publish(name: &str, depends_on: &[&str]) -> ChangeWrapper {
        ChangeWrapper {
            change: Change::Publish {
                name: name.to_owned(),
                manifest_dir: RelativePathBuf::from(name),
                dry_run: false,
                no_verify: None,
                allow_dirty: None,
                remove_dev: false,
                args: Vec::new(),
                circular_dev: Vec::new(),
                depends_on: depends_on.iter().map(|d| d.to_string()).collect(),
            },
            written: false,
        }
    }

    #[test]
    fn apply_all_skips_dependents_of_failed_publish() {
        let mut changes = [
            publish("core", &[]),
            publish("other", &[]),
            publish("lib", &["core"]),
            publish("web", &["lib", "other"]),
            publish("unrelated", &["other"]),
        ];

        let mut seen = Vec::new();

        let failed = apply_all(&mut changes, true, |change| {
            let Change::Publish { name, .. } = change else {
                unreachable!();
            };

            seen.push(name.clone());

            if name == "core" {
                bail!("{name}: failed to publish");
            }

            Ok(())
        });

        // `lib` depends on `core` and `web` transitively, so neither is
        // attempted.
        assert_eq!(seen, ["core", "other", "unrelated"]);
        assert_eq!(failed, 3);
        let written = changes.iter().map(|c| c.written).collect::<Vec<_>>();
        assert_eq!(written, [false, true, false, false, true]);
    }

    #[test]
    fn apply_all_skips_file_changes() {
        let mut changes = [
            ChangeWrapper {
                change: Change::SavePackage {
                    manifest: Manifest::parse_for_test("Cargo.toml", "[package]\n").unwrap(),
                },
                written: false,
            },
            change("action", false),
        ];

        let mut seen = 0;

        let failed = apply_all(&mut changes, true, |_| {
            seen += 1;
            Ok(())
        });

        assert_eq!((failed, seen), (0, 1));
        let written = changes.iter().map(|c| c.written).collect::<Vec<_>>();
        assert_eq!(written, [false, true]);
    }

    #[test]
    fn apply_all_without_save_writes_nothing() {
        let mut changes = [change("a", false), change("b", false)];
        let failed = apply_all(&mut changes, false, |_| Ok(()));
        assert_eq!(failed, 0);
        assert!(changes.iter().all(|c| !c.written));
    }

    /// `sample_changes` as encoded by musli 0.1.8. The representation of
    /// changes.gz must not change, or existing staged changes stop loading.
    const FIXTURE: &[u8] = &[
        3, 2, 0, 9, 2, 0, 8, 99, 114, 97, 116, 101, 115, 47, 97, 1, 19, 49, 46, 50, 46, 51, 45, 97,
        108, 112, 104, 97, 46, 49, 43, 98, 117, 105, 108, 100, 1, 1, 2, 0, 7, 1, 0, 2, 0, 19, 99,
        114, 97, 116, 101, 115, 47, 97, 47, 67, 97, 114, 103, 111, 46, 116, 111, 109, 108, 1, 46,
        91, 112, 97, 99, 107, 97, 103, 101, 93, 10, 110, 97, 109, 101, 32, 61, 32, 34, 97, 34, 32,
        35, 32, 110, 97, 109, 101, 10, 118, 101, 114, 115, 105, 111, 110, 32, 61, 32, 34, 49, 46,
        50, 46, 51, 34, 10, 1, 0, 2, 0, 10, 9, 0, 1, 97, 1, 8, 99, 114, 97, 116, 101, 115, 47, 97,
        2, 1, 3, 1, 1, 4, 1, 2, 5, 1, 6, 1, 0, 8, 45, 45, 108, 111, 99, 107, 101, 100, 7, 0, 8, 1,
        1, 98, 1, 0,
    ];

    fn sample_changes() -> Vec<ChangeWrapper> {
        vec![
            ChangeWrapper {
                change: Change::ReleaseCommit {
                    path: RelativePathBuf::from("crates/a"),
                    version: Version::parse("1.2.3-alpha.1+build").unwrap(),
                },
                written: true,
            },
            ChangeWrapper {
                change: Change::SavePackage {
                    manifest: Manifest::parse_for_test(
                        "crates/a/Cargo.toml",
                        "[package]\nname = \"a\" # name\nversion = \"1.2.3\"\n",
                    )
                    .unwrap(),
                },
                written: false,
            },
            ChangeWrapper {
                change: Change::Publish {
                    name: "a".to_owned(),
                    manifest_dir: RelativePathBuf::from("crates/a"),
                    dry_run: true,
                    no_verify: Some(NoVerify::Circular),
                    allow_dirty: Some(AllowDirty::CircularDevDependency),
                    remove_dev: true,
                    args: vec![OsString::from("--locked")],
                    circular_dev: Vec::new(),
                    depends_on: vec!["b".to_owned()],
                },
                written: false,
            },
        ]
    }

    fn write_gz(path: &std::path::Path, bytes: &[u8]) {
        use flate2::Compression;
        use flate2::write::GzEncoder;

        let mut w = GzEncoder::new(std::fs::File::create(path).unwrap(), Compression::default());
        w.write_all(bytes).unwrap();
        w.finish().unwrap();
    }

    fn bad_workflow_sample() -> Vec<ChangeWrapper> {
        let doc = nondestructive::yaml::from_slice(
            "name: CI\non:\n  push: {}\njobs:\n  test:\n    runs-on: ubuntu-latest # os\n    steps:\n      - uses: actions/checkout@v3\n        with: { x: 1.5 }\n      - run: ~\n",
        )
        .unwrap();
        let root = doc.as_ref().id();
        let mut edits = crate::edits::Edits::default();
        edits.set(root, "reason", "value");
        edits.insert(
            root,
            "ins",
            "key".to_owned(),
            crate::edits::Value::String("v".to_owned()),
        );
        edits.remove_key(root, "rm", "name".to_owned());

        vec![ChangeWrapper {
            change: Change::BadWorkflow {
                path: RelativePathBuf::from(".github/workflows/ci.yml"),
                doc,
                edits,
                errors: vec![super::WorkflowError::Error {
                    name: "n".to_owned(),
                    reason: "r".to_owned(),
                }],
            },
            written: false,
        }]
    }

    #[test]
    fn bad_workflow_round_trips() {
        let changes = bad_workflow_sample();
        let bytes = ENCODING.to_vec(&changes).unwrap();
        let decoded: Vec<ChangeWrapper> = ENCODING.from_slice(&bytes).unwrap();

        let (
            Change::BadWorkflow {
                doc: expected,
                edits: expected_edits,
                ..
            },
            Change::BadWorkflow { doc, edits, .. },
        ) = (&changes[0].change, &decoded[0].change)
        else {
            panic!("expected BadWorkflow");
        };

        assert_eq!(doc.to_string(), expected.to_string());
        assert_eq!(doc.as_ref().id(), expected.as_ref().id());
        assert_eq!(edits.changes().count(), expected_edits.changes().count());

        let edit = |doc: &nondestructive::yaml::Document| {
            let mut doc = doc.clone();
            let mut root = doc.as_mut().into_mapping_mut().unwrap();
            root.insert("added", nondestructive::yaml::Separator::Auto)
                .set_string("value");
            assert!(root.remove("name"));
            doc.to_string()
        };

        assert_eq!(edit(doc), edit(expected));
    }

    #[test]
    fn changes_encoding_is_stable() {
        assert_eq!(ENCODING.to_vec(&sample_changes()).unwrap(), FIXTURE);
    }

    #[test]
    fn load_changes_reads_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changes.gz");
        write_gz(&path, FIXTURE);

        let staged = load_changes(&path).unwrap().unwrap();
        assert!(staged.expected.is_empty());
        let changes = staged.changes;
        assert_eq!(ENCODING.to_vec(&changes).unwrap(), FIXTURE);

        let Change::SavePackage { manifest } = &changes[1].change else {
            panic!("expected SavePackage");
        };

        assert_eq!(
            manifest.to_toml_string(),
            "[package]\nname = \"a\" # name\nversion = \"1.2.3\"\n"
        );
    }

    #[test]
    fn save_changes_records_expected_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changes.gz");

        let expected = vec![
            Expected {
                path: dir.path().join("Cargo.toml"),
                original: Some(b"[package]\n".to_vec()),
            },
            Expected {
                path: dir.path().join("new.yml"),
                original: None,
            },
        ];

        save_changes(&sample_changes(), &expected, &path).unwrap();

        let staged = load_changes(&path).unwrap().unwrap();
        assert_eq!(staged.expected, expected);
        assert_eq!(ENCODING.to_vec(&staged.changes).unwrap(), FIXTURE);
    }

    #[test]
    fn load_changes_reports_field_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("changes.gz");
        write_gz(&path, &FIXTURE[..FIXTURE.len() - 4]);

        let error = load_changes(&path).err().unwrap().to_string();
        assert!(error.starts_with("[2].change"), "{error}");
        assert!(error.contains("(at byte"), "{error}");
    }
}
