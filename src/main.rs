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
//! <br>
//!
//! ## Overview
//!
//! This is an overview of the sections in the README:
//!
//! * [The `Kick.toml` configuration][config]
//! * [Tour of commands](#tour-of-commands)
//! * [Run Github Workflows locally](#run-github-workflows-locally)
//! * [Maintaining Github Actions](#github-actions)
//! * [Staged changes](#staged-changes)
//! * [Running commands over repo sets](#repo-sets)
//! * [Easily package your project](#packaging)
//! * [Flexible version specifications](#version-specification)
//! * [Integrating with Github Actions](#integrating-with-github-actions)
//!
//! <br>
//!
//! ## Introduction
//!
//! Kick can also be used *without* configuration in any standalone repository.
//! This is really all you need to get started, I frequently make use of `kick`
//! commands in regular repositories. The only pre-requisite is that there is a
//! `.git` repo with an `origin` specified:
//!
//! ```sh
//! $> kick check
//! README.md:
//! 31   > Note that kick uses a nondestructive approach, so running any command like
//! 32   > `kick check` is completely safe. To apply any proposed changes they can
//! 33   > either be reviewed later with `kick changes` or applied directly by
//! 34  -> specifying `--save`.
//!     +> specifying `--save`. See [Staged changes](#staged-changes) for more.
//! 35
//! 36   The other alternative is to run kick over a collection of repositories. To
//! 37   add a repo to kick you can add the following to a `Kick.toml` file:
//! 2025-12-06T06:50:42.488966Z  INFO kick: Writing to changes.gz, use `kick changes` to review it later
//! ```
//!
//! > Note that kick uses a nondestructive approach, so running any command like
//! > `kick check` is completely safe. To apply any proposed changes they can
//! > either be reviewed later with `kick changes` or applied directly by
//! > specifying `--save`. See [Staged changes](#staged-changes) for more.
//!
//! The other alternative is to run kick over a collection of repositories. To
//! add a repo to kick you can add the following to a `Kick.toml` file:
//!
//! ```toml
//! [repo."repos/OxidizeBot"]
//! url = "https://github.com/udoprog/OxidizeBot"
//! ```
//!
//! This can also be added as a git submodule, note that the important part is
//! what's in the `.gitmodules` file:
//!
//! ```bash
//! git submodule add https://github.com/udoprog/OxidizeBot repos/OxidizeBot
//! ```
//!
//! Once this is done, kick can run any command over a collection of repos:
//!
//! ```sh
//! $> kick gh status
//! repos/anything: https://github.com/udoprog/anything
//!   Workflow `ci` (success):
//!     git: *2ab2ad7 (main)
//!     time: 2025-11-28
//! repos/async-fuse: https://github.com/udoprog/async-fuse
//!   Workflow `ci` (success):
//!     git: *4062549 (main)
//!     time: 2025-11-29
//! repos/argwerk: https://github.com/udoprog/argwerk
//!   Workflow `ci` (success):
//!     git: *4b6377c (main)
//!     time: 2025-11-27
//! ```
//!
//! If you want a complete example of this setup, see [my `projects` repo]. For
//! documentation on how kick can be further configured, see the [configuration
//! documentation][config].
//!
//! [my `projects` repo]: https://github.com/udoprog/projects
//!
//! <br>
//!
//! ## Configuration
//!
//! Kick optionally reads `Kick.toml`, for how to configure projects. See the
//! [configuration documentation][config].
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
//!   `kick run --unreleased`.
//! * Want to do something with every project that is out-of-sync with their
//!   remote? Try `kick run --outdated`.
//!
//! And much much more!
//!
//! <br>
//!
//! ## Run Github Workflows locally
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
//! ## Maintaining Github Actions
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
//! 2023-04-13T15:05:34.162252Z  INFO kick: Writing to changes.gz, use `kick changes` to review it later
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
//! ## Packaging
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
//! The `msi` action builds an MSI package.
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
//! ## Integrating with Github Actions
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
//! [config]: https://github.com/udoprog/kick/blob/main/config.md
//! [wobbly-versions]: https://github.com/udoprog/kick/blob/main/WOBBLY_VERSIONS.md

