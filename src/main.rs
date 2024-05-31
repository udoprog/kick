//! [<img alt="github" src="https://img.shields.io/badge/github-udoprog/kick-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/kick)
//! [<img alt="crates.io" src="https://img.shields.io/crates/v/kick.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/kick)
//! [<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-kick-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/kick)
//!
//! Give your projects a good 🦶!
//!
//! This is what I'd like to call an omnibus project management tool. I'm
//! building it to do everything I need when managing my own projects to ensure
//! that they all have a valid configuration, up-to-date dependencies and a
//! consistent README style.
//!
//! Repositories to check are detected through two mechanism:
//! * If a `.gitmodules` file is present either in the current directory or the
//!   one where `Kick.toml` is found, this is used to detect repositories to
//!   manage.
//! * If a `.git` folder is present, `git remote get-url origin` is used to
//!   determine its name and repo.
//!
//! So the intent is primarily to use this separate from the projects being
//! managed, by adding each project as a submodule like so.
//!
//! ```bash
//! git submodule add https://github.com/udoprog/OxidizeBot repos/OxidizeBot
//! ```
//!
//! > **Note:** For an example of this setup, see [my `projects` repo].
//!
//! Kick can also be used without configuration in any standalone repository.
//! This is really all you need to get started, I frequently make use of `kick`
//! commands in regular repositories.
//!
//! [my `projects` repo]: https://github.com/udoprog/projects
//!
//! <br>
//!
//! ## Overview
//!
//! This is an overview of the sections in the README:
//!
//! * [Configuration](#configuration)
//! * [Tour of commands](#tour-of-commands)
//! * [Run Github Workflows](#run-github-workflows)
//! * [Github Actions](#github-actions)
//! * [Staged changes](#staged-changes)
//! * [Repo sets](#repo-sets)
//! * [Packaging actions](#packaging-actions)
//! * [Version specification](#version-specification)
//! * [Defining variables for Github Actions](#defining-variables-for-github-actions)
//!
//! <br>
//!
//! ## Configuration
//!
//! Kick optionally reads `Kick.toml`, for how to configure projects. See the
//! [configuration documentation].
//!
//! <br>
//!
//! ## Tour of commands
//!
//! This section details some of my favorite things that Kick can do for you.
//! For a complete list of options, make use of `--help`.
//!
//! Kick can `check`, which performs a project-specific sanity such as checking
//! that READMEs are up-to-date with their corresponding sources, badges are
//! configured, github actions are correctly configured and much more.
//!
//! Kick can effortlessly package your Rust projects using actions such
//! `gzip`,`zip`, or packaging systems such as `rpm`, `deb`, or `msi` preparing
//! them for distribution.
//!
//! Kick can run custom commands over git modules using convenient filters.
//! Combined with [repo sets](#repo-sets). Performing batch maintenance over
//! many git projects has never been easier!
//! * Want to do something with every project that hasn't been released yet? Try
//!   `kick for --unreleased`.
//! * Want to do something with every project that is out-of-sync with their
//!   remote? Try `kick for --outdated`.
//!
//! And much much more!
//!
//! <br>
//!
//! ## Run Github Workflows
//!
//! ![Matrix and WSL integration](https://raw.githubusercontent.com/udoprog/kick/main/images/wsl.png)
//!
//! Kick can run Github workflows locally using `kick run --job <job>`.
//!
//! This tries to use system utilities which are available locally in order to
//! run the workflow on the appropriate operating system as specified through
//! `runs-on`.
//!
//! This also comes with support for matrix expansion.
//!
//! Supported integrations are:
//! * Running on the same operating system as where Kick is run (default).
//! * Running Linux on Windows through WSL.
//!
//! <br>
//!
//! ## Github Actions
//!
//! Kick shines the brightest when used in combination with Github Actions. To
//! facilitate this, the Kick repo can be used in a job directly:
//!
//! ```yaml
//! jobs:
//!   build:
//!   - uses: udoprog/kick@nightly
//!   - run: kick --version
//! ```
//!
//! In particular it is useful to specify a global `KICK_VERSION` using the
//! [wobbly version specification][wobbly-versions] so that all kick commands
//! that run will use the same version number.
//!
//! ```yaml
//! # If the `version` input is not available through a `workflow_dispatch`, defaults to a dated release.
//! env:
//!   KICK_VERSION: "${{github.event.inputs.version}} || %date"
//! ```
//!
//! <br>
//!
//! ## Staged changes
//!
//! If you specify `--save`, proposed changes that can be applied to a project
//! will be applied. If `--save` is not specified the collection of changes will
//! be saved to `changes.gz` (in the root) to be applied later using `kick
//! apply`.
//!
//! ```text
//! > kick check
//! repos/kick/README.md: Needs update
//! repos/kick/src/main.rs: Needs update
//! 2023-04-13T15:05:34.162247Z  WARN kick: Not writing changes since `--save` was not specified
//! 2023-04-13T15:05:34.162252Z  INFO kick: Writing commit to ../changes.gz, use `kick changes` to review it later
//! ```
//!
//! Applying the staged changes:
//!
//! ```text
//! > kick changes --save
//! repos/kick/README.md: Fixing
//! repos/kick/src/main.rs: Fixing
//! 2023-04-13T15:06:23.478579Z  INFO kick: Removing ../changes.gz
//! ```
//!
//! <br>
//!
//! ## Repo sets
//!
//! Commands can produce sets under certain circumstances, the sets are usually
//! called `good` and `bad` depending on the outcome when performing the work
//! over the repo.
//!
//! If this is set during a run, it will store sets of repos, such as the set
//! for which a command failed. This set can then later be re-used through the
//! `--set <id>` switch.
//!
//! For a list of available sets, you can simply list the `sets` folder:
//!
//! ```text
//! sets\bad
//! sets\bad-20230414050517
//! sets\bad-20230414050928
//! sets\bad-20230414051046
//! sets\good-20230414050517
//! sets\good-20230414050928
//! sets\good-20230414051046
//! ```
//!
//! > **Note** the three most recent versions of each set will be retained.
//!
//! Set files are simply lists of repositories, which supports comments by
//! prefixing lines with `#`. They are intended to be edited by hand if needed.
//!
//! ```text
//! repos/kick
//! # ignore this for now
//! # repos/unsync
//! ```
//!
//! <br>
//!
//! ## Packaging actions
//!
//! The following actions are packaging actions:
//! * `zip` - Build .zip archives.
//! * `gzip` - Build .tar.gz archives.
//! * `msi` - Build .msi packages using wix.
//! * `rpm` - Build .rpm packages (builtin method).
//! * `deb` - Build .deb packages (builtin method).
//!
//! These all look at the `[package]` section in the configuration to determine
//! what to include in a given package. For example:
//!
//! ```toml
//! [[package.files]]
//! source = "desktop/se.tedro.JapaneseDictionary.desktop"
//! dest = "/usr/share/applications/"
//! mode = "600"
//! ```
//!
//! Note that:
//! * The default mode for files is 655.
//! * Where approproate, the default version specification is a wildcard version, or `*`.
//!
//! When a version specification is used, it supports the following formats:
//! * `*` - any version.
//! * `= 1.2.3` - exact version.
//! * `> 1.2.3` - greater than version.
//! * `>= 1.2.3` - greater than or equal to version.
//! * `< 1.2.3` - less than version.
//! * `<= 1.2.3` - less than or equal to version.
//!
//! <br>
//!
//! ### `rpm` specific settings
//!
//! For the `rpm` action, you can specify requires to add to the generated
//! archive in `Kick.toml`:
//!
//! ```toml
//! [[package.rpm.requires]]
//! package = "tesseract-langpack-jpn"
//! version = ">= 4.1.1"
//! ```
//!
//! <br>
//!
//! ### `deb` specific settings
//!
//! For the `deb` action, you can specify dependencies to add to the generated
//! archive in `Kick.toml`:
//!
//! ```toml
//! [[package.rpm.depends]]
//! package = "tesseract-ocr-jpn"
//! version = ">= 4.1.1"
//! ```
//!
//! <br>
//!
//! ### The `msi` action
//!
//! The `msi` action builds an MSI package for each repo.
//!
//! It is configured by a single `wix/<main>.wsx` file in the repo. For an
//! example, [see the `jpv` project].
//!
//! When building a wix package, we define the following variables that should
//! be used:
//! * `Root` - The root directory of the project. Use this for all files
//!   referenced.
//! * `Version` - The version of the package being build in the correct format
//!   the MSI expects.
//! * `Platform` - The platform the package is being built for. Either `x86` or
//!   `x64`. This is simply expected to be passed along to the `Platform`
//!   attribute in the `Package` element.
//! * `Win64` - Is either `x86_64` or `x86`. This is simply expected to be
//!   passed along to any elements with a `Win64` attribute.
//! * `ProgramFilesFolder` - The directory that corresponds to the
//!   platform-specific program files folder to use.
//! * `BinaryName` - The name of the main binary.
//! * `BinaryPath` - The path to the main binary. Should not be `Root` prefixed.
//!
//! [see the `jpv` project]: https://github.com/udoprog/jpv/tree/main/wix
//!
//! <br>
//!
//! ## Version specification
//!
//! Some actions need to determine a version to use, such as when creating a
//! github release or building a package.
//!
//! For these you can:
//! * Provide the version through the `--version <version>` switch.
//! * Defining the `KICK_VERSION` environment variable.
//!
//! This primarily supports plain versions, dates, or tags, such as `1.2.3`,
//! `2021-01-01`, or `nightly1` and will be coerced as appropriate into a
//! target version specification depending in which type of package is being
//! built.
//!
//! This also supports simple expressions such as `$VALUE || %date` which are
//! evaluated left-to-right and picks the first non-empty version defined.
//!
//! For a full specification of the supported format, see the [wobbly version
//! specification][wobbly-versions].
//!
//! <br>
//!
//! ## Defining variables for Github Actions
//!
//! Sometimes you want to export information from Kick so that it can be used in
//! other Github Actions, most commonly this involves the resolved version from
//! a [version specification](#version-specification).
//!
//! The `define` command can easily be used to achieve this:
//!
//! ```yaml
//! # If the `version` input is not available through a `workflow_dispatch`, defaults to a dated release.
//! env:
//!   KICK_VERSION: "${{github.event.inputs.version}} || %date"
//!
//! jobs:
//!   build:
//!   - uses: udoprog/kick@nightly
//!   - run: kick define --github-action
//!     id: release
//!   # echo the selected version
//!   - run: echo ${{steps.release.outputs.version}}
//!   # echo "yes" or "no" depending on if the version is a pre-release or not.
//!   - run: echo ${{steps.release.outputs.pre}}
//! ```
//!
//! Note that version information is exported by default when specifying
//! `--github-action`. For other information that can be exported, see `define
//! --help`.
//!
//! [configuration documentation]: https://github.com/udoprog/kick/blob/main/CONFIGURATION.md
//! [wobbly-versions]: https://github.com/udoprog/kick/blob/main/WOBBLY_VERSIONS.md

