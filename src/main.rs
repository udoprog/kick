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
//! <br>
//!
//! [my `projects` repo]: https://github.com/udoprog/projects
//!
//! ## Available commands
//!
//! For a complete list of options, make use of `--help`. But these are the
//! available commands.
//!
//! * `check` - Checks each repo (default action).
//! * `changes` - Apply staged changes which have previously been saved by
//!   `check` unless `--save` was specified.
//! * `for` - Run a custom command.
//! * `status` - Fetch the github build status.
//! * `msrv` - Find the minimum supported rust version through bisection.
//! * `version` - Update package versions.
//! * `publish` - Publish packages in reverse order of dependencies.
//! * `upgrade` - Perform repo-aware `cargo upgrade`.
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
//! ## Working with module sets
//!
//! Commands can produce sets under certain circumstances. Look out for the
//! switch named `--store-sets`.
//!
//! If this is set during a run, it will store sets of modules, such as the set
//! for which a command failed. This set can then later be re-used through the
//! `--set <id>` switch.
//!
//! For a list of available sets, you can simply list the `sets` folder:
//!
//! ```text
//! sets\bad
//! sets\bad.2023-04-14-050517
//! sets\bad.2023-04-14-050928
//! sets\bad.2023-04-14-051046
//! sets\good.2023-04-14-050517
//! sets\good.2023-04-14-050928
//! sets\good.2023-04-14-051046
//! ```
//!
//! > **Note** the three most recent versions of each set will be retained. If
//! > you want to save a set make you can either rename it from its dated file
//! > or make use of `--store-sets` while running a command. If there are no
//! > non-dated versions of a set the latest one is used.
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
//! ## Configuration
//!
//! Configuration for kick is stores in a `Kick.toml` file. Whenever you run the
//! command it will look recursively for the `Kick.toml` that is in the
//! shallowest possible filesystem location.
//!
//! Configuration is loaded in a hierarchy, and each option can be extended or
//! overriden on a per-repo basis. This is usually done through a
//! `[repos."<name>"]` section.
//!
//! ```toml
//! [repos."repos/OxidizeBot"]
//! crate = "oxidize"
//!
//! [repos."repos/OxidizeBot".upgrade]
//! exclude = [
//!    # We avoid touching this dependency since it has a complex set of version-dependent feature flags.
//!    "libsqlite3-sys"
//! ]
//! ```
//!
//! The equivalent would be to put the following inside of
//! `repos/OxidizeBot/Kick.toml`, but this is usually not desirable since you
//! might not want to contaminate the project folder with a random file nobody
//! knows what it is.
//!
//! ```toml
//! # repos/OxidizeBot/Kick.toml
//! crate = "oxidize"
//!
//! [upgrade]
//! exclude = [
//!    # We avoid touching this dependency since it has a complex set of version-dependent feature flags.
//!    "libsqlite3-sys"
//! ]
//! ```
//!
//! Any option defined in the following section can be used either as an
//! override, or as part of its own repo-specific configuration.
//!
//! <br>
//!
//! ## `crate`
//!
//! Overrides the detected crate name.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! [repos."repos/OxidizeBot"]
//! crate = "oxidize"
//! ```
//!
//! <br>
//!
//! ## `variables`
//!
//! Defines an arbitrary collection of extra variables that can be used in templates.
//!
//! These are overriden on a per-repo basis in the following ways:
//!
//! * Arrays are extended.
//! * Maps are extended, so conflicting keys are overriden.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! [variables]
//! github = "https://github.com"
//! docs_rs = "https://docs.rs"
//! docs_rs_image = "data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K"
//! colors = { github = "8da0cb", crates_io = "fc8d62", docs_rs = "66c2a5" }
//! badge_height = 20
//! ```
//!
//! <br>
//!
//! ## `job_name`
//!
//! Defines the name of the default workflow, this can be used to link to in
//! badges later and is also validated by the `ci` module.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! job_name = "CI"
//! ```
//!
//! <br>
//!
//! ## `workflow`
//!
//! Defines the default workflow template to use when a workflow is missing.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! workflow = "data/workflow.yml"
//! ```
//!
//! <br>
//!
//! ## `license`
//!
//! Defines the license for the current repo.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! license = "MIT/Apache-2.0"
//! ```
//!
//! <br>
//!
//! ## `authors`
//!
//! Defines a list of authors that should be present wherever appropriate, such
//! as in a `Cargo.toml`.
//!
//! Per-repo options will extend this list.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! authors = ["John-John Tedro <udoprog@tedro.se>"]
//! ```
//!
//! <br>
//!
//! ## `documentation`
//!
//! Defines the documentation link to use.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! documentation = "{{docs_rs}}/{{crate.name}}"
//! ```
//!
//! <br>
//!
//! ## `readme_badges`
//!
//! Defines a set of badges to use for readmes.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! readme_badges = ["+build"]
//! ```
//!
//! <br>
//!
//! ## `disabled`
//!
//! A list of kick modules to disable.
//!
//! ```toml
//! disabled = ["ci"]
//! ```
//!
//! <br>
//!
//! ## badges
//!
//! Defines a list of *available* badges to use, with the following options:
//!
//! * `id` the identifier of the badge, used in `lib_badges` or `readme_badges`
//!   (see below).
//! * `alt` the alt text of the badge.
//! * `src` the source image of the badge.
//! * `href` the link of the badge.
//! * `height` the height of the badge.
//! * `enabled` whether or not the badge should be enabled by default. This can
//!   be overrided in `lib_badges` and `readme_badges` (see below).
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! [[badges]]
//! alt = "github"
//! src = "https://img.shields.io/badge/github-{{dash_escape crate.repo}}-{{colors.github}}?style=for-the-badge&logo=github"
//! href = "{{github}}/{{crate.repo}}"
//! height = "{{badge_height}}"
//!
//! [[badges]]
//! id = "crates.io"
//! alt = "crates.io"
//! src = "https://img.shields.io/crates/v/{{crate.name}}.svg?style=for-the-badge&color={{colors.crates_io}}&logo=rust"
//! href = "https://crates.io/crates/{{crate.name}}"
//! height = "{{badge_height}}"
//!
//! [[badges]]
//! id = "docs.rs"
//! alt = "docs.rs"
//! src = "https://img.shields.io/badge/docs.rs-{{dash_escape crate.name}}-{{colors.docs_rs}}?style=for-the-badge&logoColor=white&logo={{docs_rs_image}}"
//! href = "{{docs_rs}}/{{crate.name}}"
//! height = "{{badge_height}}"
//!
//! [[badges]]
//! id = "build"
//! alt = "build status"
//! src = "https://img.shields.io/github/actions/workflow/status/{{crate.repo}}/ci.yml?branch=main&style=for-the-badge"
//! href = "{{github}}/{{crate.repo}}/actions?query=branch%3Amain"
//! height = "{{badge_height}}"
//! enabled = false
//!
//! [[badges]]
//! id = "discord"
//! alt = "chat on discord"
//! src = "https://img.shields.io/discord/558644981137670144.svg?logo=discord&style=flat-square"
//! href = "https://discord.gg/v5AeNkT"
//! height = "{{badge_height}}"
//! enabled = false
//! ```
//!
//! <br>
//!
//! ## `lib_badges` and `readme_badges`
//!
//! Defines set of badges to use, either for the `lib` or `readme` file (see
//! below).
//!
//! Badges are either included by prefixing them with a `+`, or excluded by
//! prefixing them with a `-`. This overrides their default option (see above).
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! lib_badges = ["-docs.rs", "-crates.io", "+discord"]
//! readme_badges = ["-docs.rs", "-crates.io", "+discord"]
//! ```
//!
//! <br>
//!
//! ## `lib` and `readme`
//!
//! Path to templates:
//! * `lib` generates the documentation header for `main.rs` or `lib.rs` entrypoints.
//! * `readme` generated the `README.md` file.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! [repos."repos/rune"]
//! lib = "data/rune.lib.md"
//! readme = "data/rune.readme.md"
//! ```
//!
//! The following variables are available for expansion:
//!
//! * `body` the rest of the comment, which does not include the generated
//!   header.
//! * `badges` a list of badges, each `{html: string, markdown: string}`.
//! * `header_marker` a header marker that can be used to indicate the end of a
//!   generated documentation header, this will only be set if there's a
//!   trailing documentation section in use (optional).
//! * `job_name` the name of the CI job, as specified in the `job_name` setting.
//! * `rust_versions.rustc` the rust version detected by running `rustc
//!   --version` (optional).
//! * `rust_versions.edition_2018` the rust version that corresponds to the 2018
//!   edition.
//! * `rust_versions.edition_2021` the rust version that corresponds to the 2021
//!   edition.
//! * `crate.name` the name of the crate.
//! * `crate.repo.owner` the owner of the repository, as in `<owner>/<name>`
//!   (optional).
//! * `crate.repo.name` the name of the repository, as in `<owner>/<name>`
//!   (optional).
//! * `crate.description` the repo-specific description from its primary
//!   `Cargo.toml` manifest (optional).
//! * `crate.rust_version` the rust-version read from its primary `Cargo.toml`
//!   manifest (optional).
//!
//! The following is an example `lib` template stored in `data/rune.lib.md`,
//! note that the optional `header_marker` is used here in case there is a
//! trailing comment in use.
//!
//! ```md
//! <img alt="rune logo" src="https://raw.githubusercontent.com/rune-rs/rune/main/assets/icon.png" />
//! <br>
//! {{#each badges}}
//! {{literal this.html}}
//! {{/each}}
//! <br>
//! {{#if crate.rust_version}}
//! Minimum support: Rust <b>{{crate.rust_version}}+</b>.
//! <br>
//! {{/if}}
//! <br>
//! <a href="https://rune-rs.github.io"><b>Visit the site 🌐</b></a>
//! &mdash;
//! <a href="https://rune-rs.github.io/book/"><b>Read the book 📖</b></a>
//! <br>
//! <br>
//!
//! {{crate.description}}
//! {{#if body}}
//!
//! <br>
//!
//! {{literal header_marker~}}
//! {{literal body}}
//! {{/if}}
//! ```
//!
//! The following is an example `readme` template stored in `data/rune.readme.md`:
//!
//! ```md
//! <img alt="rune logo" src="https://raw.githubusercontent.com/rune-rs/rune/main/assets/icon.png" />
//! <br>
//! <a href="https://rune-rs.github.io"><b>Visit the site 🌐</b></a>
//! &mdash;
//! <a href="https://rune-rs.github.io/book/"><b>Read the book 📖</b></a>
//!
//! # {{crate.name}}
//!
//! {{#each badges}}
//! {{literal this.html}}
//! {{/each}}
//! <br>
//! <br>
//!
//! {{crate.description}}
//! {{#if body}}
//!
//! <br>
//!
//! {{literal body}}
//! {{/if}}
//! ```
//!
//! <br>
//!
//! ## `[[version]]`
//!
//! Defines a list of files for which we match a regular expression for version replacements.
//!
//! Available keys are:
//! * `[[version]].paths` - list of patterns to match when performing a version
//!   replacement.
//! * `[[version]].pattern` - the regular expression which performs the
//!   replacement. Use the `?P<version>` group name to define what is being
//!   replaced.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! [[version]]
//! paths = ["src/**/*.rs"]
//! # replaces versions in module level comments that looks likle [dependencies] declarations
//! pattern = "//!\\s+[a-z-]+\\s*=\\s*.+(?P<version>[0-9]+\\.[0-9]+\\.[0-9]+).+"
//! ```
//!
//! <br>
//!
//! ## Modules
//!
//! Modules provide optional functionality that can be disabled on a per-repo
//! basis through the `disabled` option above.
//!
//! <br>
//!
//! ### `ci` module
//!
//! This looks for the github workflow configuration matching `{{job_name}}` and
//! performs some basic validation over them, such as:
//!
//! * Making sure there is a build for your `rust-version` (if defined in
//!   `Cargo.toml`).
//! * Updates `rust-version` if it mismatches with the build.
//! * Rejects outdates actions, such as `actions-rs/cargo` in favor of simple
//!   `run` commands (exact list can't be configured yet).
//!
//! To disable, specify:
//!
//! ```toml
//! disabled = ["ci"]
//! ```
//!
//! <br>
//!
//! ### `readme` module
//!
//! Generates a `README.md` based on what's in the module-level comment of your
//! primary crate's entrypoint. See the `readme` template above.
//!
//! To disable, specify:
//!
//! ```toml
//! disabled = ["readme"]
//! ```

