use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use clap::Parser;
use relative_path::RelativePathBuf;

use crate::changes::{Change, UpgradedFile};
use crate::cli::WithRepos;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::overlay::{self, Overlay};
use crate::process::Command;
use crate::workspace::CARGO_TOML;

const CARGO_LOCK: &str = "Cargo.lock";

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Pass `--workspace` to `cargo upgrade`.
    #[arg(long)]
    workspace: bool,
    /// Pass `--exclude <package>` to `cargo upgrade`.
    #[arg(long)]
    exclude: Vec<String>,
    /// Extra arguments to pass to `cargo upgrade`.
    extra: Vec<String>,
}

pub(crate) fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    with_repos.run("upgrade", format_args!("upgrade: {opts:?}"), |cx, repo| {
        upgrade(cx, opts, repo)
    })?;

    Ok(())
}

#[tracing::instrument(skip_all)]
fn upgrade(cx: &Ctxt<'_>, opts: &Opts, repo: &Repo) -> Result<()> {
    let current_dir = cx.to_path(repo.path());
    let upgrade = cx.config.upgrade(repo);
    let crates = repo.workspace(cx)?;

    let mut files = Vec::<RelativePathBuf>::new();

    for manifest in crates.manifests() {
        for path in [
            manifest.dir().join(CARGO_TOML),
            manifest.dir().join(CARGO_LOCK),
        ] {
            if !files.contains(&path) {
                files.push(path);
            }
        }
    }

    let paths = files.iter().map(|p| cx.to_path(p)).collect::<Vec<_>>();

    let mut command = Command::new("cargo");
    command.arg("upgrade");

    for exclude in upgrade.exclude.iter().chain(&opts.exclude) {
        command.args(["--exclude", exclude]);
    }

    if opts.workspace {
        command.arg("--workspace");
    }

    for arg in &opts.extra {
        command.arg(arg);
    }

    command.current_dir(&current_dir);

    let upgraded = run_staged(&cx.overlay, &paths, || {
        if !command.status()?.success() {
            bail!("Command failed");
        }

        Ok(())
    })?;

    if upgraded.is_empty() {
        tracing::info!("{}: Nothing to upgrade", repo.path());
        return Ok(());
    }

    let files = upgraded
        .into_iter()
        .map(|(index, content)| UpgradedFile {
            path: files[index].clone(),
            content,
        })
        .collect();

    cx.change(Change::Upgrade {
        path: repo.path().to_owned(),
        files,
    })
}

/// Run something which writes to the given files directly, then restore the
/// files and return the index and new content of each file it changed, so
/// that the changes can be staged instead.
///
/// The files are restored even if `run` fails or panics. Fails without
/// running anything if any of the files has a pending write in the overlay,
/// or differs from what was read through it earlier.
fn run_staged(
    overlay: &Overlay,
    paths: &[PathBuf],
    run: impl FnOnce() -> Result<()>,
) -> Result<Vec<(usize, Vec<u8>)>> {
    let mut snapshot = Snapshot::default();

    for path in paths {
        if overlay.is_pending(path) {
            bail!(
                "{}: has unsaved changes, save them before upgrading",
                path.display()
            );
        }

        let seen = overlay
            .read(path)
            .with_context(|| path.display().to_string())?;
        let original = read(path)?;

        if seen != original {
            bail!("{}: changed while kick was running", path.display());
        }

        snapshot.files.push((path.clone(), original));
    }

    let result = run();

    let mut changed = Vec::new();
    let mut read_error = None;

    for (index, (path, original)) in snapshot.files.iter().enumerate() {
        match read(path) {
            Ok(current) if current == *original => {}
            Ok(Some(current)) => changed.push((index, current)),
            Ok(None) => {
                tracing::warn!("{}: Removed by the upgrade, keeping it", path.display());
            }
            Err(error) => {
                read_error.get_or_insert(error);
            }
        }
    }

    snapshot.restore()?;
    result?;

    if let Some(error) = read_error {
        return Err(error);
    }

    Ok(changed)
}

/// Files with their original content, put back when dropped.
#[derive(Default)]
struct Snapshot {
    files: Vec<(PathBuf, Option<Vec<u8>>)>,
}

impl Snapshot {
    fn restore(&mut self) -> Result<()> {
        let mut first = None;

        for (path, original) in self.files.drain(..) {
            if let Err(error) = restore(&path, original.as_deref()) {
                tracing::error!("{}: Failed to restore: {error:#}", path.display());
                first.get_or_insert(error);
            }
        }

        match first {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        _ = self.restore();
    }
}

fn restore(path: &Path, original: Option<&[u8]>) -> Result<()> {
    if read(path)?.as_deref() == original {
        return Ok(());
    }

    match original {
        Some(original) => overlay::write_atomic(path, original),
        None => match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        },
    }
    .with_context(|| path.display().to_string())
}

fn read(path: &Path) -> Result<Option<Vec<u8>>> {
    overlay::read_original(path).with_context(|| path.display().to_string())
}

#[cfg(test)]
mod tests;
