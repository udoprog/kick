//! [<img alt="github" src="https://img.shields.io/badge/github-udoprog/kick-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/kick)
//! [<img alt="crates.io" src="https://img.shields.io/crates/v/kick.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/kick)
//! [<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-kick-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/kick)
//!
//! Give your projects a good 🦶!
//!
//! This is what I'd like to call an omnibus project management tool. I'm
//! building it to do everything I need when managing my own projects to ensure
//! that they all have a valid configuration, up-to-date dependencies and a
//! consistent style.
//!
//! Even though it's a bit opinionated `kick` also happens to be highly
//! configurable, so you might want to try it out!
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

use core::str::{Chars, FromStr};
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

use crate::cli::WithRepos;
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

/// Default parallelism to use.
const PARALLELISM: &str = "8";

/// User agent to use for http requests.
static USER_AGENT: reqwest::header::HeaderValue =
    reqwest::header::HeaderValue::from_static("kick/0.0");

#[derive(Subcommand)]
enum Command {
    /// Review or apply staged changes.
    Changes(SharedOptions),
    /// Checks each repo (default action).
    Check(SharedAction<cli::check::Opts>),
    /// Build an .deb package (builtin).
    Deb(SharedAction<cli::deb::Opts>),
    /// Collect and define release variables.
    Define(SharedAction<cli::define::Opts>),
    /// Interact with the github API parameterized over repositories.
    #[command(name = "gh")]
    Github(SharedAction<cli::gh::Opts>),
    /// Run a github action.
    GithubAction(SharedAction<cli::github_action::Opts>),
    /// Build a .tar.gz package.
    Gzip(SharedAction<cli::compress::Opts>),
    /// List paths used by kick.
    Info(SharedOptions),
    /// Configure github authentication.
    ///
    /// This can be configured by setting the `GITHUB_TOKEN` environment
    /// variable, passing `--github-token` with the token to any command,
    /// writing the token to your .github-token configuration file using this
    /// command.
    Login(SharedAction<cli::login::Opts>),
    /// Build an .msi package (using wix).
    Msi(SharedAction<cli::msi::Opts>),
    /// Find the minimum supported rust version.
    Msrv(SharedAction<cli::msrv::Opts>),
    /// Publish packages in reverse order of dependencies.
    Publish(SharedAction<cli::publish::Opts>),
    /// Build an .rpm package (builtin).
    Rpm(SharedAction<cli::rpm::Opts>),
    /// Run a custom command.
    Run(SharedAction<cli::run::Opts>),
    /// Manage sets.
    Set(SharedAction<cli::set::Opts>),
    /// Synchronize repositories.
    Sync(SharedAction<cli::sync::Opts>),
    /// Update Kick itself.
    Update(SharedOptions),
    /// Perform a repository aware cargo upgrade. In particular this prevents
    /// packages which have been denylisted from being upgraded.
    Upgrade(SharedAction<cli::upgrade::Opts>),
    /// Modify package versions.
    Version(SharedAction<cli::version::Opts>),
    /// Build a .zip package.
    Zip(SharedAction<cli::compress::Opts>),
}

impl Command {
    fn requires_token(&self) -> bool {
        matches!(self, Command::Github(..))
    }

    fn shared(&self) -> &SharedOptions {
        match self {
            Command::Changes(shared) => shared,
            Command::Check(c) => &c.shared,
            Command::Deb(c) => &c.shared,
            Command::Define(c) => &c.shared,
            Command::Github(c) => &c.shared,
            Command::GithubAction(c) => &c.shared,
            Command::Gzip(c) => &c.shared,
            Command::Info(shared) => shared,
            Command::Login(c) => &c.shared,
            Command::Msi(c) => &c.shared,
            Command::Msrv(c) => &c.shared,
            Command::Publish(c) => &c.shared,
            Command::Rpm(c) => &c.shared,
            Command::Run(c) => &c.shared,
            Command::Set(c) => &c.shared,
            Command::Sync(c) => &c.shared,
            Command::Update(shared) => shared,
            Command::Upgrade(c) => &c.shared,
            Command::Version(c) => &c.shared,
            Command::Zip(c) => &c.shared,
        }
    }