#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

mod actions;
mod cargo;
mod changes;
mod cli;
mod config;
mod ctxt;
mod deb;
mod edits;
mod env;
mod file;
mod gitmodules;
mod glob;
mod keys;
mod model;
mod musli;
mod octokit;
mod packaging;
mod process;
mod release;
mod repo_sets;
mod shell;
mod system;
mod templates;
mod urls;
mod wix;
mod workflows;
mod workspace;

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, FromArgMatches, Parser, Subcommand};

use config::{Config, Os};
use env::SecretString;
use relative_path::{RelativePath, RelativePathBuf};
use tracing::metadata::LevelFilter;
use tracing_subscriber::filter::Directive;

use crate::ctxt::Paths;
use crate::env::Env;
use crate::{glob::Fragment, model::Repo};

/// The version of kick in use.
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Name of project configuration files.
const KICK_TOML: &str = "Kick.toml";
/// User agent to use for http requests.
static USER_AGENT: reqwest::header::HeaderValue =
    reqwest::header::HeaderValue::from_static("kick/0.0");

#[derive(Subcommand)]
enum Action {
    /// Checks each repo (default action).
    Check(SharedAction<cli::check::Opts>),
    /// Review or apply staged changes.
    Changes(SharedOptions),
    /// Collect and define release variables.
    Define(SharedAction<cli::define::Opts>),
    /// Manage sets.
    Set(SharedAction<cli::set::Opts>),
    /// Run a custom command for each repo.
    Run(SharedAction<cli::run::Opts>),
    /// Fetch github actions build status for each repo.
    Status(SharedAction<cli::status::Opts>),
    /// Find the minimum supported rust version for each repo.
    Msrv(SharedAction<cli::msrv::Opts>),
    /// Update package version.
    Version(SharedAction<cli::version::Opts>),
    /// Publish packages in reverse order of dependencies.
    Publish(SharedAction<cli::publish::Opts>),
    /// Perform repository aware `cargo upgrade`.change
    Upgrade(SharedAction<cli::upgrade::Opts>),
    /// Build an .msi package (using wix).
    Msi(SharedAction<cli::msi::Opts>),
    /// Build an .rpm package (builtin).
    Rpm(SharedAction<cli::rpm::Opts>),
    /// Build an .deb package (builtin).
    Deb(SharedAction<cli::deb::Opts>),
    /// Build a .zip package.
    Zip(SharedAction<cli::compress::Opts>),
    /// Build a .tar.gz package.
    Gzip(SharedAction<cli::compress::Opts>),
    /// Build a github release.
    GithubRelease(SharedAction<cli::github_release::Opts>),
}