#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]

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
mod module_sets;
mod process;
mod rust_version;
mod templates;
mod urls;
mod workspace;

use std::cell::RefCell;
use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::path::{Component, Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use changes::Change;
use clap::{Args, FromArgMatches, Parser, Subcommand};

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
    /// Checks each repo (default action).
    Check(SharedAction<cli::check::Opts>),
    /// Apply staged changes which have previously been saved by `check` unless
    /// `--save` was specified.
    Changes(SharedOptions),
    /// Run a custom command for each repo.
    For(SharedAction<cli::foreach::Opts>),
    /// Fetch github actions build status for each module.
    Status(SharedAction<cli::status::Opts>),
    /// Find the minimum supported rust version for each module.
    Msrv(SharedAction<cli::msrv::Opts>),
    /// Update package version.
    Version(SharedAction<cli::version::Opts>),
    /// Publish packages in reverse order of dependencies.
    Publish(SharedAction<cli::publish::Opts>),
    /// Perform repo-aware `cargo upgrade`.
    Upgrade(SharedAction<cli::upgrade::Opts>),
}

impl Action {
    fn shared(&self) -> &SharedOptions {
        match self {
            Action::Check(action) => &action.shared,
            Action::For(action) => &action.shared,
            Action::Status(action) => &action.shared,
            Action::Msrv(action) => &action.shared,
            Action::Version(action) => &action.shared,
            Action::Publish(action) => &action.shared,
            Action::Upgrade(action) => &action.shared,
            Action::Changes(shared) => shared,
        }
    }

