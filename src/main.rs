//! [<img alt="github" src="https://img.shields.io/badge/github-udoprog/kick-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/kick)
//! [<img alt="crates.io" src="https://img.shields.io/crates/v/kick.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/kick)
//! [<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-kick-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/kick)
//!
//! Give your projects a good 🦶!
//!
//! ## Working with module sets
//!
//! Commands can produce sets under certain circumstances. Look out for switches
//! prefixes with `--save-*`.
//!
//! This stores and saves a set of modules depending on a certain condition,
//! such as `--save-success` for `kick for` which will save the module name for
//! every command that was successful. Or `--save-failed` for unsuccessful ones.
//!
//! The names of the sets will be printed at the end of the command, and can be
//! used with the `--with <set>` switch in subsequent iterations to only run
//! commands present in that set.

#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

macro_rules! error {
    ($error:ident, $($tt:tt)*) => {
        tracing::error!($($tt)*, error = $error);

        for $error in $error.chain().skip(1) {
            tracing::error!(concat!("caused by: ", $($tt)*), error = $error);
        }
    }
}

mod actions;
mod changes;
mod cli;
mod config;
mod ctxt;
mod file;
mod git;
mod gitmodules;
mod glob;
mod manifest;
mod model;
mod process;
mod rust_version;
mod sets;
mod templates;
mod urls;
mod utils;
mod workspace;

use std::cell::RefCell;
use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use changes::Change;
use clap::{Parser, Subcommand};

use actions::Actions;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use relative_path::{RelativePath, RelativePathBuf};
use tracing::metadata::LevelFilter;

use crate::{glob::Fragment, model::Module};

/// Name of project configuration files.
const KICK_TOML: &str = "Kick.toml";

#[derive(Subcommand)]
enum Action {
    /// Run checks non destructively for each module (default action).
    Check(cli::check::Opts),
    /// Run a command for each module.
    For(cli::foreach::Opts),
    /// Fetch github actions build status for each module.
    Status(cli::status::Opts),
    /// Find the minimum supported rust version for each module.
    Msrv(cli::msrv::Opts),
    /// Update package version.
    Version(cli::version::Opts),
    /// Publish packages in reverse order of dependencies.
    Publish(cli::publish::Opts),
    /// Upgrade packages.
    Upgrade(cli::upgrade::Opts),
    /// Apply the last saved committed set of changes.
    Apply,
}

impl Default for Action {
    fn default() -> Self {
        Self::Check(cli::check::Opts::default())
    }
}

#[derive(Default, Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Opts {
    /// Specify custom root folder for project hierarchy.
    #[arg(long, name = "path")]
    root: Option<PathBuf>,
    /// Force processing of all repos, even if the root is currently inside of
    /// an existing repo.
    #[arg(long)]
    all: bool,
    /// Only run the specified set of modules.
    #[arg(long = "module", short = 'm', name = "module")]
    modules: Vec<String>,
    /// Only run over dirty modules with changes that have not been staged in
    /// cache.
    #[arg(long)]
    dirty: bool,
    /// Only run over modules that have changes staged in cache.
    #[arg(long)]
    cached: bool,
    /// Only run over modules that only have changes staged in cached and
    /// nothing dirty.
    #[arg(long)]
    cached_only: bool,
    /// Only go over repos with unreleased changes, or ones which are on a
    /// commit that doesn't have a tag as determined by `git describe --tags`.
    #[arg(long)]
    unreleased: bool,
    /// Save any proposed changes.
    #[arg(long)]
    save: bool,
    /// Load sets with the given ids.
    #[arg(long)]
    set: Vec<String>,
    /// Subtract sets with the given ids. This will remove any items in the
    /// loaded set (as specified by `--set <id>`) that exist in the specified
    /// sets.
    #[arg(long)]
    sub_set: Vec<String>,
    /// Action to perform. Defaults to `check`.
    #[command(subcommand, name = "action")]
    action: Option<Action>,
}