impl Action {
    fn requires_token(&self) -> bool {
        matches!(self, Action::GithubRelease(..))
    }

    fn shared(&self) -> &SharedOptions {
        match self {
            Action::Check(action) => &action.shared,
            Action::Changes(shared) => shared,
            Action::Define(action) => &action.shared,
            Action::Set(action) => &action.shared,
            Action::Run(action) => &action.shared,
            Action::Status(action) => &action.shared,
            Action::Msrv(action) => &action.shared,
            Action::Version(action) => &action.shared,
            Action::Publish(action) => &action.shared,
            Action::Upgrade(action) => &action.shared,
            Action::Msi(action) => &action.shared,
            Action::Rpm(action) => &action.shared,
            Action::Deb(action) => &action.shared,
            Action::Zip(action) => &action.shared,
            Action::Gzip(action) => &action.shared,
            Action::GithubRelease(action) => &action.shared,
        }
    }

    fn repo(&self) -> Option<&RepoOptions> {
        match self {
            Action::Check(action) => Some(&action.repo),
            Action::Changes(..) => None,
            Action::Define(..) => None,
            Action::Set(action) => Some(&action.repo),
            Action::Run(action) => Some(&action.repo),
            Action::Status(action) => Some(&action.repo),
            Action::Msrv(action) => Some(&action.repo),
            Action::Version(action) => Some(&action.repo),
            Action::Publish(action) => Some(&action.repo),
            Action::Upgrade(action) => Some(&action.repo),
            Action::Msi(action) => Some(&action.repo),
            Action::Rpm(action) => Some(&action.repo),
            Action::Deb(action) => Some(&action.repo),
            Action::Zip(action) => Some(&action.repo),
            Action::Gzip(action) => Some(&action.repo),
            Action::GithubRelease(action) => Some(&action.repo),
        }
    }
}