    fn module(&self) -> Option<&ModuleOptions> {
        match self {
            Action::Check(action) => Some(&action.module),
            Action::For(action) => Some(&action.module),
            Action::Status(action) => Some(&action.module),
            Action::Msrv(action) => Some(&action.module),
            Action::Version(action) => Some(&action.module),
            Action::Publish(action) => Some(&action.module),
            Action::Upgrade(action) => Some(&action.module),
            Action::Changes(..) => None,
        }
    }
}

impl Default for Action {
    fn default() -> Self {
        Self::Check(SharedAction {
            shared: SharedOptions::default(),
            module: ModuleOptions::default(),
            action: cli::check::Opts::default(),
        })
    }
}

#[derive(Default, Parser)]
struct SharedOptions {
    /// Specify custom root folder for project hierarchy.
    #[arg(long, name = "path")]
    root: Option<PathBuf>,
    /// Save any proposed or loaded changes.
    #[arg(long)]
    save: bool,
}

#[derive(Default, Parser)]
struct ModuleOptions {
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
    /// Load sets with the given ids.
    #[arg(long)]
    set: Vec<String>,
    /// Subtract sets with the given ids. This will remove any items in the
    /// loaded set (as specified by one or more `--set <id>`) that exist in the
    /// specified sets.
    #[arg(long)]
    sub_set: Vec<String>,
}

