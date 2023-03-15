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

    let mut buf = Vec::new();
    let modules = model::load_gitmodules(&root, &mut buf)?;

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
fn should_skip(filters: &[String], module: &Module<'_>) -> bool {
    !filters.is_empty() && !filters.iter().all(|filter| module.name.contains(filter))
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