impl Default for Action {
    fn default() -> Self {
        Self::Check(SharedAction {
            shared: SharedOptions::default(),
            repo: RepoOptions::default(),
            action: cli::check::Opts::default(),
        })
    }
}

#[derive(Default, Parser)]
struct SharedOptions {
    /// Specify custom root folder for project hierarchy.
    #[arg(long, name = "root", value_name = "path")]
    root: Option<PathBuf>,
    /// Save any proposed or loaded changes.
    #[arg(long)]
    save: bool,
    /// Enable trace level logging.
    #[arg(long)]
    trace: bool,
    /// Provide an access token to use to access the Github API.
    ///
    /// This can also be set through the `GITHUB_TOKEN` environment variable, or
    /// by writing the token to a `.github-token` file in the root of the
    /// project.
    #[arg(long, value_name = "token")]
    github_token: Option<SecretString>,
}

#[derive(Default, Parser)]
struct RepoOptions {
    /// Force processing of all repos, even if the root is currently inside of
    /// an existing repo.
    #[arg(long)]
    all: bool,
    /// Only run the specified set of repos.
    #[arg(long = "path", short = 'p', name = "repos", value_name = "path")]
    repos: Vec<String>,
    /// If we should fetch the latest updates from remotes before filtering.
    #[arg(long)]
    fetch: bool,
    /// Only run over dirty modules with changes that have not been staged in
    /// cache.
    #[arg(long)]
    dirty: bool,
    /// Test if the repository is outdated.
    ///
    /// A repo is considered outdated if its branch is ahead of its remote.
    #[arg(long)]
    outdated: bool,
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
    /// Only run over repos which have declared that the same operating system
    /// is supported.
    #[arg(long)]
    same_os: bool,
    /// Load sets with the given id.
    #[arg(long, value_name = "set")]
    set: Vec<String>,
    /// Intersect with the specified set.
    #[arg(long, value_name = "set")]
    set_intersect: Vec<String>,
    /// Difference with the specified set.
    #[arg(long, value_name = "set")]
    set_difference: Vec<String>,
}