impl ModuleOptions {
    fn needs_git(&self) -> bool {
        self.dirty || self.cached || self.cached_only || self.unreleased
    }
}

#[derive(Parser)]
struct SharedAction<A>
where
    A: FromArgMatches + Args,
{
    #[command(flatten)]
    shared: SharedOptions,
    #[command(flatten)]
    module: ModuleOptions,
    #[command(flatten)]
    action: A,
}

#[derive(Default, Parser)]
#[command(author, version, about, long_about = None)]
#[command(propagate_version = true)]
struct Opts {
    /// Action to perform. Defaults to `check`.
    #[command(subcommand, name = "action")]
    action: Option<Action>,
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

    let action = opts.action.unwrap_or_default();
    let shared = action.shared();
    let module = action.module();

    let current_dir = match &shared.root {
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

    let mut sets = module_sets::ModuleSets::new(root.join("sets"))?;

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

    if let Some(module) = module {
        let current_path = if !module.all && modules.iter().any(|m| m.path() == current_path) {
            Some(current_path.as_ref())
        } else {
            None
        };

        let mut filters = Vec::new();

        for module in &module.modules {
            filters.push(Fragment::parse(module));
        }

        let set = match &module.set[..] {
            [] => None,
            ids => {
                let mut set = HashSet::new();

                for id in ids {
                    if let Some(s) = sets.load(id)? {
                        set.extend(s.iter().map(RelativePath::to_owned));
                    }
                }

                for id in &module.sub_set {
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
            module,
            git.as_ref(),
            &modules,
            &filters,
            current_path,
            set.as_ref(),
        )?;
    }

    let changes_path = root.join("changes.gz");

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

    match &action {
        Action::Check(opts) => {
            cli::check::entry(&cx, &opts.action).await?;
        }
        Action::For(opts) => {
            cli::foreach::entry(&mut cx, &opts.action)?;
        }
        Action::Status(opts) => {
            cli::status::entry(&cx, &opts.action).await?;
        }
        Action::Msrv(opts) => {
            cli::msrv::entry(&mut cx, &opts.action)?;
        }
        Action::Version(opts) => {
            cli::version::entry(&cx, &opts.action)?;
        }
        Action::Publish(opts) => {
            cli::publish::entry(&cx, &opts.action)?;
        }
        Action::Upgrade(opts) => {
            cli::upgrade::entry(&mut cx, &opts.action)?;
        }
        Action::Changes(shared) => {
            let changes = load_changes(&changes_path)
                .with_context(|| anyhow!("{}", changes_path.display()))?;

            let Some(changes) = changes else {
                tracing::info!("No changes found: {}", changes_path.display());
                return Ok(());
            };

            if !shared.save {
                tracing::warn!("Not writing changes since `--save` was not specified");
            }

            for change in changes {
                crate::changes::apply(&cx, &change, shared.save)?;
            }

            if shared.save {
                tracing::info!("Removing {}", changes_path.display());
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
        crate::changes::apply(&cx, change, shared.save)?;
    }

    if cx.can_save() && !shared.save {
        tracing::warn!("Not writing changes since `--save` was not specified");
        tracing::info!(
            "Writing commit to {}, use `kick changes` to review it later",
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
    opts: &ModuleOptions,
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
            let module_path = module.path().to_path(root);

            let cached = git.is_cached(&module_path)?;
            let dirty = git.is_dirty(&module_path)?;

            let span =
                tracing::trace_span!("git", ?cached, ?dirty, module = module.path().to_string());
            let _enter = span.enter();

            if opts.dirty && !dirty {
                tracing::trace!("Directory is not dirty");
                module.set_disabled(true);
            }

            if opts.cached && !cached {
                tracing::trace!("Directory has no cached changes");
                module.set_disabled(true);
            }

            if opts.cached_only && (!cached || dirty) {
                tracing::trace!("Directory has no cached changes");
                module.set_disabled(true);
            }

            if opts.unreleased {
                if let Some((tag, offset)) = git.describe_tags(&module_path)? {
                    if offset.is_none() {
                        tracing::trace!("No offset detected (tag: {tag})");
                        module.set_disabled(true);
                    }
                } else {
                    tracing::trace!("No tags to describe");
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