    fn repo(&self) -> Option<&RepoOptions> {
        match self {
            Command::Changes(..) => None,
            Command::Check(action) => Some(&action.repo),
            Command::Deb(c) => Some(&c.repo),
            Command::Define(c) => Some(&c.repo),
            Command::Github(c) => Some(&c.repo),
            Command::GithubAction(c) => Some(&c.repo),
            Command::Gzip(c) => Some(&c.repo),
            Command::Info(..) => None,
            Command::Login(..) => None,
            Command::Msi(c) => Some(&c.repo),
            Command::Msrv(c) => Some(&c.repo),
            Command::Publish(c) => Some(&c.repo),
            Command::Rpm(c) => Some(&c.repo),
            Command::Run(c) => Some(&c.repo),
            Command::Set(c) => Some(&c.repo),
            Command::Sync(c) => Some(&c.repo),
            Command::Update(..) => None,
            Command::Upgrade(c) => Some(&c.repo),
            Command::Version(c) => Some(&c.repo),
            Command::Zip(c) => Some(&c.repo),
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

#[derive(Default, Debug, Parser)]
struct SharedOptions {
    /// Specify custom root folder for project hierarchy.
    #[arg(long)]
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
    /// This can also be set through the GITHUB_TOKEN environment variable, or
    /// by setting a token with the login command.
    #[arg(long)]
    github_token: Option<SecretString>,
    /// List all found system tools.
    #[arg(long)]
    list_tools: bool,
    /// The number of operations to do in parallel if applicable.
    #[arg(long, default_value = PARALLELISM)]
    parallelism: usize,
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

#[derive(Default, Debug, Parser)]
struct RepoOptions {
    /// Force processing of all repos, even if the root is currently inside of
    /// an existing repo.
    #[arg(long)]
    all: bool,
    /// Only run the specified set of repos.
    #[arg(long = "path", short = 'p')]
    repos: Vec<String>,
    /// If we should fetch the latest updates from remotes before filtering.
    #[arg(long)]
    fetch: bool,
    /// Only run over repos which have declared that the same operating system
    /// is supported.
    #[arg(long)]
    supported_os: bool,
    /// Load sets of repositories to operate on. These can take the operators +
    /// to add, - to remove, and ^ to intersect.
    ///
    /// This allows for flexible expressions like @all - ignore or good - @dirty
    ///
    /// Sets prefixed with @ are special sets. @all refers to all repos. @dirty
    /// refers to dirty repos as detected by vcs. @outdated refers to repos that
    /// are out-of-date with remote. @cached that have cached changes.
    /// @unreleased refers to repos that point to a revision which does not have
    /// a remote tag.
    #[arg(long)]
    set: Vec<SetOperations>,
    /// Save remaining or failed repos to the specified set.
    ///
    /// In case an operation is cancelled, or for repos where the operation
    /// fails, this will cause the remaining repos to be saved to the set of the
    /// specified names.
    #[arg(long)]
    set_remaining: Vec<String>,
}

#[derive(Debug, Clone)]
enum Set {
    All,
    Dirty,
    Outdated,
    Cached,
    Unreleased,
    Named(String),
}

#[derive(Default, Debug, Clone)]
struct SetOperations {
    sets: Vec<(SetOp, Set)>,
}

impl FromStr for SetOperations {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        struct Parser<I>(I);

        impl Parser<Chars<'_>> {
            fn peek(&self) -> char {
                self.0.clone().next().unwrap_or('\0')
            }

            fn skip(&mut self, n: usize) {
                for _ in 0..n {
                    _ = self.0.next();
                }
            }

            fn next(&mut self) -> char {
                self.0.next().unwrap_or('\0')
            }

            fn skip_whitespace(&mut self) {
                while self.peek().is_whitespace() {
                    self.skip(1);
                }
            }

            fn remaining(&self) -> &str {
                self.0.as_str()
            }
        }

        let mut sets = Vec::new();

        let mut p = Parser(s.chars());

        // Buffer to collect identifiers.
        let mut id = String::new();
        // Only allow first set to not have an operator.
        let mut first = true;

        loop {
            p.skip_whitespace();

            let (n, op) = match p.peek() {
                '+' => (1, SetOp::Add),
                '-' => (1, SetOp::Sub),
                '^' => (1, SetOp::Difference),
                _ if first => (0, SetOp::Add),
                '\0' => break,
                c => {
                    return Err(anyhow!(
                        "expected a set operation like +, -, or ^ but found '{c}'",
                    ));
                }
            };

            first = false;

            p.skip(n);
            p.skip_whitespace();

            while matches!(p.peek(), '@' | 'a'..='z' | 'A'..='Z' | '0'..='9' | '_') {
                id.push(p.next());
            }

            if id.is_empty() {
                return Err(anyhow!(
                    "expected a non-empty set name containing a-z, A-Z, 0-9, or _ but found {:?}",
                    p.remaining()
                ));
            }

            let set = if let Some(special) = id.strip_prefix('@') {
                match special {
                    "all" => Set::All,
                    "dirty" => Set::Dirty,
                    "outdated" => Set::Outdated,
                    "cached" => Set::Cached,
                    "unreleased" => Set::Unreleased,
                    other => {
                        return Err(anyhow!(
                            "unknown special set name '@{}', expected '@all' or '@dirty'",
                            other
                        ));
                    }
                }
            } else {
                Set::Named(id.to_string())
            };

            sets.push((op, set));
            id.clear();
        }

        Ok(SetOperations { sets })
    }
}

impl SetOperations {
    fn is_empty(&self) -> bool {
        self.sets.is_empty()
    }
}

#[derive(Debug, Clone, Copy)]
enum SetOp {
    Add,
    Sub,
    Difference,
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
#[command(author, version = VERSION, max_term_width = 80)]
struct Opts {
    /// Action to perform. Defaults to `check`.
    #[command(subcommand)]
    action: Command,
}

#[tokio::main]
async fn main() -> Result<ExitCode> {
    let opts = Opts::parse();

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

    env.update_from_env();

    if env.github_tokens.is_empty() {
        if opts.action.requires_token() {
            tracing::error!("No github token found");
        } else {
            tracing::warn!(
                "No github token found, heavy rate limiting will apply if accessing the Github"
            )
        }

        tracing::info!("See `kick login --help` for more information on how to set this up");
    }

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

    let cx = ctxt::Ctxt {
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

    let mut with_repos = WithRepos::new(cx, shared.parallelism);

    match &opts.action {
        Command::Check(opts) => {
            cli::check::entry(&mut with_repos, &opts.action).await?;
        }
        Command::Info(..) => {
            println!("Os: {}", with_repos.cx.os);
            println!("Dist: {}", with_repos.cx.dist);
            println!("Root: {}", with_repos.cx.paths.root.display());

            if let Some(current) = with_repos.cx.paths.current {
                println!("Current: {current}");
            }

            if let Some(config) = with_repos.cx.paths.config {
                println!("Config: {}", config.display());
            }

            if let Some(cache) = with_repos.cx.paths.cache {
                println!("Cache: {}", cache.display());
            }

            return Ok(ExitCode::SUCCESS);
        }
        Command::Changes(..) => {
            cli::changes::entry(&with_repos.cx, &changes_path)?;
        }
        Command::Update(shared) => {
            cli::update::entry(&mut with_repos.cx, shared).await?;
            return Ok(ExitCode::SUCCESS);
        }
        Command::Login(opts) => {
            cli::login::entry(&mut with_repos.cx, &opts.action)?;
            return Ok(ExitCode::SUCCESS);
        }
        Command::Define(opts) => {
            cli::define::entry(&mut with_repos, &opts.action)?;
        }
        Command::Set(opts) => {
            cli::set::entry(&mut with_repos.cx, &opts.action)?;
        }
        Command::Run(opts) => {
            cli::run::entry(&mut with_repos, &opts.action)?;
        }
        Command::Msrv(opts) => {
            cli::msrv::entry(&mut with_repos, &opts.action)?;
        }
        Command::Version(opts) => {
            cli::version::entry(&mut with_repos, &opts.action)?;
        }
        Command::Publish(opts) => {
            cli::publish::entry(&mut with_repos, &opts.action)?;
        }
        Command::Upgrade(opts) => {
            cli::upgrade::entry(&mut with_repos, &opts.action)?;
        }
        Command::Msi(opts) => {
            cli::msi::entry(&mut with_repos, &opts.action)?;
        }
        Command::Rpm(opts) => {
            cli::rpm::entry(&mut with_repos, &opts.action)?;
        }
        Command::Deb(opts) => {
            cli::deb::entry(&mut with_repos, &opts.action)?;
        }
        Command::Zip(opts) => {
            cli::compress::entry(&mut with_repos, cli::compress::Kind::Zip, &opts.action)?;
        }
        Command::Gzip(opts) => {
            cli::compress::entry(&mut with_repos, cli::compress::Kind::Gzip, &opts.action)?;
        }
        Command::GithubAction(opts) => {
            cli::github_action::entry(&mut with_repos, &opts.action)?;
        }
        Command::Github(opts) => {
            cli::gh::entry(&mut with_repos, &opts.action).await?;
        }
        Command::Sync(opts) => {
            cli::sync::entry(&mut with_repos, &opts.action)?;
        }
    }

    let cx = with_repos.into_cx();

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

struct GitCache<'repo, 'a> {
    repos: &'repo [Repo],
    paths: Paths<'a>,
    system: &'a system::System,
    fetch: bool,
    dirty_init: bool,
    dirty: Vec<&'repo RelativePath>,
    cached_init: bool,
    cached: Vec<&'repo RelativePath>,
    outdated_init: bool,
    outdated: Vec<&'repo RelativePath>,
    unreleased_init: bool,
    unreleased: Vec<&'repo RelativePath>,
}

impl<'repo, 'a> GitCache<'repo, 'a> {
    fn new(
        repos: &'repo [Repo],
        paths: Paths<'a>,
        system: &'a system::System,
        fetch: bool,
    ) -> Self {
        Self {
            repos,
            paths,
            system,
            fetch,
            dirty_init: false,
            dirty: Vec::new(),
            cached_init: false,
            cached: Vec::new(),
            outdated_init: false,
            outdated: Vec::new(),
            unreleased_init: false,
            unreleased: Vec::new(),
        }
    }

    fn dirty_set(&mut self) -> Result<&[&'repo RelativePath]> {
        if !self.dirty_init {
            let git = self
                .system
                .git
                .first()
                .context("no working git command found")?;

            for repo in self.repos {
                let path = self.paths.to_path(repo.path());

                if git.is_dirty(&path)? {
                    self.dirty.push(repo.path());
                }
            }

            self.dirty_init = true;
        }

        Ok(&self.dirty)
    }

    fn cached_set(&mut self) -> Result<&[&'repo RelativePath]> {
        if !self.cached_init {
            let git = self
                .system
                .git
                .first()
                .context("no working git command found")?;

            for repo in self.repos {
                let path = self.paths.to_path(repo.path());

                if git.is_cached(&path)? {
                    self.cached.push(repo.path());
                }
            }

            self.cached_init = true;
        }

        Ok(&self.cached)
    }

    fn outdated_set(&mut self) -> Result<&[&'repo RelativePath]> {
        if !self.outdated_init {
            let git = self
                .system
                .git
                .first()
                .context("no working git command found")?;

            for repo in self.repos {
                let path = self.paths.to_path(repo.path());

                if git.is_outdated(&path, self.fetch)? {
                    self.outdated.push(repo.path());
                }
            }

            self.outdated_init = true;
        }

        Ok(&self.outdated)
    }

    fn unreleased_set(&mut self) -> Result<&[&'repo RelativePath]> {
        if !self.unreleased_init {
            let git = self
                .system
                .git
                .first()
                .context("no working git command found")?;

            for repo in self.repos {
                let path = self.paths.to_path(repo.path());

                let outcome = 'outcome: {
                    let Some(describe) = git.describe_tags(&path, self.fetch)? else {
                        tracing::trace!("No tags to describe");
                        break 'outcome true;
                    };

                    if describe.offset.is_none() {
                        tracing::trace!("No offset detected (tag: {})", describe.tag);
                        break 'outcome true;
                    }

                    false
                };

                if outcome {
                    self.unreleased.push(repo.path());
                }
            }

            self.unreleased_init = true;
        }

        Ok(&self.unreleased)
    }
}

enum LoadedPaths<'a, 'path> {
    Owned(&'a Vec<RelativePathBuf>),
    Borrowed(&'a [&'path RelativePath]),
}

impl LoadedPaths<'_, '_> {
    fn for_each(&self, mut f: impl FnMut(&RelativePath)) {
        match self {
            Self::Owned(v) => {
                for p in *v {
                    f(p);
                }
            }
            Self::Borrowed(v) => {
                for p in *v {
                    f(p);
                }
            }
        }
    }
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

    let mut git_cache = GitCache::new(repos, paths, system, repo_opts.fetch);
    let mut owned_work = Vec::new();
    let mut work = Vec::new();

    let set = if !repo_opts.set.iter().all(|s| s.is_empty()) {
        let mut set = HashSet::new();

        for (op, s) in repo_opts.set.iter().flat_map(|s| s.sets.iter()) {
            let work = match s {
                Set::All => {
                    work.clear();

                    for repo in repos {
                        work.push(repo.path());
                    }

                    LoadedPaths::Borrowed(&work)
                }
                Set::Dirty => LoadedPaths::Borrowed(git_cache.dirty_set()?),
                Set::Cached => LoadedPaths::Borrowed(git_cache.cached_set()?),
                Set::Outdated => LoadedPaths::Borrowed(git_cache.outdated_set()?),
                Set::Unreleased => LoadedPaths::Borrowed(git_cache.unreleased_set()?),
                Set::Named(id) => {
                    owned_work.clear();

                    if let Some(s) = sets.load(id)? {
                        owned_work.extend(s.into_iter());
                    }

                    LoadedPaths::Owned(&owned_work)
                }
            };

            let op: fn(&mut HashSet<_>, &RelativePath) = match op {
                SetOp::Add => |set, id| {
                    if !set.contains(id) {
                        set.insert(id.to_owned());
                    }
                },
                SetOp::Sub => |set, id| {
                    set.remove(id);
                },
                SetOp::Difference => |set, id| {
                    if set.contains(id) {
                        set.remove(id);
                    } else {
                        set.insert(id.to_owned());
                    }
                },
            };

            work.for_each(|id| {
                op(&mut set, id);
            });
        }

        Some(set)
    } else {
        None
    };

    let in_current_path = if !repo_opts.all && in_repo_path {
        paths.current
    } else {
        None
    };

    filter_repos(
        config,
        in_current_path,
        repo_opts,
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
    in_current_path: Option<&RelativePath>,
    repo_opts: &RepoOptions,
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