impl RepoOptions {
    fn needs_git(&self) -> bool {
        self.dirty || self.cached || self.cached_only || self.unreleased || self.outdated
    }
}

#[derive(Parser)]
#[command(version = None)]
struct SharedAction<A>
where
    A: FromArgMatches + Args,
{
    #[command(flatten)]
    action: A,
    #[command(flatten)]
    repo: RepoOptions,
    #[command(flatten)]
    shared: SharedOptions,
}

/// Give your projects a good 🦶!
///
/// Kick optionally reads Kick.toml, for how to configure projects. See the
/// configuration documentation:
///
/// https://github.com/udoprog/kick/blob/main/CONFIGURATION.md
#[derive(Default, Parser)]
#[command(author, version = crate::VERSION, max_term_width = 80)]
struct Opts {
    /// Action to perform. Defaults to `check`.
    #[command(subcommand, name = "action")]
    action: Option<Action>,
}

#[tokio::main]
async fn main() -> Result<ExitCode> {
    let opts = match Opts::try_parse() {
        Ok(opts) => opts,
        Err(error) => {
            match error.kind() {
                clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion => {
                    print!("{error}");
                }
                _ => {
                    return Err(error.into());
                }
            }

            return Ok(ExitCode::SUCCESS);
        }
    };

    let filter = if let Some(shared) = opts.action.as_ref().map(|a| a.shared()) {
        if shared.trace {
            Directive::from_str("kick=trace")?
        } else {
            Directive::from(LevelFilter::INFO)
        }
    } else {
        Directive::from(LevelFilter::INFO)
    };

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::builder()
                .with_default_directive(filter)
                .from_env_lossy(),
        )
        .try_init()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    entry(opts).await
}