#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

mod action;
mod actions;
mod cargo;
mod changes;
mod cli;
mod commands;
mod config;
mod ctxt;
mod deb;
mod edits;
mod env;
mod file;
mod fs;
mod gitmodules;
mod gix;
mod glob;
mod keys;
mod model;
mod musli;
mod octokit;
mod once;
mod packaging;
mod process;
mod release;
mod repo_sets;
mod restore;
mod rstr;
mod shell;
mod system;
mod template;
mod templates;
mod urls;
mod utils;
mod wix;
mod workflows;
mod workspace;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand};

use config::{Config, Distribution, Os};
use directories::ProjectDirs;
use env::SecretString;
use model::State;
use relative_path::{RelativePath, RelativePathBuf};
use repo_sets::RepoSet;
use termcolor::{ColorChoice, StandardStream};

use crate::ctxt::Paths;
use crate::env::Env;
use crate::model::{RepoSource, load_from_git};
use crate::{glob::Fragment, model::Repo};

/// The version of kick in use.
const VERSION: &str = const {
    match option_env!("KICK_VERSION") {
        Some(version) => version,
        None => env!("CARGO_PKG_VERSION"),
    }
};

const OWNER: &str = "udoprog";
const REPO: &str = "kick";

/// Name of project configuration files.
const KICK_TOML: &str = "Kick.toml";

/// Name of the github token file.
const GITHUB_TOKEN: &str = ".github-token";

/// User agent to use for http requests.
static USER_AGENT: reqwest::header::HeaderValue =
    reqwest::header::HeaderValue::from_static("kick/0.0");

#[derive(Subcommand)]
enum Command {
    /// Checks each repo (default action).
    Check(SharedAction<cli::check::Opts>),
    /// Review or apply staged changes.
    Changes(SharedOptions),
    /// List paths used by kick.
    Info(SharedOptions),
    /// Update Kick itself.
    Update(SharedOptions),
    /// Collect and define release variables.
    Define(SharedAction<cli::define::Opts>),
    /// Make sure you are logged into Github to access the API without rate
    /// limiting.
    Login(SharedAction<cli::login::Opts>),
    /// Manage sets.
    Set(SharedAction<cli::set::Opts>),
    /// Run a custom command.
    Run(SharedAction<cli::run::Opts>),
    /// Find the minimum supported rust version.
    Msrv(SharedAction<cli::msrv::Opts>),
    /// Modify package versions.
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
    /// Run a github action.
    GithubAction(SharedAction<cli::github_action::Opts>),
    /// Interact with the github API parameterized over repositories.
    #[command(name = "gh")]
    Github(SharedAction<cli::gh::Opts>),
}

impl Command {
    fn requires_token(&self) -> bool {
        matches!(self, Command::Github(..))
    }

    fn shared(&self) -> &SharedOptions {
        match self {
            Command::Check(action) => &action.shared,
            Command::Changes(shared) => shared,
            Command::Info(shared) => shared,
            Command::Update(shared) => shared,
            Command::Define(action) => &action.shared,
            Command::Login(action) => &action.shared,
            Command::Set(action) => &action.shared,
            Command::Run(action) => &action.shared,
            Command::Github(action) => &action.shared,
            Command::Msrv(action) => &action.shared,
            Command::Version(action) => &action.shared,
            Command::Publish(action) => &action.shared,
            Command::Upgrade(action) => &action.shared,
            Command::Msi(action) => &action.shared,
            Command::Rpm(action) => &action.shared,
            Command::Deb(action) => &action.shared,
            Command::Zip(action) => &action.shared,
            Command::Gzip(action) => &action.shared,
            Command::GithubAction(action) => &action.shared,
        }
    }

