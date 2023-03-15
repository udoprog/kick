//! [<img alt="github" src="https://img.shields.io/badge/github-udoprog/kick-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/kick)
//! [<img alt="crates.io" src="https://img.shields.io/crates/v/kick.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/kick)
//! [<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-kick-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/kick)
//! [<img alt="build status" src="https://img.shields.io/github/actions/workflow/status/udoprog/kick/ci.yml?branch=main&style=for-the-badge" height="20">](https://github.com/udoprog/kick/actions?query=branch%3Amain)
mod actions;
mod cli;
mod config;
mod ctxt;
mod file;
mod git;
mod gitmodules;
mod manifest;
mod model;
mod rust_version;
mod templates;
mod urls;
mod utils;
mod validation;
mod workspace;

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use model::Module;

use actions::Actions;
use tracing::metadata::LevelFilter;

/// Name of project configuration files.
const KICK_TOML: &str = "Kick.toml";

#[derive(Subcommand)]
enum Action {
    /// Run checks for each repo.
    Check(cli::check::Opts),
    /// Fix repo.
    Fix(cli::check::Opts),
    /// Run a command for each repo.
    For(cli::foreach::Opts),
    /// Get the build status for each repo.
    Status(cli::status::Opts),
    /// Find the minimum supported rust version through bisection.
    Msrv(cli::msrv::Opts),
}

#[derive(Default, Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Opts {
    #[command(subcommand)]
    action: Option<Action>,
}

impl Default for Action {
    fn default() -> Self {
        Self::Check(cli::check::Opts::default())
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
    let root = find_root()?;

    let github_auth = match std::fs::read_to_string(root.join(".github-auth")) {
        Ok(auth) => Some(auth.trim().to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!("no .github-auth found, heavy rate limiting will apply");
            None
        }
        Err(e) => return Err(anyhow::Error::from(e)).with_context(|| anyhow!(".github-auth")),
    };

    let templating = templates::Templating::new()?;

    let config = {
        let config_path = root.join(KICK_TOML);
        config::load(&config_path, &templating)
            .with_context(|| config_path.to_string_lossy().into_owned())?
    };

    let opts = Opts::try_parse()?;

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

    let modules = model::load_modules(&root)?;

    let cx = ctxt::Ctxt {
        root: &root,
        config: &config,
        actions: &actions,
        modules,
        github_auth,
        rustc_version: ctxt::rustc_version(),
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
    }

    Ok(())
}

/// Test if module should be skipped.
fn should_skip(filters: &[String], module: &Module) -> bool {
    !filters.is_empty()
        && !filters
            .iter()
            .all(|filter| module.path.as_str().contains(filter))
}

/// Find root path to use.
fn find_root() -> Result<PathBuf> {
    let mut current = std::env::current_dir()?;

    loop {
        if current.join(KICK_TOML).is_file() {
            return Ok(current);
        }

        if !current.pop() {
            return Err(anyhow!("missing projects directory"));
        }
    }
}