impl Opts {
    fn needs_git(&self) -> bool {
        self.dirty || self.cached || self.cached_only || self.unreleased
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .try_init()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    entry().await
}

async fn entry() -> Result<()> {
    let opts = match Opts::try_parse() {
        Ok(opts) => opts,
        Err(error) => {
            match error.kind() {
                clap::error::ErrorKind::DisplayHelp => {
                    print!("{error}");
                }
                _ => {
                    return Err(error.into());
                }
            }

            return Ok(());
        }
    };

    let current_dir = match &opts.root {
        Some(root) => root.clone(),
        None => PathBuf::new(),
    };

    let (root, current_path) = if let Some((root, current_path)) = find_root(current_dir.clone())? {
        (root, current_path)
    } else {
        (current_dir, RelativePathBuf::new())
    };

    tracing::trace!(
        root = root.display().to_string(),
        ?current_path,
        "found project roots"
    );

    let github_auth = root.join(".github-auth");

    let github_auth = match std::fs::read_to_string(&github_auth) {
        Ok(auth) => Some(auth.trim().to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!("no .github-auth found, heavy rate limiting will apply");
            None
        }
        Err(e) => {
            return Err(anyhow::Error::from(e)).with_context(|| github_auth.display().to_string())
        }
    };

    let git = git::Git::find()?;

    let templating = templates::Templating::new()?;
    let modules = model::load_modules(&root, git.as_ref())?;

    tracing::trace!(
        modules = modules
            .iter()
            .map(|m| m.path().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        "loaded modules"
    );

    let config = config::load(&root, &templating, &modules)?;

    let mut sets = sets::Sets::new(root.join("sets"))?;

    let mut actions = Actions::default();
    actions.latest("actions/checkout", "v3");
    actions.check(
        "actions-rs/toolchain",
        &actions::ActionsRsToolchainActionsCheck,
    );
    actions.deny("actions-rs/cargo", "using `run` is less verbose and faster");
    actions.deny(
        "actions-rs/toolchain",
        "using `run` is less verbose and faster",
    );

    let current_path = if !opts.all && modules.iter().any(|m| m.path() == current_path) {
        Some(current_path.as_ref())
    } else {
        None
    };

    let mut filters = Vec::new();

    for module in &opts.modules {
        filters.push(Fragment::parse(module));
    }

    let set = match &opts.set[..] {
        [] => None,
        ids => {
            let mut set = HashSet::new();

            for id in ids {
                if let Some(s) = sets.load(id)? {
                    set.extend(s.iter().map(RelativePath::to_owned));
                }
            }

            for id in &opts.sub_set {
                if let Some(s) = sets.load(id)? {
                    for module in s.iter() {
                        set.remove(module);
                    }
                }
            }

            Some(set)
        }
    };

    filter_modules(
        &root,
        &opts,
        git.as_ref(),
        &modules,
        &filters,
        current_path,
        set.as_ref(),
    )?;

    let changes_path = root.join("changes.data.gz");

    let mut cx = ctxt::Ctxt {
        root: &root,
        config: &config,
        actions: &actions,
        modules: &modules,
        github_auth,
        rustc_version: ctxt::rustc_version(),
        git,
        warnings: RefCell::new(Vec::new()),
        changes: RefCell::new(Vec::new()),
        sets: &mut sets,
    };

    match opts.action.unwrap_or_default() {
        Action::Check(opts) => {
            cli::check::entry(&cx, &opts).await?;
        }
        Action::For(opts) => {
            cli::foreach::entry(&mut cx, &opts)?;
        }
        Action::Status(opts) => {
            cli::status::entry(&cx, &opts).await?;
        }
        Action::Msrv(opts) => {
            cli::msrv::entry(&cx, &opts)?;
        }
        Action::Version(opts) => {
            cli::version::entry(&cx, &opts)?;
        }
        Action::Publish(opts) => {
            cli::publish::entry(&cx, &opts)?;
        }
        Action::Upgrade(opts) => {
            cli::upgrade::entry(&cx, &opts)?;
        }
        Action::Apply => {
            let changes = load_changes(&changes_path)
                .with_context(|| anyhow!("{}", changes_path.display()))?;

            let Some(changes) = changes else {
                tracing::info!("No changes found: {}", changes_path.display());
                return Ok(());
            };

            if !opts.save {
                tracing::warn!("Not writing changes since `--save` was not specified");
            }

            for change in changes {
                crate::changes::apply(&cx, &change, opts.save)?;
            }

            if opts.save {
                tracing::warn!("Removing {}", changes_path.display());
                std::fs::remove_file(&changes_path)
                    .with_context(|| anyhow!("{}", changes_path.display()))?;
            }

            return Ok(());
        }
    }

    for warning in cx.warnings().iter() {
        crate::changes::report(warning)?;
    }

    for change in cx.changes().iter() {
        crate::changes::apply(&cx, change, opts.save)?;
    }

    if cx.can_save() && !opts.save {
        tracing::warn!("Not writing changes since `--save` was not specified");
        tracing::info!(
            "Writing commit to {}, use `kick apply` to apply it later",
            changes_path.display()
        );
        save_changes(&cx, &changes_path).with_context(|| anyhow!("{}", changes_path.display()))?;
    }

    sets.commit()?;
    Ok(())
}

/// Save changes to the given path.
fn load_changes(path: &Path) -> Result<Option<Vec<Change>>> {
    let f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };

    let encoder = GzDecoder::new(f);
    let out = serde_cbor::from_reader(encoder)?;
    Ok(out)
}

/// Save changes to the given path.
fn save_changes(cx: &ctxt::Ctxt<'_>, path: &Path) -> Result<()> {
    let f = File::create(path)?;
    let encoder = GzEncoder::new(f, Compression::default());
    let changes = cx.changes().iter().cloned().collect::<Vec<_>>();
    serde_cbor::to_writer(encoder, &changes)?;
    Ok(())
}

/// Perform more advanced filtering over modules.
fn filter_modules(
    root: &Path,
    opts: &Opts,
    git: Option<&git::Git>,
    modules: &[model::Module],
    filters: &[Fragment<'_>],
    current_path: Option<&RelativePath>,
    set: Option<&HashSet<RelativePathBuf>>,
) -> Result<(), anyhow::Error> {
    // Test if module should be skipped.
    let should_disable = |module: &Module| -> bool {
        if let Some(set) = set {
            if !set.contains(module.path()) {
                return true;
            }
        }

        if filters.is_empty() {
            if let Some(path) = current_path {
                return path != module.path();
            }

            return false;
        }

        !filters
            .iter()
            .any(|filter| filter.is_match(module.path().as_str()))
    };

    for module in modules {
        module.set_disabled(should_disable(module));

        if module.is_disabled() {
            continue;
        }

        if opts.needs_git() {
            let git = git.context("no working git command")?;
            let module_path = crate::utils::to_path(module.path(), root);

            let cached = git.is_cached(&module_path)?;
            let dirty = git.is_dirty(&module_path)?;

            let span =
                tracing::trace_span!("git", ?cached, ?dirty, module = module.path().to_string());
            let _enter = span.enter();

            if opts.dirty && !dirty {
                tracing::trace!("directory is not dirty");
                module.set_disabled(true);
            }

            if opts.cached && !cached {
                tracing::trace!("directory has no cached changes");
                module.set_disabled(true);
            }

            if opts.cached_only && (!cached || dirty) {
                tracing::trace!("directory has no cached changes");
                module.set_disabled(true);
            }

            if opts.unreleased {
                if let Some((tag, offset)) = git.describe_tags(&module_path)? {
                    if offset.is_none() {
                        tracing::trace!("no offset detected (tag: {tag})");
                        module.set_disabled(true);
                    }
                } else {
                    tracing::trace!("no tags to describe");
                    module.set_disabled(true);
                }
            }
        }
    }

    Ok(())
}

/// Find root path to use.
fn find_root(mut current_dir: PathBuf) -> Result<Option<(PathBuf, RelativePathBuf)>> {
    let mut current = current_dir.clone();
    let mut last = None;
    let mut current_path = RelativePathBuf::new();

    if !current_dir.is_absolute() {
        if current.components().next().is_none() {
            current_dir = std::env::current_dir()?;
        } else {
            current_dir = current_dir.canonicalize()?;
        }
    }

    while current.components().next().is_none() || current.is_dir() {
        if current.join(KICK_TOML).is_file() {
            last = Some((current.clone(), current_path.components().rev().collect()));
        }

        if let Some(c) = current_dir.file_name() {
            current_path.push(c.to_string_lossy().as_ref());
            current_dir.pop();
        }

        if current.file_name().is_some() {
            current.pop();
        } else {
            current.push(Component::ParentDir);
        }
    }

    let Some((relative, current)) = last else {
        tracing::trace!("no {KICK_TOML} found in hierarchy");
        return Ok(None);
    };

    Ok(Some((relative, current)))
}