async fn entry(opts: Opts) -> Result<ExitCode> {
    let action = opts.action.unwrap_or_default();
    let shared = action.shared();

    let repo_opts = action.repo();

    let (root, current_path) = match &shared.root {
        Some(root) => {
            let root = root.canonicalize()?;
            let current = std::env::current_dir()?.canonicalize()?;

            let current_path = if let Ok(prefix) = current.strip_prefix(&root) {
                Some(RelativePathBuf::from_path(prefix)?)
            } else {
                None
            };

            (root.to_owned(), current_path)
        }
        None => {
            let current_dir = std::env::current_dir().context("Getting current directory")?;

            match find_from_current_dir(&current_dir) {
                Some((root, current_path)) => (root, Some(current_path)),
                None => (current_dir, Some(RelativePathBuf::new())),
            }
        }
    };

    let paths = Paths {
        root: &root,
        current_path: current_path.as_deref(),
    };

    tracing::trace!(?paths, "Using project root");

    let mut env = Env::new();
    tracing::trace!(?env, "Using environment");

    if let Action::Define(opts) = &action {
        cli::define::entry(&env, &opts.action)?;
        return Ok(ExitCode::SUCCESS);
    };

    if let Some(github_token) = &shared.github_token {
        env.github_token = Some(github_token.clone());
    }

    if env.github_token.is_none() {
        let path = root.join(".github-token");
        env.github_token = crate::env::read_secret_string(path)?;
    }

    if env.github_token.is_none() {
        if action.requires_token() {
            tracing::warn!("No .github-token or --token argument found");
        } else {
            tracing::trace!("No .github-token or --token argument found, heavy rate limiting will apply and unless specified some actions will not work")
        }
    }

    let system = system::detect()?;

    let templating = templates::Templating::new()?;
    let repos = model::load_gitmodules(&root)?;

    let defaults = config::defaults();

    let config = config::load(
        paths,
        &templating,
        repos.as_deref().unwrap_or_default(),
        &defaults,
    )
    .context("Loading kick configuration")?;

    let repos = match repos {
        Some(repos) => repos,
        None => model::load_from_git(&root, system.git.first())?,
    };

    tracing::trace!(
        modules = repos
            .iter()
            .map(|m| m.path().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        "loaded modules"
    );

    let mut sets = repo_sets::RepoSets::new(root.join("sets"))?;

    let os = match std::env::consts::OS {
        "linux" => Os::Linux,
        "windows" => Os::Windows,
        "macos" => Os::Mac,
        other => Os::Other(other.into()),
    };

    if let Some(repo_opts) = repo_opts {
        let mut filters = Vec::new();

        for repo in &repo_opts.repos {
            filters.push(Fragment::parse(repo));
        }

        let set = match &repo_opts.set[..] {
            [] => None,
            ids => {
                let mut set = HashSet::new();

                for id in ids {
                    if let Some(s) = sets.load(id)? {
                        set.extend(s.iter().map(RelativePath::to_owned));
                    }
                }

                if !repo_opts.set_intersect.is_empty() {
                    let mut intersect = HashSet::new();

                    for id in &repo_opts.set_intersect {
                        if let Some(s) = sets.load(id)? {
                            intersect.extend(s.iter().map(RelativePath::to_owned));
                        }
                    }

                    set = &set & &intersect;
                }

                if !repo_opts.set_difference.is_empty() {
                    let mut intersect = HashSet::new();

                    for id in &repo_opts.set_difference {
                        if let Some(s) = sets.load(id)? {
                            intersect.extend(s.iter().map(RelativePath::to_owned));
                        }
                    }

                    set = &set ^ &intersect;
                }

                Some(set)
            }
        };

        let in_current_path = paths
            .current_path
            .filter(|p| !repo_opts.all && repos.iter().any(|m| p.starts_with(m.path())));

        filter_repos(
            &config,
            paths,
            in_current_path,
            repo_opts,
            system.git.first(),
            &repos,
            &filters,
            set.as_ref(),
            &os,
        )?;
    }

    let changes_path = root.join("changes.gz");

    let git_credentials = match system.git.first() {
        Some(git) => match git.get_credentials("github.com", "https") {
            Ok(credentials) => {
                tracing::trace!("Using git credentials for github.com");
                Some(credentials)
            }
            Err(e) => {
                tracing::warn!("Failed to get git credentials: {e}");
                None
            }
        },
        None => None,
    };

    let mut cx = ctxt::Ctxt {
        system: &system,
        git_credentials: &git_credentials,
        os,
        paths,
        config: &config,
        repos: &repos,
        rustc_version: ctxt::rustc_version(),
        warnings: RefCell::new(Vec::new()),
        changes: RefCell::new(Vec::new()),
        sets: &mut sets,
        env: &env,
    };

    match &action {
        Action::Check(opts) => {
            cli::check::entry(&mut cx, &opts.action).await?;
        }
        Action::Changes(shared) => {
            cli::changes::entry(&mut cx, shared, &changes_path)?;
            return Ok(ExitCode::SUCCESS);
        }
        Action::Set(opts) => {
            cli::set::entry(&mut cx, &opts.action)?;
        }
        Action::Run(opts) => {
            cli::run::entry(&mut cx, &opts.action)?;
        }
        Action::Status(opts) => {
            cli::status::entry(&mut cx, &opts.action).await?;
        }
        Action::Msrv(opts) => {
            cli::msrv::entry(&mut cx, &opts.action)?;
        }
        Action::Version(opts) => {
            cli::version::entry(&mut cx, &opts.action)?;
        }
        Action::Publish(opts) => {
            cli::publish::entry(&mut cx, &opts.action)?;
        }
        Action::Upgrade(opts) => {
            cli::upgrade::entry(&mut cx, &opts.action)?;
        }
        Action::Msi(opts) => {
            cli::msi::entry(&mut cx, &opts.action)?;
        }
        Action::Rpm(opts) => {
            cli::rpm::entry(&mut cx, &opts.action)?;
        }
        Action::Deb(opts) => {
            cli::deb::entry(&mut cx, &opts.action)?;
        }
        Action::Zip(opts) => {
            cli::compress::entry(&mut cx, cli::compress::Kind::Zip, &opts.action)?;
        }
        Action::Gzip(opts) => {
            cli::compress::entry(&mut cx, cli::compress::Kind::Gzip, &opts.action)?;
        }
        Action::GithubRelease(opts) => {
            cli::github_release::entry(&mut cx, &opts.action).await?;
        }
        _ => {
            bail!("Unsupported action at this stage")
        }
    }

    for warning in cx.warnings().iter() {
        crate::changes::report(&cx, warning)?;
    }

    for change in cx.changes().iter() {
        crate::changes::apply(&cx, change, shared.save)?;
    }

    if cx.can_save() && !shared.save {
        tracing::warn!("Not writing changes since `--save` was not specified");
        tracing::info!(
            "Writing commit to {}, use `kick changes` to review it later",
            changes_path.display()
        );
        changes::save_changes(&cx, &changes_path)
            .with_context(|| anyhow!("{}", changes_path.display()))?;
    }

    let outcome = cx.outcome();
    sets.commit()?;
    Ok(outcome)
}

/// Perform more advanced filtering over modules.
fn filter_repos(
    config: &Config,
    paths: Paths<'_>,
    in_current_path: Option<&RelativePath>,
    repo_opts: &RepoOptions,
    git: Option<&system::Git>,
    repos: &[model::Repo],
    filters: &[Fragment<'_>],
    set: Option<&HashSet<RelativePathBuf>>,
    expected: &Os,
) -> Result<()> {
    // Test if repo should be skipped.
    let should_disable = |repo: &Repo| -> bool {
        if let Some(set) = set {
            if !set.contains(repo.path()) {
                return true;
            }
        }

        if filters.is_empty() {
            if let Some(path) = in_current_path {
                return !path.starts_with(repo.path());
            }

            return false;
        }

        !filters
            .iter()
            .any(|filter| filter.is_match(repo.path().as_str()))
    };

    for repo in repos {
        if should_disable(repo) {
            repo.disable();
        }

        if repo.is_disabled() {
            continue;
        }

        if repo_opts.same_os {
            let os = config.os(repo);

            if !os.is_empty() && !os.contains(expected) {
                tracing::trace!("Operating systems {os:?} does not contain {expected:?}");
                repo.disable();
            }
        }

        if repo_opts.needs_git() {
            let git = git.context("no working git command found")?;
            let repo_path = paths.to_path(repo.path());

            let cached = git.is_cached(&repo_path)?;
            let dirty = git.is_dirty(&repo_path)?;

            let span = tracing::trace_span!("git", ?cached, ?dirty, repo = repo.path().to_string());
            let _enter = span.enter();

            let disable = 'outcome: {
                if repo_opts.dirty && !dirty {
                    tracing::trace!("Directory is not dirty");
                    break 'outcome true;
                }

                if repo_opts.cached && !cached {
                    tracing::trace!("Directory has no cached changes");
                    break 'outcome true;
                }

                if repo_opts.cached_only && (!cached || dirty) {
                    tracing::trace!("Directory has no cached changes");
                    break 'outcome true;
                }

                if repo_opts.outdated
                    && (!dirty && !git.is_outdated(&repo_path, repo_opts.fetch)?)
                {
                    tracing::trace!("Directory is not outdated");
                    break 'outcome true;
                }

                if repo_opts.unreleased {
                    if let Some(describe) = git.describe_tags(&repo_path, repo_opts.fetch)? {
                        if describe.offset.is_none() {
                            tracing::trace!("No offset detected (tag: {})", describe.tag);
                            break 'outcome true;
                        }
                    } else {
                        tracing::trace!("No tags to describe");
                        break 'outcome true;
                    }
                }

                false
            };

            if disable {
                repo.disable();
            }
        }
    }

    Ok(())
}

/// Find root path to use.
fn find_from_current_dir(current_dir: &Path) -> Option<(PathBuf, RelativePathBuf)> {
    fn clone_or_current(path: &Path) -> PathBuf {
        if path.components().next().is_none() {
            PathBuf::from_iter([Component::CurDir])
        } else {
            path.to_owned()
        }
    }

    let mut parent = current_dir.to_owned();

    let mut path = PathBuf::new();
    let mut relative = Vec::<String>::new();

    let mut last_kick_toml = None;
    let mut first_git = None;

    loop {
        if first_git.is_none() {
            let git = parent.join(".git");

            if git.exists() {
                tracing::trace!("Found .git in {}", git.display());
                first_git = Some((clone_or_current(&path), relative.iter().rev().collect()));
            }
        }

        let kick_toml = parent.join(KICK_TOML);

        if kick_toml.is_file() {
            tracing::trace!("Found {KICK_TOML} in {}", kick_toml.display());
            last_kick_toml = Some((clone_or_current(&path), relative.iter().rev().collect()));
        }

        let Some(Component::Normal(normal)) = parent.components().next_back() else {
            break;
        };

        relative.push(normal.to_string_lossy().into_owned());

        path.push(Component::ParentDir);
        parent.pop();
    }

    if let Some((path, relative)) = last_kick_toml {
        return Some((path, relative));
    }

    first_git
}