    fn repo(&self) -> Option<&RepoOptions> {
        match self {
            Command::Check(action) => Some(&action.repo),
            Command::Changes(..) => None,
            Command::Info(..) => None,
            Command::Update(..) => None,
            Command::Define(..) => None,
            Command::Login(..) => None,
            Command::Set(action) => Some(&action.repo),
            Command::Run(action) => Some(&action.repo),
            Command::Github(action) => Some(&action.repo),
            Command::Msrv(action) => Some(&action.repo),
            Command::Version(action) => Some(&action.repo),
            Command::Publish(action) => Some(&action.repo),
            Command::Upgrade(action) => Some(&action.repo),
            Command::Msi(action) => Some(&action.repo),
            Command::Rpm(action) => Some(&action.repo),
            Command::Deb(action) => Some(&action.repo),
            Command::Zip(action) => Some(&action.repo),
            Command::Gzip(action) => Some(&action.repo),
            Command::GithubAction(action) => Some(&action.repo),
        }
    }

    #[inline]
    fn needs_ctrlc_handler(&self) -> bool {
        !matches!(self, Command::Login(..))
    }
}

impl Default for Command {
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
    /// Enable debug level logging.
    #[arg(long)]
    debug: bool,
    /// Provide an access token to use to access the Github API.
    ///
    /// This can also be set through the `GITHUB_TOKEN` environment variable, or
    /// by writing the token to a `.github-token` file in the root of the
    /// project.
    #[arg(long, value_name = "token")]
    github_token: Option<SecretString>,
    /// List all found system tools.
    #[arg(long)]
    list_tools: bool,
}

impl SharedOptions {
    fn directive(&self) -> &'static str {
        if self.trace {
            return "kick=trace";
        }

        if self.debug {
            return "kick=debug";
        }

        "kick=info"
    }
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
    supported_os: bool,
    /// Load sets with the given id.
    #[arg(long, value_name = "set")]
    set: Vec<String>,
    /// Save remaining or failed repos to the specified set.
    ///
    /// In case an operation is cancelled, or for repos where the operation
    /// fails, this will cause the remaining repos to be saved to the set of the
    /// specified names.
    #[arg(long, value_name = "set")]
    set_remaining: Vec<String>,
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
    A: Args,
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
    action: Command,
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

    let default_directive = opts.action.shared().directive();
    let filter = tracing_subscriber::EnvFilter::builder();

    let filter = if let Ok(var) = std::env::var("RUST_LOG") {
        let var = format!("{var},{default_directive}");
        filter.parse_lossy(var)
    } else {
        filter.parse(default_directive)?
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .try_init()
        .map_err(|e| anyhow::anyhow!("{e}"))?;

    entry(opts).await
}

