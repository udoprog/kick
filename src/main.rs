//! [<img alt="github" src="https://img.shields.io/badge/github-udoprog/kick-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/kick)
//! [<img alt="crates.io" src="https://img.shields.io/crates/v/kick.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/kick)
//! [<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-kick-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/kick)
//!
//! Give your projects a good 🦶!

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
mod cli;
mod config;
mod ctxt;
mod file;
mod git;
mod gitmodules;
mod glob;
mod manifest;
mod model;
mod rust_version;
mod templates;
mod urls;
mod utils;
mod validation;
mod workspace;

use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use actions::Actions;
use relative_path::{RelativePath, RelativePathBuf};
use tracing::metadata::LevelFilter;

use crate::{glob::Fragment, model::Module};

/// Name of project configuration files.
const KICK_TOML: &str = "Kick.toml";

#[derive(Subcommand)]
enum Action {
    /// Run checks non destructively for each module (default action).
    Check(cli::check::Opts),
    /// Try to fix anything that can be fixed automatically for each module.
    Fix(cli::check::Opts),
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
    let mut modules = model::load_modules(&root, git.as_ref())?;

    tracing::trace!(
        modules = modules
            .iter()
            .map(|m| m.path.to_string())
            .collect::<Vec<_>>()
            .join(", "),
        "loaded modules"
    );

    let config = config::load(&root, &templating, &modules)?;

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

    let current_path = if !opts.all && modules.iter().any(|m| m.path.as_ref() == current_path) {
        Some(current_path.as_ref())
    } else {
        None
    };

    let mut filters = Vec::new();

    for module in &opts.modules {
        filters.push(Fragment::parse(module));
    }

    filter_modules(
        &root,
        &opts,
        git.as_ref(),
        &mut modules,
        &filters,
        current_path,
    )?;

    let cx = ctxt::Ctxt {
        root: &root,
        config: &config,
        actions: &actions,
        modules: &modules,
        github_auth,
        rustc_version: ctxt::rustc_version(),
        git,
    };

    match opts.action.unwrap_or_default() {
        Action::Check(opts) => {
            cli::check::entry(&cx, &opts, false).await?;
        }
        Action::Fix(opts) => {
            cli::check::entry(&cx, &opts, true).await?;
        }
        Action::For(opts) => {
            cli::foreach::entry(&cx, &opts)?;
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
    }

    Ok(())
}

/// Perform more advanced filtering over modules.
fn filter_modules(
    root: &Path,
    opts: &Opts,
    git: Option<&git::Git>,
    modules: &mut [model::Module],
    filters: &[Fragment<'_>],
    current_path: Option<&RelativePath>,
) -> Result<(), anyhow::Error> {
    // Test if module should be skipped.
    let should_disable = |module: &Module| -> bool {
        if filters.is_empty() {
            if let Some(path) = current_path {
                return path != module.path.as_ref();
            }

            return false;
        }

        !filters
            .iter()
            .any(|filter| filter.is_match(module.path.as_str()))
    };

    for module in modules {
        module.disabled = should_disable(module);

        if module.disabled {
            continue;
        }

        if opts.needs_git() {
            let git = git.context("no working git command")?;
            let module_path = module.path.to_path(root);

            let cached = git.is_cached(&module_path)?;
            let dirty = git.is_dirty(&module_path)?;

            let span =
                tracing::trace_span!("git", ?cached, ?dirty, module = module.path.to_string());
            let _enter = span.enter();

            if opts.dirty && !dirty {
                tracing::trace!("directory is not dirty");
                module.disabled = true;
            }

            if opts.cached && !cached {
                tracing::trace!("directory has no cached changes");
                module.disabled = true;
            }

            if opts.cached_only && (!cached || dirty) {
                tracing::trace!("directory has no cached changes");
                module.disabled = true;
            }

            if opts.unreleased {
                if let Some((tag, offset)) = git.describe_tags(&module_path)? {
                    if offset.is_none() {
                        tracing::trace!("no offset detected (tag: {tag})");
                        module.disabled = true;
                    }
                } else {
                    tracing::trace!("no tags to describe");
                    module.disabled = true;
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