async fn entry(opts: Opts) -> Result<ExitCode> {
    let term = Arc::new(AtomicBool::new(false));

    let shared = opts.action.shared();

    if opts.action.needs_ctrlc_handler() {
        ctrlc::try_set_handler({
            let term = term.clone();

            move || {
                term.store(true, Ordering::SeqCst);
            }
        })?;
    }

    let repo_opts = opts.action.repo();

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
            let found = find_from_current_dir(&current_dir);

            match found {
                Some((root, current_path)) => (root, Some(current_path)),
                None => (current_dir, Some(RelativePathBuf::new())),
            }
        }
    };

    let project_dirs = ProjectDirs::from("se", "tedro", "kick");

    let paths = Paths {
        root: &root,
        current: current_path.as_deref(),
        config: project_dirs.as_ref().map(|p| p.config_dir()),
        cache: project_dirs.as_ref().map(|p| p.cache_dir()),
    };

    tracing::trace!(?paths, "Using paths");

    let mut env = Env::new();
    tracing::trace!(?env, "Using environment");

    if let Some(github_token) = &shared.github_token {
        env.github_tokens
            .push(env::GithubToken::cli(github_token.clone()));
    }

    for p in paths
        .config
        .into_iter()
        .map(|p| p.join(GITHUB_TOKEN))
        .chain([root.join(GITHUB_TOKEN)])
    {
        if let Some(secret) = crate::env::read_secret_string(&p)? {
            env.github_tokens.push(env::GithubToken::path(&p, secret));
        }

        let issues = self::fs::test_secure(&p);

        for issue in issues {
            if let Err(e) = issue.fix(&p).with_context(|| p.display().to_string()) {
                tracing::error!("{}: {issue}: {e}", p.display());
            }
        }
    }

    if env.github_tokens.is_empty() {
        if opts.action.requires_token() {
            tracing::warn!("No .github-token or --token argument found");
        } else {
            tracing::trace!(
                "No .github-token or --token argument found, heavy rate limiting will apply and unless specified some actions will not work"
            )
        }
    }

    env.update_from_env();

    let templating = templates::Templating::new()?;
    let extra_repos = model::load_gitmodules(&root)?;

    let defaults = config::defaults();

    let config = config::load(paths, &templating, extra_repos, &defaults)
        .context("Loading kick configuration")?;

    let os = match std::env::consts::OS {
        "linux" => Os::Linux,
        "windows" => Os::Windows,
        "macos" => Os::Mac,
        other => Os::Other(other.into()),
    };

    let dist = match &os {
        Os::Linux => Distribution::linux_distribution().unwrap_or_default(),
        _ => Distribution::Other,
    };

    let system = system::detect()?;

    if shared.list_tools {
        println!("Os: {os}, Dist: {dist}");

        let shell = os.shell();

        for (name, path, extra) in system.tools() {
            let path = path.to_string_lossy();
            let path = shell.escape(path.as_ref());

            if let Some(extra) = extra {
                println!("{name} ({extra}): {path}");
            } else {
                println!("{name}: {path}");
            }
        }
    } else {
        tracing::debug!("Os: {os}, Dist: {dist}");
    }

    let mut from_group = true;
    let mut repos = Vec::new();

    for (path, config) in config.repos.iter() {
        let Some(url) = config.urls.iter().next() else {
            continue;
        };

        repos.push(Repo::new(
            [RepoSource::Config(path.clone())],
            path.to_owned(),
            url.clone(),
        ));
    }

    tracing::trace!(
        modules = repos
            .iter()
            .map(|m| m.path().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        "loaded modules"
    );

    if repos.is_empty() {
        from_group = false;

        if let Some((path, url)) = load_from_git(&root, system.git.first())? {
            repos.push(Repo::new(BTreeSet::from([RepoSource::Git]), path, url));
        }
    }

    let mut sets = repo_sets::RepoSets::new(root.join("sets"))?;

    // This is `true` if the current directory is currently inside one of the
    // repos.
    let in_repo_path = paths
        .current
        .is_some_and(|p| repos.iter().any(|m| p.starts_with(m.path())));

    if let Some(opts) = repo_opts {
        apply_repo_options(
            opts,
            paths,
            &config,
            &os,
            &system,
            &repos,
            &sets,
            in_repo_path,
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
                tracing::trace!("Failed to get git credentials: {e}");
                None
            }
        },
        None => None,
    };

    let mut cx = ctxt::Ctxt {
        term,
        system: &system,
        git_credentials: &git_credentials,
        os,
        dist,
        paths,
        config: &config,
        repos: &repos,
        rustc_version: ctxt::rustc_version(),
        warnings: RefCell::new(Vec::new()),
        changes: RefCell::new(Vec::new()),
        sets: &mut sets,
        env: &env,
    };

    let with_repos = cli::WithReposImpl::new(&mut cx);

    match &opts.action {
        Command::Check(opts) => {
            cli::check::entry(with_repos, &opts.action).await?;
        }
        Command::Info(..) => {
            println!("Os: {}", cx.os);
            println!("Dist: {}", cx.dist);
            println!("Root: {}", cx.paths.root.display());

            if let Some(current) = cx.paths.current {
                println!("Current: {current}");
            }

            if let Some(config) = cx.paths.config {
                println!("Config: {}", config.display());
            }

            if let Some(cache) = cx.paths.cache {
                println!("Cache: {}", cache.display());
            }

            return Ok(ExitCode::SUCCESS);
        }
        Command::Changes(..) => {
            cli::changes::entry(&cx, &changes_path)?;
        }
        Command::Update(shared) => {
            cli::update::entry(&mut cx, shared).await?;
            return Ok(ExitCode::SUCCESS);
        }
        Command::Define(opts) => {
            cli::define::entry(with_repos, &opts.action)?;
            return Ok(ExitCode::SUCCESS);
        }
        Command::Login(opts) => {
            cli::login::entry(&mut cx, &opts.action)?;
            return Ok(ExitCode::SUCCESS);
        }
        Command::Set(opts) => {
            cli::set::entry(&mut cx, &opts.action)?;
        }
        Command::Run(opts) => {
            cli::run::entry(with_repos, &opts.action)?;
        }
        Command::Msrv(opts) => {
            cli::msrv::entry(with_repos, &opts.action)?;
        }
        Command::Version(opts) => {
            cli::version::entry(with_repos, &opts.action)?;
        }
        Command::Publish(opts) => {
            cli::publish::entry(with_repos, &opts.action)?;
        }
        Command::Upgrade(opts) => {
            cli::upgrade::entry(with_repos, &opts.action)?;
        }
        Command::Msi(opts) => {
            cli::msi::entry(with_repos, &opts.action)?;
        }
        Command::Rpm(opts) => {
            cli::rpm::entry(with_repos, &opts.action)?;
        }
        Command::Deb(opts) => {
            cli::deb::entry(with_repos, &opts.action)?;
        }
        Command::Zip(opts) => {
            cli::compress::entry(with_repos, cli::compress::Kind::Zip, &opts.action)?;
        }
        Command::Gzip(opts) => {
            cli::compress::entry(with_repos, cli::compress::Kind::Gzip, &opts.action)?;
        }
        Command::GithubAction(opts) => {
            cli::github_action::entry(with_repos, &opts.action)?;
        }
        Command::Github(opts) => {
            cli::gh::entry(with_repos, &opts.action).await?;
        }
    }

    let mut o = StandardStream::stdout(ColorChoice::Auto);

    for warning in cx.warnings().iter() {
        changes::report(&mut o, &cx, warning)?;
    }

    for change in cx.changes_mut().iter_mut() {
        if change.written {
            continue;
        }

        match changes::apply(&mut o, &cx, &change.change, shared.save) {
            Ok(()) => {
                if shared.save {
                    change.written = true;
                }
            }
            Err(error) => {
                tracing::error!("Failed to apply change: {error}");

                for cause in error.chain().skip(1) {
                    tracing::error!("Caused by: {cause}");
                }
            }
        }
    }

    if cx.can_save() {
        tracing::info!(
            "Writing to {}, use `kick changes` to review it later",
            changes_path.display()
        );
        changes::save_changes(&cx.changes(), &changes_path)
            .with_context(|| anyhow!("{}", changes_path.display()))?;
    } else {
        match std::fs::remove_file(&changes_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => Err(error).context(changes_path.display().to_string())?,
        }
    }

    let outcome = cx.outcome();

    if let Some(opts) = repo_opts {
        let mut remaining = RepoSet::default();

        for repo in cx.repos() {
            if !matches!(repo.state(), State::Error | State::Pending) {
                remaining.insert(repo);
            }
        }

        for name in opts.set_remaining.iter() {
            sets.save(name, remaining.clone(), "Remaining repo set");
        }
    }

    if from_group && !in_repo_path {
        sets.commit()?;
    }

    Ok(outcome)
}

fn apply_repo_options(
    repo_opts: &RepoOptions,
    paths: Paths<'_>,
    config: &Config<'_>,
    os: &Os,
    system: &system::System,
    repos: &[Repo],
    sets: &repo_sets::RepoSets,
    in_repo_path: bool,
) -> Result<()> {
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

    let in_current_path = if !repo_opts.all && in_repo_path {
        paths.current
    } else {
        None
    };

    filter_repos(
        config,
        paths,
        in_current_path,
        repo_opts,
        system.git.first(),
        repos,
        &filters,
        set.as_ref(),
        os,
    )?;

    Ok(())
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
        if let Some(set) = set
            && !set.contains(repo.path())
        {
            return true;
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

        if repo_opts.supported_os {
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
