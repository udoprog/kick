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
//! * [Deploying over ssh](#deploying-over-ssh)
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
//! Repos are only ever declared in `Kick.toml`. Git submodules and the
//! `.gitmodules` file are ignored, so a submodule only becomes a repo if it is
//! declared like above. Without any declared repos, kick acts on the project
//! itself.
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
//! To run over only some of the repos, pass `-p`/`--path`. A path to an existing
//! directory, relative to the current directory, absolute or starting with `~/`,
//! selects the repo containing it no matter how it is spelled, so `-p .` inside of
//! a repo or `-p ~/repos/rune/src` both work. Anything else is matched against the
//! repo paths relative to the root, like `-p repos/rune` or `-p 'repos/*'`. Inside
//! of a repo kick only runs over that repo, pass `--all` to run over every repo.
//!
//! Repos can also be selected by name with `-n`/`--repo-name`, like `-n rune`. A
//! repo's name is its `alias` if configured, else its `name`, else the last
//! component of its directory. A name which matches no repo or more than one is an
//! error, and `-n` combines with `-p` to select the repos matched by either.
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
//! To see which configuration kick loaded and which repos it would act on from
//! the current directory, run `kick inspect`. It lists every `Kick.toml` which
//! was looked for in load order (with the keys and `[repo."<path>"]` sections
//! each one sets, or that it is missing), any errors loading them, every repo
//! and why the selection includes or excludes it, and the effective `[build]`,
//! `[install]` and `[deploy]` configuration of the selected repos, including the
//! profile `kick deploy` would pick and the absolute paths of files and unit
//! templates. It takes the same selection options as other commands, such as
//! `--all`, `-p` and `--set`, `--to <profile>` to report on a specific profile,
//! and `--json` for scripting. It exits with an error if the configuration or the
//! selection would make other commands refuse to run.
//!
//! <br>
//!
//! ### Global configuration
//!
//! Kick also loads a user-global `Kick.toml` from its user configuration
//! directory, which is `~/.config/kick/Kick.toml` on Linux (`kick info` prints
//! it). It has the same schema as a project `Kick.toml`.
//!
//! The global configuration is loaded after every project `Kick.toml`, but it is
//! the least specific layer: anything a project or repo `Kick.toml` sets takes
//! precedence over it, so it's a good place for user defaults. Paths in it, such
//! as `[repo."<path>"]` keys and template paths, may be absolute, start with
//! `~/`, or be relative in which case they are resolved against the directory
//! holding the global `Kick.toml`, so `template = "data/ci.yml"` in
//! `~/.config/kick/Kick.toml` refers to `~/.config/kick/data/ci.yml`.
//!
//! Path-valued settings in any `Kick.toml` interpolate a leading `~` and
//! environment variables as `$VAR`, `${VAR}` or `${VAR:-default}`, and `$$` is a
//! literal `$`. Referencing an unset variable without a `:-` fallback is an error,
//! it never expands to an empty string. See [config.md](config.md#paths-and-environment-variables)
//! for details.
//!
//! When kick runs *outside* of a project, that is when no `Kick.toml` or git
//! checkout is found in the current directory or any of its parents, it acts on
//! the repos declared in the global configuration instead. This lets you keep
//! your list of projects per-user rather than in a `Kick.toml` hierarchy. Inside
//! of a project the `[repo]` sections of the global configuration are ignored,
//! unless `--all` is passed, a `-p`/`--path` names a directory outside of the
//! project which is inside of one of them, or a `-n`/`--repo-name` matches no repo
//! in the project but one of them, in which case kick acts on the global repos
//! instead.
//!
//! ```toml
//! # ~/.config/kick/Kick.toml
//! [repo."~/projects/OxidizeBot"]
//! url = "https://github.com/udoprog/OxidizeBot"
//! ```
//!
//! Projects in the global configuration can be managed with `kick project`, which
//! edits it in place while preserving formatting and comments:
//!
//! ```sh
//! kick project add                 # register the git checkout of the current directory
//! kick project add ~/src/foo --url https://github.com/udoprog/foo
//! kick project list
//! kick project remove ~/src/foo
//! kick project edit ~/src/foo      # edit its settings in $VISUAL or $EDITOR
//! ```
//!
//! `kick project add` stores directories inside of your home directory with a `~/`
//! prefix, and detects the url from the `origin` remote unless `--url` is given.
//! `kick project edit` (which defaults to the git checkout of the current
//! directory) writes the `[repo."<key>"]` table of a project to a temporary file
//! and opens it in `$VISUAL`, `$EDITOR` or `vi` (`notepad` on Windows). Once the
//! editor exits the edited settings are validated and merged back into the global
//! configuration; if they are invalid nothing is changed and the temporary file is
//! kept so the edits are not lost.
//! Repo sets and staged changes made while acting on global repos are stored in
//! the user configuration directory.
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
//! Commands run by `kick run` have their stdin closed, so a tool which prompts
//! for input sees end of input instead of stalling the run. Pass `--interactive`
//! to let them read from the terminal.
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
//! There are a number of special sets you can use:
//! * `@all` - All repositories.
//! * `@dirty` - Repositories with uncommitted changes.
//! * `@outdated` - Repositories that are out-of-date with their remote.
//! * `@cached` - Repositories with cached changes staged.
//! * `@unreleased` - Repositories have checked out revisions which do not have
//!   a corresponding remote tag.
//!
//! The `--set` parameter also supports simple set operations such as adding
//! `+`, subtracting `-`, and difference `^`, and intersecting `&` between sets.
//! Such as selecting all repos which *are not* outdated with `@all -
//! @outdated`.
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
//! ## Deploying over ssh
//!
//! The `deploy` action is a lightweight way of getting a project onto a server.
//!
//! It builds the project, sends the binary over `ssh`, installs it, and makes sure
//! that a systemd unit is installed and running. The defaults are picked so
//! that this needs no configuration at all:
//!
//! ```sh
//! kick deploy --host moore docular
//! ```
//!
//! That builds with `cargo build --release`, installs `target/release/docular` as
//! `/usr/local/bin/docular`, and installs and starts a `docular.service`. Anything
//! the build needs can be passed along:
//!
//! ```sh
//! kick deploy --host moore docular --pre-build "trunk build --release" --features bundle
//! ```
//!
//! The same things go in the configuration once they stop fitting on a command
//! line, or when they belong to the project rather than to one deployment. How the
//! project is built goes in `[build]`, which [`kick install`][install-config]
//! shares to build and install the binary into `~/.cargo/bin`, and where it goes in
//! `[deploy]`:
//!
//! ```toml
//! [build]
//! pre_build = ["trunk build --release"]
//! features = ["bundle"]
//!
//! [deploy]
//! host = "integration@moore"
//!
//! [deploy.systemd]
//! args = ["--bind", "0.0.0.0:3004"]
//! user = "track"
//! group = "track"
//! ```
//!
//! The host can also be left out of the configuration, typically in a
//! [profile][deploy-profiles] which is meant for any machine, in which case it is
//! given when deploying:
//!
//! ```sh
//! kick deploy --to remote --host moore
//! ```
//!
//! Either way `kick deploy` runs the build, and over a single `ssh` connection
//! uploads the binary, stops the service, installs the binary into `/usr/local/bin`,
//! installs the unit into `/etc/systemd/system`, and starts the service again. The
//! unit is compared against the installed one first, so systemd is only reloaded
//! when it actually changed.
//!
//! The unit comes from a built-in template which is filled in from the variables
//! in `[deploy.systemd]`, so there is nothing to write to get a working service. When you need something it doesn't cover, point `systemd` at a unit file
//! of your own instead — they are [minijinja] templates, so
//! `ExecStart={{ exec }} {{ args | join(" ") }}` picks up the remote path of the
//! binary along with anything else you have defined.
//!
//! [minijinja]: https://docs.rs/minijinja
//!
//! Before any of this happens the host is logged into to make sure that we can
//! actually reach it as the user we expect, that the commands being used are
//! available, and that `sudo` doesn't need a password. Finding that out after a
//! lengthy build is not very helpful. The connection is kept open while building
//! and the deployment reuses it, so the host is only logged into once.
//!
//! That last one matters because the install script is handed to `ssh` as a single
//! non-interactive command, which runs under `/bin/sh` whatever the login shell
//! of the user is. There is no terminal for a password prompt to be
//! answered on, so every privileged command uses `sudo -n` and the deploying user is
//! expected to have a `NOPASSWD` entry in sudoers.
//!
//! Since nothing is installed remotely unless the upload succeeded, a failed
//! deployment leaves the currently running service alone.
//!
//! To see exactly what would happen without changing anything, use `kick deploy
//! --dry-run`. For all available options, see the [deployment
//! documentation][deploy-config].
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
//! [deploy-config]: https://github.com/udoprog/kick/blob/main/config/deploy.md
//! [deploy-profiles]: https://github.com/udoprog/kick/blob/main/config/deploy.md#profiles
//! [install-config]: https://github.com/udoprog/kick/blob/main/config/install.md
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
mod git_cache;
mod gix;
mod glob;
mod global;
mod interpolate;
mod keys;
mod model;
mod musli;
mod octokit;
mod packaging;
mod process;
mod release;
mod repo_sets;
mod restore;
mod rstr;
mod shell;
mod system;
mod systemd;
mod templates;
mod urls;
mod utils;
mod version_groups;
mod wix;
mod workflows;
mod workspace;
mod worktree;

use std::cell::RefCell;
use std::collections::{BTreeSet, HashSet};
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::str::{Chars, FromStr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser, Subcommand};

use config::{Config, Distribution, Os};
use directories::{BaseDirs, ProjectDirs};
use env::SecretString;
use model::State;
use relative_path::{RelativePath, RelativePathBuf};
use repo_sets::RepoSet;
use termcolor::{ColorChoice, StandardStream};

use crate::cli::WithRepos;
use crate::ctxt::Paths;
use crate::env::Env;
use crate::git_cache::GitCache;
use crate::glob::Fragment;
use crate::model::Repo;
use crate::model::{RepoSource, load_from_git};

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
    /// Deploy a project over ssh.
    Deploy(SharedAction<cli::deploy::Opts>),
    /// Interact with the github API parameterized over repositories.
    #[command(name = "gh")]
    Github(SharedAction<cli::gh::Opts>),
    /// Run a github action.
    GithubAction(SharedAction<cli::github_action::Opts>),
    /// Build a .tar.gz package.
    Gzip(SharedAction<cli::compress::Opts>),
    /// List paths used by kick.
    Info(SharedOptions),
    /// Show the configuration kick loaded and the repos it would act on.
    ///
    /// This takes the same repo selection options as other commands, such as
    /// `--all`, `-p` and `--set`, and reports every configuration file which
    /// was looked for in load order, load errors, every repo and whether the
    /// selection includes it, and the effective configuration of selected
    /// repos.
    Inspect(SharedAction<cli::inspect::Opts>),
    /// Install a project locally by running its install commands.
    Install(SharedAction<cli::install::Opts>),
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
    /// Manage the projects registered in the global configuration.
    ///
    /// The global configuration is a `Kick.toml` in kick's user configuration
    /// directory (see `kick info`). When kick runs outside of a project it
    /// acts on the projects registered in it.
    Project(ProjectAction),
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
            Command::Deploy(c) => &c.shared,
            Command::Github(c) => &c.shared,
            Command::GithubAction(c) => &c.shared,
            Command::Gzip(c) => &c.shared,
            Command::Info(shared) => shared,
            Command::Inspect(c) => &c.shared,
            Command::Install(c) => &c.shared,
            Command::Login(c) => &c.shared,
            Command::Msi(c) => &c.shared,
            Command::Msrv(c) => &c.shared,
            Command::Project(c) => &c.shared,
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
            Command::Deploy(c) => Some(&c.repo),
            Command::Github(c) => Some(&c.repo),
            Command::GithubAction(c) => Some(&c.repo),
            Command::Gzip(c) => Some(&c.repo),
            Command::Info(..) => None,
            Command::Inspect(c) => Some(&c.repo),
            Command::Install(c) => Some(&c.repo),
            Command::Login(..) => None,
            Command::Msi(c) => Some(&c.repo),
            Command::Msrv(c) => Some(&c.repo),
            Command::Project(..) => None,
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
        !matches!(self, Command::Login(..) | Command::Project(..))
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
    /// Force processing of all repos, even if the current directory is inside
    /// of one of them.
    ///
    /// Inside of a project, this runs over the repos declared in the global
    /// configuration instead if it declares any.
    #[arg(long)]
    all: bool,
    /// Only run over the specified repos.
    ///
    /// A path to an existing directory (relative to the current directory,
    /// absolute, or starting with `~/`) selects the repo containing it.
    /// Anything else is matched against the repo paths relative to the root,
    /// and may use `*` wildcards.
    ///
    /// Inside of a project, a directory outside of it which belongs to a repo
    /// declared in the global configuration selects among the global repos
    /// instead.
    #[arg(long = "path", short = 'p', value_name = "PATH")]
    repos: Vec<String>,
    /// Only run over the repos with the specified names.
    ///
    /// A repo's name is its `alias` if configured, else its `name`, else the
    /// last component of its directory. A name which matches no repo, or more
    /// than one, is an error. Combines with `-p`, selecting the repos matched
    /// by either.
    ///
    /// Inside of a project, a name which matches no repo in it but one declared
    /// in the global configuration selects among the global repos instead.
    #[arg(long = "repo-name", short = 'n', value_name = "NAME")]
    names: Vec<String>,
    /// If we should fetch the latest updates from remotes before filtering.
    #[arg(long)]
    fetch: bool,
    /// Only run over repos which have declared that the same operating system
    /// is supported.
    #[arg(long)]
    supported_os: bool,
    /// Load sets of repositories to operate on. These can take the operators +
    /// to add, - to remove, ^ to get the difference, and & to intersect.
    ///
    /// This allows for flexible expressions like @all - ignore or good - @dirty
    ///
    /// Sets prefixed with @ are special sets. @all refers to all repos. @dirty
    /// refers to dirty repos as detected by vcs. @outdated refers to repos that
    /// are out-of-date with remote. @cached that have cached changes.
    /// @unreleased refers to repos that point to a revision which does not have
    /// a remote tag.
    ///
    ///
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
                '+' | '|' => (1, SetOp::Add),
                '-' => (1, SetOp::Sub),
                '^' => (1, SetOp::Difference),
                '&' => (1, SetOp::Intersection),
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

impl fmt::Display for SetOperations {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, (op, set)) in self.sets.iter().enumerate() {
            match (index, op) {
                (0, SetOp::Add) => {}
                (_, SetOp::Add) => f.write_str(" + ")?,
                (_, SetOp::Sub) => f.write_str(" - ")?,
                (_, SetOp::Difference) => f.write_str(" ^ ")?,
                (_, SetOp::Intersection) => f.write_str(" & ")?,
            }

            match set {
                Set::All => f.write_str("@all")?,
                Set::Dirty => f.write_str("@dirty")?,
                Set::Outdated => f.write_str("@outdated")?,
                Set::Cached => f.write_str("@cached")?,
                Set::Unreleased => f.write_str("@unreleased")?,
                Set::Named(name) => f.write_str(name)?,
            }
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Copy)]
enum SetOp {
    Add,
    Sub,
    Difference,
    Intersection,
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

#[derive(Parser)]
#[command(version = None)]
struct ProjectAction {
    #[command(flatten)]
    action: cli::project::Opts,
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

    let project_dirs = ProjectDirs::from("se", "tedro", "kick");
    let home = BaseDirs::new().map(|d| d.home_dir().to_owned());
    let global_path = project_dirs
        .as_ref()
        .map(|p| p.config_dir().join(KICK_TOML));

    if let Command::Project(project) = &opts.action {
        let (Some(home), Some(global_path)) = (&home, &global_path) else {
            bail!("Could not determine the home and user configuration directories");
        };

        let current_dir = std::env::current_dir().context("Getting current directory")?;
        let system = system::detect()?;

        let cx = cli::project::Cx {
            config: global_path,
            home,
            current_dir: &current_dir,
            git: system.git.first(),
        };

        cli::project::entry(&cx, &project.action)?;
        return Ok(ExitCode::SUCCESS);
    }

    let current_dir = std::env::current_dir().context("Getting current directory")?;

    let (root, current_path, global_repos) = match &shared.root {
        Some(root) => {
            let root = root.canonicalize()?;
            let current = std::env::current_dir()?.canonicalize()?;

            let current_path = if let Ok(prefix) = current.strip_prefix(&root) {
                Some(RelativePathBuf::from_path(prefix)?)
            } else {
                None
            };

            (root.to_owned(), current_path, false)
        }
        None => {
            let found = find_root(&current_dir, home.as_deref(), global_path.as_deref());

            match (&home, &global_path) {
                (Some(home), Some(global_path))
                    if !found.2
                        && selects_global_repos(
                            &current_dir,
                            &found.0,
                            repo_opts,
                            home,
                            global_path,
                        ) =>
                {
                    tracing::trace!(
                        "Using repos from global configuration {}",
                        global_path.display()
                    );
                    (home.clone(), None, true)
                }
                _ => found,
            }
        }
    };

    let global = match (&home, &global_path) {
        (Some(home), Some(path)) => Some(global::Global {
            path: path.clone(),
            home: home.clone(),
            provides_repos: global_repos,
        }),
        _ => None,
    };

    // NB: State written by kick is stored in the root of the project, or in
    // the user configuration directory when acting on global repos.
    let state_dir = match project_dirs.as_ref() {
        Some(dirs) if global_repos => dirs.config_dir().to_owned(),
        _ => root.clone(),
    };

    let paths = Paths {
        root: &root,
        current: current_path.as_deref(),
        config: project_dirs.as_ref().map(|p| p.config_dir()),
        cache: project_dirs.as_ref().map(|p| p.cache_dir()),
        redirect: None,
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
        .chain((!global_repos).then(|| root.join(GITHUB_TOKEN)))
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
    let defaults = config::defaults();

    // NB: `inspect` reports configuration errors instead of failing on them.
    let inspecting = matches!(opts.action, Command::Inspect(..));
    let mut report = cli::inspect::Collected::default();

    let config = load_config(
        paths,
        global.as_ref(),
        &templating,
        &defaults,
        inspecting,
        &mut report,
    )?;

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

    let (repos, from_group) = match collect_repos(&config, &root, system.git.first()) {
        Ok(collected) => collected,
        Err(error) if inspecting => {
            report.selection_error = Some(format!("{error:#}"));
            (Vec::new(), false)
        }
        Err(error) => return Err(error),
    };

    let mut sets = repo_sets::RepoSets::new(state_dir.join("sets"))?;

    // When running from a git worktree nested inside of a registered repo, use
    // the worktree as that repo's working directory.
    let worktree = match nested_worktree(paths, repo_opts, &repos, from_group) {
        Ok(worktree) => worktree,
        Err(error) if inspecting => {
            report.selection_error = Some(error.to_string());
            None
        }
        Err(error) => return Err(error),
    };

    let (paths, config) = match &worktree {
        Some((repo, checkout)) => {
            let paths = Paths {
                redirect: Some(ctxt::Redirect {
                    repo,
                    worktree: checkout,
                }),
                ..paths
            };

            // Reload the configuration so that repo configuration and
            // templates are read from the worktree.
            let config = load_config(
                paths,
                global.as_ref(),
                &templating,
                &defaults,
                inspecting,
                &mut report,
            )?;
            (paths, config)
        }
        None => (paths, config),
    };

    // This is `true` if the current directory is currently inside one of the
    // repos.
    let in_repo_path = paths
        .current
        .is_some_and(|p| repos.iter().any(|m| p.starts_with(m.path())));

    if let Some(opts) = repo_opts {
        let result = apply_repo_options(
            opts,
            paths,
            &current_dir,
            home.as_deref(),
            &config,
            &os,
            &system,
            &repos,
            &sets,
            in_repo_path,
            from_group,
        );

        match result {
            Ok(exclusions) => {
                report.exclusions = exclusions;
            }
            Err(error) if inspecting && report.selection_error.is_none() => {
                report.selection_error = Some(error.to_string());
            }
            Err(error) => return Err(error),
        }
    }

    if let Command::Inspect(inspect) = &opts.action {
        let cx = cli::inspect::Inspect {
            paths,
            worktree: worktree
                .as_ref()
                .map(|(r, c)| (r.as_relative_path(), c.as_relative_path())),
            config: &config,
            global: global.as_ref(),
            repos: &repos,
            from_group,
            in_repo_path,
            repo_opts: &inspect.repo,
            os: &os,
            collected: report,
        };

        return cli::inspect::entry(&cx, &inspect.action);
    }

    if let Some((repo, checkout)) = &worktree
        && repos
            .iter()
            .any(|r| r.path().normalize() == repo.normalize() && !r.is_disabled())
    {
        let dir = checkout.to_path(&root);
        let dir = dir.canonicalize().unwrap_or(dir);
        let repo = if repo.as_str().is_empty() {
            RelativePath::new(".")
        } else {
            repo.as_relative_path()
        };
        tracing::info!("using worktree {} for repo {repo}", dir.display());
    }

    let changes_path = state_dir.join("changes.gz");

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
        Command::Inspect(..) => {
            unreachable!("inspect is handled before repos are processed")
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

            if let Some(global) = &global {
                let state = if global.path.is_file() {
                    "present"
                } else {
                    "absent"
                };

                println!("Global config: {} ({state})", global.path.display());

                if global.provides_repos {
                    println!("Using repos from the global config");
                }
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
        Command::Deploy(opts) => {
            cli::deploy::entry(&mut with_repos, &opts.action)?;
        }
        Command::Install(opts) => {
            cli::install::entry(&mut with_repos, &opts.action)?;
        }
        Command::Set(opts) => {
            cli::set::entry(&mut with_repos.cx, &opts.action)?;
        }
        Command::Project(..) => {
            unreachable!("project is handled before configuration is loaded")
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

    let failed = changes::apply_all(&mut cx.changes_mut(), shared.save, |change| {
        changes::apply(&mut o, &cx, change, shared.save)
    });

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

    let mut outcome = cx.outcome();

    if failed > 0 {
        tracing::error!("Failed to apply {failed} change(s)");
        outcome = ExitCode::FAILURE;
    }

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

/// Load configuration, failing on errors unless `inspecting`, in which case
/// the files which were looked for and the errors are recorded in `report`.
fn load_config<'a>(
    paths: Paths<'a>,
    global: Option<&'a global::Global>,
    templating: &templates::Templating,
    defaults: &'a toml::Table,
    inspecting: bool,
    report: &mut cli::inspect::Collected,
) -> Result<Config<'a>> {
    if !inspecting {
        return config::load_with(paths, templating, defaults, global)
            .context("Loading kick configuration");
    }

    let loaded = config::load_all_with(paths, templating, defaults, global);
    report.sources = loaded.sources;
    report.errors = loaded.errors;
    Ok(loaded.config)
}

/// Detect if the current directory is inside a git checkout nested in a
/// registered repo.
///
/// Returns the `(repo, worktree)` pair to redirect if it is a worktree of that
/// same repo, and errors if it is some other checkout and no repos were
/// selected explicitly. `--all` disables the detection.
///
/// Without registered repos, the repo is the git checkout at the root itself
/// (a standalone repo with its `Kick.toml` at the checkout root). Then only a
/// worktree of it is redirected, with the root as the repo (an empty path),
/// and any other nested checkout is left alone as before.
fn nested_worktree(
    paths: Paths<'_>,
    repo_opts: Option<&RepoOptions>,
    repos: &[Repo],
    from_group: bool,
) -> Result<Option<(RelativePathBuf, RelativePathBuf)>> {
    let (Some(opts), Some(current)) = (repo_opts, paths.current) else {
        return Ok(None);
    };

    if opts.all {
        return Ok(None);
    }

    if !from_group {
        if !repos
            .iter()
            .any(|r| r.path().normalize().as_str().is_empty())
        {
            return Ok(None);
        }

        let checkout = worktree::detect_root(paths.root, current);
        return Ok(checkout.map(|checkout| (RelativePathBuf::new(), checkout)));
    }

    let explicit =
        !opts.repos.is_empty() || !opts.names.is_empty() || !opts.set.iter().all(|s| s.is_empty());

    match worktree::detect(paths.root, current, repos.iter().map(|r| r.path())) {
        Some(worktree::Nested::Worktree { repo, checkout }) => Ok(Some((repo, checkout))),
        Some(worktree::Nested::Other { repo, checkout }) if !explicit => {
            let root = canonical_root(paths.root);

            bail!(
                "{}",
                worktree::nested_checkout_message(
                    &root,
                    &canonical(paths.root, &checkout),
                    &repo,
                    &canonical(paths.root, &repo)
                )
            )
        }
        Some(worktree::Nested::Broken {
            repo,
            checkout,
            gitdir,
        }) if !explicit => {
            let root = canonical_root(paths.root);

            bail!(
                "{}",
                worktree::broken_worktree_message(
                    &root,
                    &canonical(paths.root, &checkout),
                    &gitdir,
                    &repo,
                    &canonical(paths.root, &repo)
                )
            )
        }
        _ => Ok(None),
    }
}

fn canonical_root(root: &Path) -> PathBuf {
    root.canonicalize().unwrap_or_else(|_| root.to_owned())
}

fn canonical(root: &Path, path: &RelativePath) -> PathBuf {
    let dir = path.to_path(root);
    dir.canonicalize().unwrap_or(dir)
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
    current_dir: &Path,
    home: Option<&Path>,
    config: &Config<'_>,
    os: &Os,
    system: &system::System,
    repos: &[Repo],
    sets: &repo_sets::RepoSets,
    in_repo_path: bool,
    from_group: bool,
) -> Result<Vec<Option<Exclusion>>> {
    let has_set = !repo_opts.set.iter().all(|s| s.is_empty());

    // Refuse to guess when there is no explicit selection and we're running
    // from inside of a git checkout which is not one of the configured repos
    // (e.g. an unregistered worktree). Otherwise every configured repo would
    // silently be selected.
    if from_group
        && !repo_opts.all
        && repo_opts.repos.is_empty()
        && repo_opts.names.is_empty()
        && !has_set
        && !in_repo_path
        && let Some(current) = paths.current
        && let Some(checkout) = unregistered_git_checkout(paths.root, current)
    {
        let dir = checkout.to_path(paths.root);
        let dir = dir.canonicalize().unwrap_or(dir);
        let root = paths.root.canonicalize();
        let root = root.as_deref().unwrap_or(paths.root);

        let url = system
            .git
            .first()
            .and_then(|git| git.get_url(&dir, "origin").ok());

        bail!(
            "{}",
            unregistered_checkout_message(root, &dir, checkout, url.as_ref().map(|u| u.as_str()))
        );
    }

    let mut filters = repo_opts
        .repos
        .iter()
        .map(|raw| PathFilter::new(raw, current_dir, home, paths.root, repos))
        .collect::<Vec<_>>();

    // A `-p` filter which is neither a directory inside of a repo nor matches
    // any repo path, like `.` at the root, would silently select nothing.
    // Refuse to run in that case.
    if let Some(message) = unmatched_path_filters_message(
        paths.root,
        &repo_opts.repos,
        &filters,
        repos.iter().map(|repo| repo.path()),
    ) {
        bail!("{message}");
    }

    if !repo_opts.names.is_empty() {
        let named = repos
            .iter()
            .filter_map(|repo| Some((config.selection_name(paths.root, repo.path())?, repo.path())))
            .collect::<Vec<_>>();

        let selected =
            select_by_name(&repo_opts.names, &named).map_err(|message| anyhow!("{message}"))?;
        filters.extend(selected.into_iter().map(PathFilter::Repo));
    }

    let mut git_cache = GitCache::new(repos, paths, system, repo_opts.fetch);
    let mut owned_work = Vec::new();
    let mut work = Vec::new();

    let set = if has_set {
        let mut set = HashSet::<RelativePathBuf>::new();

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
                SetOp::Difference => {
                    let mut other = HashSet::new();

                    work.for_each(|id| {
                        other.insert(id.to_owned());
                    });

                    set = set
                        .symmetric_difference(&other)
                        .cloned()
                        .collect::<HashSet<_>>();
                    continue;
                }
                SetOp::Intersection => {
                    let mut added = HashSet::new();

                    work.for_each(|id| {
                        added.insert(id.to_owned());
                    });

                    set.retain(|id| added.contains(id));
                    continue;
                }
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
    )
}

/// Number of example repo paths to show when `-p` filters match nothing.
const PATH_FILTER_EXAMPLES: usize = 2;

/// Build an error message if any of the `-p`/`--path` `filters` (with their
/// original spelling in `raw`) matches none of the configured `repos`.
///
/// Returns `None` if every filter matches at least one repo.
fn unmatched_path_filters_message<'a>(
    root: &Path,
    raw: &[String],
    filters: &[PathFilter<'_>],
    repos: impl IntoIterator<Item = &'a RelativePath>,
) -> Option<String> {
    let repos = repos.into_iter().collect::<Vec<_>>();

    let unmatched = raw
        .iter()
        .zip(filters)
        .filter(|(_, filter)| !repos.iter().any(|repo| filter.is_match(repo)))
        .map(|(raw, _)| format!("`{raw}`"))
        .collect::<Vec<_>>();

    if unmatched.is_empty() {
        return None;
    }

    let root = root.canonicalize().unwrap_or_else(|_| root.to_owned());

    let (noun, verb) = if unmatched.len() == 1 {
        ("filter", "does")
    } else {
        ("filters", "do")
    };

    let examples = repos
        .iter()
        .take(PATH_FILTER_EXAMPLES)
        .map(|repo| format!("`{}`", repo.as_str()))
        .collect::<Vec<_>>();

    let examples = if examples.is_empty() {
        String::from(" (no repos are configured)")
    } else {
        format!(", such as {}", examples.join(" or "))
    };

    Some(format!(
        "The `-p`/`--path` {noun} {unmatched} {verb} not match any repo configured in `{kick_toml}`.\n\
         \n\
         `-p` takes a directory inside of a repo, or a repo path relative to `{root}`{examples} which supports `*` wildcards.\n\
         Use `--all` to run over every repo, or `--set` to select repos by set.",
        unmatched = unmatched.join(", "),
        kick_toml = root.join(KICK_TOML).display(),
        root = root.display(),
    ))
}

/// Placeholder used in the suggested `Kick.toml` snippet when the checkout has
/// no readable `origin` remote.
const URL_PLACEHOLDER: &str = "<url of the repo>";

/// Build the error explaining that `dir` looks like a project not tracked by
/// the `Kick.toml` in `root`, with a snippet registering it.
fn unregistered_checkout_message(
    root: &Path,
    dir: &Path,
    checkout: &RelativePath,
    url: Option<&str>,
) -> String {
    let kick_toml = root.join(KICK_TOML);
    let url = url.unwrap_or(URL_PLACEHOLDER);

    format!(
        "The current directory looks like a project (git checkout `{dir}`) which is not tracked by kick's {KICK_TOML} at `{root}`.\n\
         \n\
         Passing `--all` runs the command over every repo in that {KICK_TOML} hierarchy (or in the global configuration, if it declares any) instead of just this project.\n\
         Use `-p`/`--path` or `--set` to select repos explicitly.\n\
         \n\
         To make this error go away, register the project in `{kick_toml}`:\n\
         \n\
         [repo.\"{checkout}\"]\n\
         url = \"{url}\"",
        dir = dir.display(),
        root = root.display(),
        kick_toml = kick_toml.display(),
        checkout = checkout.as_str(),
    )
}

/// Find the nearest git checkout (a directory containing a `.git` directory or
/// file) at or above `current`, but strictly below `root`.
fn unregistered_git_checkout<'a>(
    root: &Path,
    current: &'a RelativePath,
) -> Option<&'a RelativePath> {
    let mut path = Some(current);

    while let Some(p) = path {
        if p.as_str().is_empty() {
            break;
        }

        if p.to_path(root).join(".git").exists() {
            return Some(p);
        }

        path = p.parent();
    }

    None
}

/// Why the repo selection excludes a repo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Exclusion {
    /// The repo is not in the sets selected with `--set`.
    NotInSet,
    /// The current directory is inside of another repo.
    OutsideCurrent(RelativePathBuf),
    /// The repo matches none of the `-p`/`--path` filters or `-n`/`--repo-name` names.
    NoPathMatch,
    /// The repo does not support the current operating system, and
    /// `--supported-os` was passed.
    UnsupportedOs(Vec<String>),
    /// The repo was disabled before filtering.
    Disabled,
}

impl fmt::Display for Exclusion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Exclusion::NotInSet => write!(f, "not in the sets selected with `--set`"),
            Exclusion::OutsideCurrent(current) => write!(
                f,
                "the current directory `{current}` is in another repo, pass `--all` to include it"
            ),
            Exclusion::NoPathMatch => {
                write!(f, "not selected by any `-p`/`--path` or `-n`/`--repo-name`")
            }
            Exclusion::UnsupportedOs(os) => write!(
                f,
                "`--supported-os` was passed and it only supports {}",
                os.join(", ")
            ),
            Exclusion::Disabled => write!(f, "disabled"),
        }
    }
}

/// Perform more advanced filtering over modules.
///
/// Returns why each repo was excluded, in the same order as `repos`, or
/// `None` for repos which are selected.
fn filter_repos(
    config: &Config,
    in_current_path: Option<&RelativePath>,
    repo_opts: &RepoOptions,
    repos: &[model::Repo],
    filters: &[PathFilter<'_>],
    set: Option<&HashSet<RelativePathBuf>>,
    expected: &Os,
) -> Result<Vec<Option<Exclusion>>> {
    // Test if repo should be skipped.
    let should_disable = |repo: &Repo| -> Option<Exclusion> {
        if let Some(set) = set
            && !set.contains(repo.path())
        {
            return Some(Exclusion::NotInSet);
        }

        if filters.is_empty() {
            if let Some(path) = in_current_path
                && !path.starts_with(repo.path())
            {
                return Some(Exclusion::OutsideCurrent(path.to_owned()));
            }

            return None;
        }

        if filters.iter().any(|filter| filter.is_match(repo.path())) {
            None
        } else {
            Some(Exclusion::NoPathMatch)
        }
    };

    let mut exclusions = Vec::with_capacity(repos.len());

    for repo in repos {
        let mut exclusion = if repo.is_disabled() {
            Some(Exclusion::Disabled)
        } else {
            should_disable(repo)
        };

        if exclusion.is_none() && repo_opts.supported_os {
            let os = config.os(repo);

            if !os.is_empty() && !os.contains(expected) {
                tracing::trace!("Operating systems {os:?} does not contain {expected:?}");
                exclusion = Some(Exclusion::UnsupportedOs(
                    os.iter().map(|os| os.to_string()).collect(),
                ));
            }
        }

        if exclusion.is_some() {
            repo.disable();
        }

        exclusions.push(exclusion);
    }

    Ok(exclusions)
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

/// Display a repo path, using `.` for the root itself.
fn display_repo_path(path: &RelativePath) -> String {
    let path = path.normalize();

    if path.as_str().is_empty() {
        String::from(".")
    } else {
        path.to_string()
    }
}

/// Number of known repo names to show when a `-n` name matches nothing.
const NAME_EXAMPLES: usize = 5;

/// Resolve `-n`/`--repo-name` arguments to the paths of the repos they name, given
/// the selection name of each repo in `named`.
///
/// Every name must match exactly one repo. Otherwise the error lists the
/// names which match nothing and the candidate paths of those which match
/// more than one repo.
fn select_by_name<S>(
    names: &[String],
    named: &[(S, &RelativePath)],
) -> Result<Vec<RelativePathBuf>, String>
where
    S: AsRef<str>,
{
    let mut selected = Vec::new();
    let mut unknown = Vec::new();
    let mut ambiguous = Vec::new();

    for name in names {
        let mut candidates = named
            .iter()
            .filter(|(n, _)| n.as_ref() == name)
            .map(|(_, path)| path.normalize())
            .collect::<Vec<_>>();

        candidates.sort();
        candidates.dedup();

        match candidates.len() {
            0 => unknown.push(format!("`{name}`")),
            1 => selected.extend(candidates),
            _ => {
                let candidates = candidates
                    .iter()
                    .map(|path| format!("`{}`", display_repo_path(path)))
                    .collect::<Vec<_>>();

                ambiguous.push(format!("`{name}` matches {}", candidates.join(", ")));
            }
        }
    }

    if unknown.is_empty() && ambiguous.is_empty() {
        return Ok(selected);
    }

    let mut out = String::new();

    if !unknown.is_empty() {
        let (noun, verb) = if unknown.len() == 1 {
            ("name", "does")
        } else {
            ("names", "do")
        };

        let mut known = named.iter().map(|(n, _)| n.as_ref()).collect::<Vec<_>>();
        known.sort();
        known.dedup();

        let examples = if known.is_empty() {
            String::from("No repos are configured.")
        } else {
            let mut examples = known
                .iter()
                .take(NAME_EXAMPLES)
                .map(|n| format!("`{n}`"))
                .collect::<Vec<_>>();

            if known.len() > NAME_EXAMPLES {
                examples.push(format!("and {} more", known.len() - NAME_EXAMPLES));
            }

            format!("Known names are {}.", examples.join(", "))
        };

        out.push_str(&format!(
            "The `-n`/`--repo-name` {noun} {} {verb} not match any repo. {examples}\n",
            unknown.join(", ")
        ));
    }

    if !ambiguous.is_empty() {
        let (noun, verb) = if ambiguous.len() == 1 {
            ("name", "matches")
        } else {
            ("names", "match")
        };

        out.push_str(&format!(
            "The `-n`/`--repo-name` {noun} {verb} more than one repo: {}.\n",
            ambiguous.join("; ")
        ));
        out.push_str("Select one of them with `-p`/`--path`, or give it a distinct `alias` in its configuration.\n");
    }

    out.push_str(
        "\nA repo's name is its `alias` if configured, else its `name`, else the last component of its directory.",
    );

    Err(out)
}

/// A `-p`/`--path` filter.
pub(crate) enum PathFilter<'a> {
    /// The argument names a directory inside of the repo at this path.
    Repo(RelativePathBuf),
    /// The argument is matched against repo paths relative to the root.
    Glob(Fragment<'a>),
}

impl<'a> PathFilter<'a> {
    /// Interpret a `-p`/`--path` argument.
    ///
    /// If it names an existing directory, relative to `current_dir`, absolute
    /// or starting with `~/`, it selects the repo containing that directory.
    /// Otherwise it is matched against the repo paths relative to `root`,
    /// which itself may be relative to `current_dir`.
    fn new(
        raw: &'a str,
        current_dir: &Path,
        home: Option<&Path>,
        root: &Path,
        repos: &[Repo],
    ) -> Self {
        if let Some(dir) = resolve_dir(raw, current_dir, home)
            && let Some(repo) = repo_containing(&dir, &current_dir.join(root), repos)
        {
            return Self::Repo(repo.normalize());
        }

        Self::Glob(Fragment::parse(raw))
    }

    /// Test if the filter matches the repo at `path`.
    pub(crate) fn is_match(&self, path: &RelativePath) -> bool {
        match self {
            Self::Repo(repo) => *repo == path.normalize(),
            Self::Glob(fragment) => fragment.is_match(path.as_str()),
        }
    }
}

/// Resolve a `-p`/`--path` argument to the canonical path of the directory it
/// names, if it names an existing directory.
fn resolve_dir(raw: &str, current_dir: &Path, home: Option<&Path>) -> Option<PathBuf> {
    if raw.is_empty() {
        return None;
    }

    let path = match home {
        Some(home) => global::resolve(home, current_dir, raw),
        None => current_dir.join(raw),
    };

    let path = path.canonicalize().ok()?;
    path.is_dir().then_some(path)
}

/// Find the innermost of `repos` whose directory contains the canonical `dir`.
fn repo_containing<'r>(dir: &Path, root: &Path, repos: &'r [Repo]) -> Option<&'r RelativePath> {
    repos
        .iter()
        .filter_map(|repo| {
            let repo_dir = repo.path().to_path(root).canonicalize().ok()?;
            dir.starts_with(&repo_dir)
                .then(|| (repo_dir.components().count(), repo.path()))
        })
        .max_by_key(|(depth, _)| *depth)
        .map(|(_, path)| path)
}

/// Test if a command run from inside of the project at `root` should act on
/// the repos declared in the global configuration at `global_path` instead.
///
/// This is the case if the global configuration declares repos and either
/// `--all` is passed, or a `-p`/`--path` argument names a directory outside
/// of the project which is inside of one of the global repos.
fn selects_global_repos(
    current_dir: &Path,
    root: &Path,
    repo_opts: Option<&RepoOptions>,
    home: &Path,
    global_path: &Path,
) -> bool {
    let Some(opts) = repo_opts else {
        return false;
    };

    let declared = global::declared_repos(global_path);

    if declared.is_empty() {
        return false;
    }

    if opts.all {
        return true;
    }

    let Ok(root) = current_dir.join(root).canonicalize() else {
        return false;
    };

    let global = global::Global {
        path: global_path.to_owned(),
        home: home.to_owned(),
        provides_repos: true,
    };

    let repo_dirs = declared
        .iter()
        .filter_map(|key| global.resolve(key).ok()?.canonicalize().ok())
        .collect::<Vec<_>>();

    let by_path = opts.repos.iter().any(|raw| {
        resolve_dir(raw, current_dir, Some(home)).is_some_and(|dir| {
            !dir.starts_with(&root) && repo_dirs.iter().any(|repo| dir.starts_with(repo))
        })
    });

    by_path || names_select_global_repos(&opts.names, &root, &global)
}

/// Test if a `-n`/`--repo-name` argument matches no repo in the project at `root`
/// but does match one of the repos declared in the `global` configuration.
fn names_select_global_repos(names: &[String], root: &Path, global: &global::Global) -> bool {
    if names.is_empty() {
        return false;
    }

    let Ok(templating) = templates::Templating::new() else {
        return false;
    };

    let defaults = config::defaults();

    let project = global::Global {
        provides_repos: false,
        ..global.clone()
    };

    let in_project = repo_names(root, &project, &templating, &defaults);

    let unmatched = names
        .iter()
        .filter(|name| !in_project.iter().any(|n| n == *name))
        .collect::<Vec<_>>();

    if unmatched.is_empty() {
        return false;
    }

    let in_global = repo_names(&global.home, global, &templating, &defaults);
    unmatched
        .iter()
        .any(|name| in_global.iter().any(|n| n == *name))
}

/// The selection names of the repos kick would act on at `root`.
///
/// Errors loading the configuration are ignored here, they are reported when
/// the configuration is loaded for real.
fn repo_names(
    root: &Path,
    global: &global::Global,
    templating: &templates::Templating,
    defaults: &toml::Table,
) -> Vec<String> {
    let paths = Paths {
        root,
        current: None,
        config: None,
        cache: None,
        redirect: None,
    };

    let loaded = config::load_all_with(paths, templating, defaults, Some(global));

    let Ok((repos, _)) = collect_repos(&loaded.config, root, None) else {
        return Vec::new();
    };

    let mut paths = repos
        .iter()
        .map(|repo| repo.path().to_owned())
        .collect::<Vec<_>>();

    // Without configured repos the root itself is the repo.
    if paths.is_empty() && !global.provides_repos {
        paths.push(RelativePathBuf::new());
    }

    paths
        .iter()
        .filter_map(|path| Some(loaded.config.selection_name(root, path)?.into_owned()))
        .collect()
}

/// Find the root to use from the current directory, returning the root, the
/// current directory relative to it, and whether the repos come from the
/// global configuration at `global_path`.
///
/// Inside a project (a `Kick.toml` or a git checkout is found upward) that is
/// the project. Outside of one, if the global configuration declares repos,
/// the root is the home directory and the global repos are used. Otherwise
/// the current directory is the root.
fn find_root(
    current_dir: &Path,
    home: Option<&Path>,
    global_path: Option<&Path>,
) -> (PathBuf, Option<RelativePathBuf>, bool) {
    if let Some((root, current_path)) = find_from_current_dir(current_dir) {
        return (root, Some(current_path), false);
    }

    if let (Some(home), Some(global_path)) = (home, global_path)
        && !global::declared_repos(global_path).is_empty()
    {
        tracing::trace!(
            "Using repos from global configuration {}",
            global_path.display()
        );
        return (home.to_owned(), None, true);
    }

    (current_dir.to_owned(), Some(RelativePathBuf::new()), false)
}

/// Collect the repos kick acts on.
///
/// Repos are only ever those declared in configuration with
/// `[repo."<path>"]`. Without any, the project root itself is the repo. The
/// returned flag is `true` if the repos came from configuration.
fn collect_repos(
    config: &Config<'_>,
    root: &Path,
    git: Option<&system::Git>,
) -> Result<(Vec<Repo>, bool)> {
    let mut repos = Vec::new();

    for (path, config) in config.repos.iter() {
        let Some(url) = config.urls.iter().next() else {
            continue;
        };

        let source = config
            .sources
            .iter()
            .find(|s| matches!(s, RepoSource::Global(..)))
            .cloned()
            .unwrap_or_else(|| RepoSource::Config(path.clone()));

        repos.push(Repo::new([source], path.to_owned(), url.clone()));
    }

    tracing::trace!(
        modules = repos
            .iter()
            .map(|m| m.path().to_string())
            .collect::<Vec<_>>()
            .join(", "),
        "loaded modules"
    );

    if !repos.is_empty() {
        return Ok((repos, true));
    }

    if let Some((path, url)) = load_from_git(root, git)? {
        repos.push(Repo::new(BTreeSet::from([RepoSource::Git]), path, url));
    }

    Ok((repos, false))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use relative_path::RelativePath;

    use super::{
        PathFilter, RepoOptions, collect_repos, nested_worktree, selects_global_repos,
        unmatched_path_filters_message, unregistered_checkout_message, unregistered_git_checkout,
    };
    use crate::config;
    use crate::ctxt::{Paths, Redirect};
    use crate::glob::{Fragment, Glob};
    use crate::model::{Repo, RepoSource};
    use crate::system::Git;
    use crate::templates::Templating;
    use crate::worktree::tests::{git, init};

    /// Load the configuration the way `entry` does when running from
    /// `current`, returning the worktree redirect, if any, and the deploy
    /// user of each repo.
    fn load_from(root: &Path, current: &str) -> (Option<String>, Vec<(String, Option<String>)>) {
        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        // Paths relative to `current` are resolved from the working
        // directory, so only use it to detect the worktree and resolve
        // everything else from the root.
        let paths = Paths {
            root,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let config = config::load(paths, &templating, &defaults).unwrap();
        let url = url::Url::parse("https://example.com/repo").unwrap();

        let mut repos = config
            .repos
            .keys()
            .map(|path| {
                Repo::new(
                    [RepoSource::Config(path.clone())],
                    path.clone(),
                    url.clone(),
                )
            })
            .collect::<Vec<_>>();

        let from_group = !repos.is_empty();

        if !from_group {
            repos.push(Repo::new([RepoSource::Git], ".".into(), url));
        }

        let opts = RepoOptions::default();
        let detect = Paths {
            current: Some(RelativePath::new(current)),
            ..paths
        };

        let worktree = nested_worktree(detect, Some(&opts), &repos, from_group).unwrap();

        let (redirect, config) = match &worktree {
            Some((repo, checkout)) => {
                let paths = Paths {
                    redirect: Some(Redirect {
                        repo,
                        worktree: checkout,
                    }),
                    ..paths
                };

                let config = config::load(paths, &templating, &defaults).unwrap();
                (Some(format!("{repo} -> {checkout}")), config)
            }
            None => (None, config),
        };

        let users = repos
            .iter()
            .map(|repo| (repo.path().to_string(), config.deploy(repo).user))
            .collect();

        (redirect, users)
    }

    fn users(users: &[(&str, &str)]) -> Vec<(String, Option<String>)> {
        users
            .iter()
            .map(|(repo, user)| ((*repo).to_owned(), Some((*user).to_owned())))
            .collect()
    }

    /// A git submodule is not a repo: without any `[repo]` sections the
    /// project root is the only repo, and deploy file sources resolve against
    /// it.
    #[test]
    fn gitmodules_are_not_repos() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        init(root);
        git(
            root,
            &["remote", "add", "origin", "https://example.com/project"],
        );
        init(&root.join("3rdparty/heroicons"));
        fs::write(
            root.join(".gitmodules"),
            "[submodule \"3rdparty/heroicons\"]\n\
             \tpath = 3rdparty/heroicons\n\
             \turl = https://github.com/tailwindlabs/heroicons\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("systemd/remote")).unwrap();
        fs::write(root.join("systemd/remote/sysusers.conf"), "u app - -\n").unwrap();
        fs::write(
            root.join("Kick.toml"),
            "[[deploy.profiles.remote.files]]\n\
             source = \"systemd/remote/sysusers.conf\"\n\
             dest = \"/usr/lib/sysusers.d/\"\n",
        )
        .unwrap();

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        let paths = Paths {
            root,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let config = config::load(paths, &templating, &defaults).unwrap();
        let git = Git::new("git".into());
        let (repos, from_group) = collect_repos(&config, root, Some(&git)).unwrap();

        assert!(!from_group);

        let found = repos
            .iter()
            .map(|repo| (repo.path().as_str(), repo.url().as_str()))
            .collect::<Vec<_>>();
        assert_eq!(found, [(".", "https://example.com/project")]);

        let repo = &repos[0];
        let deploy = config.deploy(repo).with_profile("remote").unwrap();
        let repo_root = repo.path().to_path(root);

        let mut matched = Vec::new();

        for file in &deploy.files {
            for source in Glob::new(&repo_root, &file.source).matcher() {
                let path = repo_root.join(source.unwrap().as_str());
                assert!(path.is_file(), "{}", path.display());
                matched.push(path);
            }
        }

        assert_eq!(matched, [root.join("systemd/remote/sysusers.conf")]);
    }

    #[test]
    fn standalone_repo_worktree_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        init(root);
        fs::write(root.join("Kick.toml"), "[deploy]\nuser = \"main\"\n").unwrap();
        git(root, &["add", "Kick.toml"]);
        git(root, &["commit", "-q", "-m", "kick"]);
        git(root, &["worktree", "add", "-q", ".claude/worktrees/wt"]);
        fs::create_dir_all(root.join(".claude/worktrees/wt/src")).unwrap();
        fs::write(
            root.join(".claude/worktrees/wt/Kick.toml"),
            "[deploy]\nuser = \"worktree\"\n",
        )
        .unwrap();
        // A nested clone which is not a worktree.
        init(&root.join("vendor/clone"));

        let worktree = Some(String::from(" -> .claude/worktrees/wt"));

        assert_eq!(
            load_from(root, ".claude/worktrees/wt"),
            (worktree.clone(), users(&[(".", "worktree")]))
        );
        assert_eq!(
            load_from(root, ".claude/worktrees/wt/src"),
            (worktree, users(&[(".", "worktree")]))
        );
        assert_eq!(load_from(root, ""), (None, users(&[(".", "main")])));
        assert_eq!(
            load_from(root, "vendor/clone"),
            (None, users(&[(".", "main")]))
        );

        // Running from the worktree as root, as with `--root .`.
        let worktree = root.join(".claude/worktrees/wt");
        assert_eq!(
            load_from(&worktree, ""),
            (None, users(&[(".", "worktree")]))
        );
    }

    #[test]
    fn registered_repo_worktree_config() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let track = root.join("repos/track");

        fs::write(
            root.join("Kick.toml"),
            "[repo.\"repos/track\"]\nurl = \"https://example.com/track\"\n\
             [repo.\"repos/other\"]\nurl = \"https://example.com/other\"\n",
        )
        .unwrap();

        init(&track);
        init(&root.join("repos/other"));
        fs::write(track.join("Kick.toml"), "[deploy]\nuser = \"main\"\n").unwrap();
        fs::write(
            root.join("repos/other/Kick.toml"),
            "[deploy]\nuser = \"other\"\n",
        )
        .unwrap();
        git(&track, &["add", "Kick.toml"]);
        git(&track, &["commit", "-q", "-m", "kick"]);
        git(&track, &["worktree", "add", "-q", ".claude/worktrees/wt"]);
        fs::write(
            track.join(".claude/worktrees/wt/Kick.toml"),
            "[deploy]\nuser = \"worktree\"\n",
        )
        .unwrap();

        assert_eq!(
            load_from(root, "repos/track/.claude/worktrees/wt"),
            (
                Some(String::from(
                    "repos/track -> repos/track/.claude/worktrees/wt"
                )),
                users(&[("repos/other", "other"), ("repos/track", "worktree")])
            )
        );
        assert_eq!(
            load_from(root, "repos/track"),
            (
                None,
                users(&[("repos/other", "other"), ("repos/track", "main")])
            )
        );
    }

    fn path_filters_message(filters: &[&str], repos: &[&str]) -> Option<String> {
        let raw = filters.iter().map(|f| f.to_string()).collect::<Vec<_>>();
        let fragments = raw
            .iter()
            .map(|f| PathFilter::Glob(Fragment::parse(f)))
            .collect::<Vec<_>>();

        unmatched_path_filters_message(
            Path::new("/nonexistent-kick-root"),
            &raw,
            &fragments,
            repos.iter().map(|r| RelativePath::new(*r)),
        )
    }

    #[test]
    fn path_filters_all_matching() {
        let repos = ["repos/foo", "repos/bar", "other/baz"];
        assert_eq!(path_filters_message(&[], &repos), None);
        assert_eq!(path_filters_message(&["repos/foo"], &repos), None);
        assert_eq!(path_filters_message(&["repos/*", "*baz"], &repos), None);
    }

    #[test]
    fn path_filters_single_unmatched() {
        let repos = ["repos/foo", "repos/bar", "other/baz"];

        assert_eq!(
            path_filters_message(&["."], &repos).as_deref(),
            Some(
                "The `-p`/`--path` filter `.` does not match any repo configured in `/nonexistent-kick-root/Kick.toml`.\n\
                 \n\
                 `-p` takes a directory inside of a repo, or a repo path relative to `/nonexistent-kick-root`, such as `repos/foo` or `repos/bar` which supports `*` wildcards.\n\
                 Use `--all` to run over every repo, or `--set` to select repos by set."
            )
        );
    }

    #[test]
    fn path_filters_partially_unmatched() {
        let repos = ["repos/foo"];

        let message = path_filters_message(&["repos/foo", ".", "nope*"], &repos)
            .expect("unmatched filters should fail");

        assert!(
            message.starts_with(
                "The `-p`/`--path` filters `.`, `nope*` do not match any repo configured in"
            ),
            "{message}"
        );
        assert!(message.contains("such as `repos/foo` which"), "{message}");
    }

    #[test]
    fn path_filters_without_repos() {
        let message = path_filters_message(&["foo"], &[]).expect("no repos should fail");
        assert!(message.contains("(no repos are configured)"), "{message}");
    }

    #[test]
    fn unregistered_checkout_message_with_url() {
        let message = unregistered_checkout_message(
            Path::new("/src"),
            Path::new("/src/repos/foo"),
            RelativePath::new("repos/foo"),
            Some("https://github.com/udoprog/foo"),
        );

        assert_eq!(
            message,
            "The current directory looks like a project (git checkout `/src/repos/foo`) which is not tracked by kick's Kick.toml at `/src`.\n\
             \n\
             Passing `--all` runs the command over every repo in that Kick.toml hierarchy (or in the global configuration, if it declares any) instead of just this project.\n\
             Use `-p`/`--path` or `--set` to select repos explicitly.\n\
             \n\
             To make this error go away, register the project in `/src/Kick.toml`:\n\
             \n\
             [repo.\"repos/foo\"]\n\
             url = \"https://github.com/udoprog/foo\""
        );
    }

    #[test]
    fn unregistered_checkout_message_without_url() {
        let message = unregistered_checkout_message(
            Path::new("/src"),
            Path::new("/src/foo"),
            RelativePath::new("foo"),
            None,
        );

        assert!(
            message.ends_with("[repo.\"foo\"]\nurl = \"<url of the repo>\""),
            "{message}"
        );
    }

    #[test]
    fn detects_unregistered_git_checkouts() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();

        // A worktree has a `.git` file, a regular clone a `.git` directory.
        fs::create_dir_all(root.join("worktree/src/nested"))?;
        fs::write(root.join("worktree/.git"), "gitdir: /elsewhere\n")?;
        fs::create_dir_all(root.join("clone/.git"))?;
        fs::create_dir_all(root.join("plain/sub"))?;
        // A `.git` in root itself must not count.
        fs::create_dir_all(root.join(".git"))?;

        let check = |p: &str| {
            unregistered_git_checkout(root, RelativePath::new(p)).map(|p| p.as_str().to_owned())
        };

        assert_eq!(check("worktree").as_deref(), Some("worktree"));
        assert_eq!(check("worktree/src/nested").as_deref(), Some("worktree"));
        assert_eq!(check("clone").as_deref(), Some("clone"));
        assert_eq!(check("plain/sub"), None);
        assert_eq!(check("plain"), None);
        assert_eq!(check(""), None);
        Ok(())
    }

    /// Write the global configuration and return a [`Global`] for it.
    fn global_config(dir: &Path, contents: &str, provides_repos: bool) -> crate::global::Global {
        let path = dir.join("config/kick/Kick.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, contents).unwrap();

        crate::global::Global {
            path,
            home: dir.join("home"),
            provides_repos,
        }
    }

    #[test]
    fn outside_project_uses_global_repos() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let outside = dir.path().join("outside");
        let project = dir.path().join("project");
        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::create_dir_all(project.join("sub")).unwrap();
        fs::write(project.join("Kick.toml"), "").unwrap();

        let global = global_config(
            dir.path(),
            "[repo.\"~/a\"]\nurl = \"https://example.com/a\"\n",
            true,
        );

        let (root, current, global_repos) =
            super::find_root(&outside, Some(&home), Some(&global.path));
        assert_eq!(root, home);
        assert_eq!(current, None);
        assert!(global_repos);

        // Inside a project the global repos are not used.
        let (_, current, global_repos) =
            super::find_root(&project.join("sub"), Some(&home), Some(&global.path));
        assert_eq!(current.as_deref().map(|c| c.as_str()), Some("sub"));
        assert!(!global_repos);

        // Without global repos, the current directory is the root as before.
        let empty = global_config(dir.path(), "license = \"MIT\"\n", true);
        let (root, current, global_repos) =
            super::find_root(&outside, Some(&home), Some(&empty.path));
        assert_eq!(root, outside);
        assert_eq!(current.as_deref().map(|c| c.as_str()), Some(""));
        assert!(!global_repos);

        // A repo without a url is not a repo.
        let no_url = global_config(dir.path(), "[repo.\"~/a\"]\nbranch = \"main\"\n", true);
        let (_, _, global_repos) = super::find_root(&outside, Some(&home), Some(&no_url.path));
        assert!(!global_repos);
    }

    #[test]
    fn global_repos_resolve_against_home() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let elsewhere = dir.path().join("elsewhere/c");

        fs::create_dir_all(home.join("a")).unwrap();
        fs::create_dir_all(home.join("templates")).unwrap();
        fs::create_dir_all(&elsewhere).unwrap();
        fs::write(home.join("a/Kick.toml"), "license = \"own\"\n").unwrap();
        fs::write(home.join("templates/lib.md"), "lib template\n").unwrap();

        let global = global_config(
            dir.path(),
            &format!(
                "license = \"global\"\n\
                 lib = \"~/templates/lib.md\"\n\
                 \n\
                 [deploy]\n\
                 user = \"global\"\n\
                 \n\
                 [repo.\"~/a\"]\n\
                 url = \"https://example.com/a\"\n\
                 \n\
                 [repo.\"~/b\"]\n\
                 url = \"https://example.com/b\"\n\
                 \n\
                 [repo.\"{}\"]\n\
                 url = \"https://example.com/c\"\n",
                elsewhere.display()
            ),
            true,
        );

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        let paths = Paths {
            root: &home,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let loaded = config::load_all_with(paths, &templating, &defaults, Some(&global));
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let config = loaded.config;

        let keys = config.repos.keys().map(|k| k.as_str()).collect::<Vec<_>>();
        assert_eq!(keys, ["../elsewhere/c", "a", "b"]);

        let (repos, from_group) = collect_repos(&config, &home, None).unwrap();
        assert!(from_group);
        assert_eq!(repos.len(), 3);

        let a = repos.iter().find(|r| r.path() == "a").unwrap();
        let b = repos.iter().find(|r| r.path() == "b").unwrap();

        assert!(
            a.source_list()
                .any(|s| *s == RepoSource::Global(String::from("~/a")))
        );

        // The repo's own configuration takes precedence over the global one.
        assert_eq!(config.license(a), "own");
        assert_eq!(config.license(b), "global");
        assert_eq!(config.deploy(a).user.as_deref(), Some("global"));
        assert!(config.lib(b).is_some());

        // The global configuration is reported as a source.
        let global_source = loaded.sources.iter().find(|s| s.global).unwrap();
        assert!(!global_source.repos_ignored);
        assert!(global_source.repos.iter().any(|(p, _)| p == "a"));
    }

    #[test]
    fn global_relative_paths_resolve_against_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let config_dir = dir.path().join("config/kick");

        fs::create_dir_all(&home).unwrap();
        fs::create_dir_all(config_dir.join("data")).unwrap();
        fs::create_dir_all(config_dir.join("repos/a")).unwrap();
        fs::write(config_dir.join("data/lib.md"), "lib template\n").unwrap();

        // A template with the same relative path under the home directory
        // must not be the one used.
        fs::create_dir_all(home.join("data")).unwrap();
        fs::write(home.join("data/lib.md"), "wrong template\n").unwrap();

        let global = global_config(
            dir.path(),
            "lib = \"data/lib.md\"\n\
             \n\
             [repo.\"repos/a\"]\n\
             url = \"https://example.com/a\"\n",
            true,
        );

        assert_eq!(global.dir(), config_dir);
        assert_eq!(
            global.resolve("data/lib.md").unwrap(),
            config_dir.join("data/lib.md")
        );

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        let paths = Paths {
            root: &home,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let loaded = config::load_all_with(paths, &templating, &defaults, Some(&global));
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let config = loaded.config;

        // The relative repo key resolves against the directory of the global
        // `Kick.toml`, expressed relative to the root.
        let keys = config.repos.keys().map(|k| k.as_str()).collect::<Vec<_>>();
        assert_eq!(keys, ["../config/kick/repos/a"]);

        let source = loaded.sources.iter().find(|s| s.global).unwrap();
        assert!(
            source
                .repos
                .iter()
                .any(|(p, _)| p == "../config/kick/repos/a")
        );

        let (repos, _) = collect_repos(&config, &home, None).unwrap();
        let a = &repos[0];
        assert_eq!(a.path(), "../config/kick/repos/a");

        // The template is read from the directory of the global `Kick.toml`.
        let lib = config.lib(a).expect("lib template");
        let rendered = lib.render(&serde_json::json!({})).unwrap();
        assert_eq!(rendered, "lib template\n");
    }

    #[test]
    fn global_config_is_least_specific_in_project() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir_all(dir.path().join("home")).unwrap();
        fs::create_dir_all(root.join("x")).unwrap();

        fs::write(
            root.join("Kick.toml"),
            "[deploy]\nuser = \"project\"\n\n[repo.\"x\"]\nurl = \"https://example.com/x\"\n",
        )
        .unwrap();

        let global = global_config(
            dir.path(),
            "license = \"global\"\nbranch = \"trunk\"\n\n[deploy]\nuser = \"global\"\n\n[repo.\"~/a\"]\nurl = \"https://example.com/a\"\n",
            false,
        );

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        let paths = Paths {
            root: &root,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let loaded = config::load_all_with(paths, &templating, &defaults, Some(&global));
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);

        // The global configuration is loaded last.
        let last = loaded.sources.last().unwrap();
        assert!(last.global);
        assert!(last.repos_ignored);

        let config = loaded.config;
        let keys = config.repos.keys().map(|k| k.as_str()).collect::<Vec<_>>();
        assert_eq!(keys, ["x"], "global repos are not merged into a project");

        let (repos, _) = collect_repos(&config, &root, None).unwrap();
        let x = &repos[0];

        assert_eq!(config.deploy(x).user.as_deref(), Some("project"));
        assert_eq!(config.license(x), "global");
        assert_eq!(config.branch(x), Some("trunk"));
    }

    /// A variable which is never set, used to test interpolation errors and
    /// fallbacks.
    const UNSET: &str = "KICK_TEST_UNSET_61D2B90B";

    #[test]
    fn project_paths_are_interpolated() {
        // NB: Cargo sets this when running tests.
        assert_eq!(
            std::env::var("CARGO_PKG_NAME").ok().as_deref(),
            Some(env!("CARGO_PKG_NAME"))
        );

        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir_all(dir.path().join("home")).unwrap();
        fs::create_dir_all(root.join("templates")).unwrap();
        fs::write(root.join("templates/lib.md"), "lib template\n").unwrap();

        fs::write(
            root.join("Kick.toml"),
            format!(
                "lib = \"${{{UNSET}:-templates}}/lib.md\"\n\
                 \n\
                 [repo.\"repos/$CARGO_PKG_NAME\"]\n\
                 url = \"https://example.com/kick\"\n\
                 cargo_toml = \"$$weird/${{CARGO_PKG_NAME}}.toml\"\n\
                 \n\
                 [repo.\"~/elsewhere\"]\n\
                 url = \"https://example.com/elsewhere\"\n"
            ),
        )
        .unwrap();

        let global = global_config(dir.path(), "", false);

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        let paths = Paths {
            root: &root,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let loaded = config::load_all_with(paths, &templating, &defaults, Some(&global));
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let config = loaded.config;

        // `~` expands to the home directory and environment variables are
        // interpolated in repo keys.
        let keys = config.repos.keys().map(|k| k.as_str()).collect::<Vec<_>>();
        assert_eq!(keys, ["../home/elsewhere", "repos/kick"]);

        let (repos, _) = collect_repos(&config, &root, None).unwrap();
        let kick = repos.iter().find(|r| r.path() == "repos/kick").unwrap();

        // `$$` is a literal `$`.
        assert_eq!(
            config.cargo_toml(kick).map(|p| p.as_str()),
            Some("$weird/kick.toml")
        );

        // The fallback is used for the unset variable.
        assert!(config.lib(kick).is_some());
    }

    #[test]
    fn global_paths_are_interpolated() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        fs::create_dir_all(home.join("templates")).unwrap();
        fs::write(home.join("templates/lib.md"), "lib template\n").unwrap();

        let global = global_config(
            dir.path(),
            &format!(
                "lib = \"~/${{{UNSET}:-templates}}/lib.md\"\n\
                 \n\
                 [repo.\"~/src/$CARGO_PKG_NAME\"]\n\
                 url = \"https://example.com/kick\"\n\
                 \n\
                 [repo.\"${{{UNSET}:-~/other}}\"]\n\
                 url = \"https://example.com/other\"\n"
            ),
            true,
        );

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        let paths = Paths {
            root: &home,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let loaded = config::load_all_with(paths, &templating, &defaults, Some(&global));
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let config = loaded.config;

        // NB: `~` is only expanded at the start of a value, so the fallback
        // `~/other` is a relative path starting with a directory named `~`,
        // which is resolved against the directory of the global `Kick.toml`.
        let keys = config.repos.keys().map(|k| k.as_str()).collect::<Vec<_>>();
        assert_eq!(keys, ["../config/kick/~/other", "src/kick"]);

        let (repos, _) = collect_repos(&config, &home, None).unwrap();
        let kick = repos.iter().find(|r| r.path() == "src/kick").unwrap();

        // The key is kept as written.
        assert!(
            kick.source_list()
                .any(|s| *s == RepoSource::Global(String::from("~/src/$CARGO_PKG_NAME")))
        );

        assert!(config.lib(kick).is_some());

        // The source lists the repos by the path they resolve to.
        let source = loaded.sources.iter().find(|s| s.global).unwrap();
        assert!(source.repos.iter().any(|(p, _)| p == "src/kick"));
    }

    #[test]
    fn unset_variables_are_errors() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        fs::create_dir_all(dir.path().join("home")).unwrap();
        fs::create_dir_all(&root).unwrap();

        fs::write(
            root.join("Kick.toml"),
            format!(
                "[repo.\"repos/${UNSET}\"]\n\
                 url = \"https://example.com/a\"\n\
                 \n\
                 [repo.\"b\"]\n\
                 url = \"https://example.com/b\"\n\
                 cargo_toml = \"${{{UNSET}}}/Cargo.toml\"\n"
            ),
        )
        .unwrap();

        let global = global_config(dir.path(), &format!("lib = \"~/${UNSET}/lib.md\"\n"), false);

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        let paths = Paths {
            root: &root,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let loaded = config::load_all_with(paths, &templating, &defaults, Some(&global));
        let errors = loaded
            .errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>();

        assert_eq!(errors.len(), 3, "{errors:?}");

        let project = root.join("Kick.toml");
        let project = project.display();

        // Each error names the file, the key and the variable.
        let find = |needle: &str| {
            errors
                .iter()
                .find(|e| e.contains(needle))
                .unwrap_or_else(|| panic!("no error containing {needle:?} in {errors:?}"))
        };

        let repo_key = find(&format!("repos/${UNSET}"));
        assert!(repo_key.starts_with(&format!("{project}: ")), "{repo_key}");
        assert!(
            repo_key.contains(&format!("environment variable `{UNSET}` is not set")),
            "{repo_key}"
        );

        let cargo_toml = find("cargo_toml");
        assert!(
            cargo_toml.starts_with(&format!("{project}: ")),
            "{cargo_toml}"
        );
        assert!(cargo_toml.contains(&format!("`{UNSET}`")), "{cargo_toml}");

        let lib = find("lib");
        assert!(
            lib.starts_with(&format!("{}: .lib: ", global.path.display())),
            "{lib}"
        );
        assert!(lib.contains(&format!("`{UNSET}`")), "{lib}");
    }

    fn test_repos(paths: &[&str]) -> Vec<Repo> {
        let url = url::Url::parse("https://example.com/repo").unwrap();

        paths
            .iter()
            .map(|path| Repo::new([RepoSource::Git], (*path).into(), url.clone()))
            .collect()
    }

    /// Resolve `-p` arguments from `current`, returning the paths of the
    /// matching repos for each.
    fn select(
        raw: &[&str],
        current: &Path,
        home: &Path,
        root: &Path,
        repos: &[Repo],
    ) -> Vec<Vec<String>> {
        raw.iter()
            .map(|raw| {
                let filter = PathFilter::new(raw, current, Some(home), root, repos);

                repos
                    .iter()
                    .filter(|repo| filter.is_match(repo.path()))
                    .map(|repo| repo.path().to_string())
                    .collect()
            })
            .collect()
    }

    #[test]
    fn path_filter_spellings() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let root = home.join("projects");

        fs::create_dir_all(root.join("repos/a/src/nested")).unwrap();
        fs::create_dir_all(root.join("repos/ab")).unwrap();
        fs::create_dir_all(root.join("repos/b")).unwrap();
        fs::create_dir_all(root.join("other")).unwrap();

        let repos = test_repos(&["repos/a", "repos/ab", "repos/b"]);
        let repos_dir = root.join("repos");
        let a = &[vec![String::from("repos/a")]][..];

        // Relative to the current directory.
        assert_eq!(select(&["a"], &repos_dir, &home, &root, &repos), a);
        assert_eq!(select(&["./a"], &repos_dir, &home, &root, &repos), a);
        assert_eq!(select(&["a/"], &repos_dir, &home, &root, &repos), a);
        assert_eq!(select(&["../repos/a"], &repos_dir, &home, &root, &repos), a);
        assert_eq!(
            select(
                &["../../projects/repos/a/"],
                &repos_dir,
                &home,
                &root,
                &repos
            ),
            a
        );
        assert_eq!(
            select(&[".."], &root.join("repos/a/src"), &home, &root, &repos),
            a
        );
        assert_eq!(
            select(&["."], &root.join("repos/a"), &home, &root, &repos),
            a
        );

        // Absolute and home-relative.
        let absolute = root.join("repos/a");
        let absolute = absolute.to_str().unwrap();
        assert_eq!(select(&[absolute], &home, &home, &root, &repos), a);
        assert_eq!(
            select(&["~/projects/repos/a"], &root, &home, &root, &repos),
            a
        );
        assert_eq!(
            select(&["~/projects/repos/a/"], &root, &home, &root, &repos),
            a
        );

        // A directory inside of a repo selects the repo.
        assert_eq!(
            select(&["a/src/nested"], &repos_dir, &home, &root, &repos),
            a
        );

        // Spelled as a path relative to the root (from the root) as before.
        assert_eq!(select(&["repos/a"], &root, &home, &root, &repos), a);

        // A root which is relative to the current directory.
        assert_eq!(
            select(&["a"], &repos_dir, &home, Path::new(".."), &repos),
            a
        );

        // Symlinks resolve to the repo they point to.
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(root.join("repos/a"), home.join("link")).unwrap();
            assert_eq!(select(&["~/link"], &root, &home, &root, &repos), a);
            assert_eq!(select(&["link/src"], &home, &home, &root, &repos), a);
        }
    }

    #[test]
    fn path_filter_glob_fallback() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().canonicalize().unwrap();
        let root = home.join("projects");

        fs::create_dir_all(root.join("repos/a")).unwrap();
        fs::create_dir_all(root.join("repos/b")).unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();

        let repos = test_repos(&["repos/a", "repos/b"]);
        let sub = root.join("sub");

        // Not a directory relative to the current directory, so it is matched
        // against the repo paths relative to the root.
        assert_eq!(
            select(&["repos/*", "repos/b", "*a"], &sub, &home, &root, &repos),
            [
                vec![String::from("repos/a"), String::from("repos/b")],
                vec![String::from("repos/b")],
                vec![String::from("repos/a")],
            ]
        );

        // An existing directory which is not inside of a repo falls back as
        // well, and matches nothing.
        assert_eq!(
            select(&[".", "nope"], &root, &home, &root, &repos),
            [Vec::<String>::new(), Vec::new()]
        );

        // The unmatched error names the arguments as typed.
        let raw = [
            String::from("repos/a"),
            String::from("."),
            String::from("nope"),
        ];
        let filters = raw
            .iter()
            .map(|r| PathFilter::new(r, &root, Some(&home), &root, &repos))
            .collect::<Vec<_>>();
        let message =
            unmatched_path_filters_message(&root, &raw, &filters, repos.iter().map(|r| r.path()))
                .expect("unmatched filters should fail");
        assert!(
            message.contains("filters `.`, `nope` do not match"),
            "{message}"
        );
    }

    #[test]
    fn global_repos_from_inside_project() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let home = base.join("home");
        let project = home.join("project");
        let other = home.join("src/other");

        fs::create_dir_all(project.join("repos/inner/src")).unwrap();
        fs::create_dir_all(other.join("src")).unwrap();
        fs::create_dir_all(home.join("src/third")).unwrap();
        fs::create_dir_all(home.join("unregistered")).unwrap();
        fs::write(project.join("Kick.toml"), "license = \"MIT\"\n").unwrap();

        const GLOBAL: &str = "[repo.\"~/src/other\"]\nurl = \"https://example.com/other\"\n\
             [repo.\"~/src/third\"]\nurl = \"https://example.com/third\"\n\
             [repo.\"~/project\"]\nurl = \"https://example.com/project\"\n";

        let global = global_config(&base, GLOBAL, true);

        let current = project.join("repos/inner/src");
        let (root, _, global_repos) = super::find_root(&current, Some(&home), Some(&global.path));
        assert!(!global_repos);
        assert_eq!(current.join(&root).canonicalize().unwrap(), project);

        let opts = |all: bool, paths: &[&str]| RepoOptions {
            all,
            repos: paths.iter().map(|p| p.to_string()).collect(),
            ..RepoOptions::default()
        };

        let switch = |opts: &RepoOptions, global_path: &Path| {
            selects_global_repos(&current, &root, Some(opts), &home, global_path)
        };

        // A directory outside of the project inside of a global repo.
        assert!(switch(
            &opts(false, &["../../../../src/other"]),
            &global.path
        ));
        assert!(switch(&opts(false, &["~/src/other/src"]), &global.path));
        let absolute = other.to_str().unwrap();
        assert!(switch(&opts(false, &[absolute]), &global.path));
        // `--all` switches to the global repos.
        assert!(switch(&opts(true, &[]), &global.path));

        // Directories inside of the project, globs and directories outside of
        // any global repo stay in the project.
        assert!(!switch(&opts(false, &[]), &global.path));
        assert!(!switch(&opts(false, &["."]), &global.path));
        assert!(!switch(
            &opts(false, &["~/project/repos/inner"]),
            &global.path
        ));
        assert!(!switch(&opts(false, &["repos/*"]), &global.path));
        assert!(!switch(&opts(false, &["~/unregistered"]), &global.path));

        // Without global repos nothing changes.
        let empty = global_config(&base, "license = \"MIT\"\n", true);
        assert!(!switch(&opts(true, &[]), &empty.path));
        assert!(!switch(&opts(false, &["~/src/other"]), &empty.path));

        // Once switched, the root is the home directory and the same arguments
        // select among the global repos.
        let global = global_config(&base, GLOBAL, true);

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        let paths = Paths {
            root: &home,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let loaded = config::load_all_with(paths, &templating, &defaults, Some(&global));
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let (repos, from_group) = collect_repos(&loaded.config, &home, None).unwrap();
        assert!(from_group);

        let mut all = repos
            .iter()
            .map(|r| r.path().to_string())
            .collect::<Vec<_>>();
        all.sort();
        assert_eq!(all, ["project", "src/other", "src/third"]);

        assert_eq!(
            select(
                &["../../../../src/other", "~/src/third/", ".."],
                &current,
                &home,
                &home,
                &repos
            ),
            [
                vec![String::from("src/other")],
                vec![String::from("src/third")],
                vec![String::from("project")],
            ]
        );
    }

    #[test]
    fn clap_definition() {
        use clap::CommandFactory;
        super::Opts::command().debug_assert();
    }

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    #[test]
    fn select_by_name_matches() {
        let named = [
            ("a", RelativePath::new("repos/a")),
            ("b", RelativePath::new("repos/b")),
            ("dup", RelativePath::new("one/dup")),
            ("dup", RelativePath::new("two/dup")),
            ("root", RelativePath::new("")),
        ];

        assert_eq!(
            super::select_by_name(&names(&["b", "a", "root"]), &named).unwrap(),
            ["repos/b", "repos/a", ""]
                .map(relative_path::RelativePathBuf::from)
                .to_vec()
        );

        let message = super::select_by_name(&names(&["nope"]), &named).unwrap_err();
        assert_eq!(
            message,
            "The `-n`/`--repo-name` name `nope` does not match any repo. Known names are `a`, `b`, `dup`, `root`.\n\
             \n\
             A repo's name is its `alias` if configured, else its `name`, else the last component of its directory."
        );

        let message = super::select_by_name(&names(&["a", "dup"]), &named).unwrap_err();
        assert_eq!(
            message,
            "The `-n`/`--repo-name` name matches more than one repo: `dup` matches `one/dup`, `two/dup`.\n\
             Select one of them with `-p`/`--path`, or give it a distinct `alias` in its configuration.\n\
             \n\
             A repo's name is its `alias` if configured, else its `name`, else the last component of its directory."
        );

        // Every problem is reported at once.
        let message = super::select_by_name(&names(&["x", "dup", "y", "a"]), &named).unwrap_err();
        assert!(
            message.contains("names `x`, `y` do not match any repo"),
            "{message}"
        );
        assert!(
            message.contains("`dup` matches `one/dup`, `two/dup`"),
            "{message}"
        );

        // The known names are capped.
        let many = (0..7)
            .map(|i| (format!("r{i}"), RelativePath::new("x")))
            .collect::<Vec<_>>();
        let message = super::select_by_name(&names(&["nope"]), &many).unwrap_err();
        assert!(
            message.contains("`r0`, `r1`, `r2`, `r3`, `r4`, and 2 more."),
            "{message}"
        );

        let message = super::select_by_name::<&str>(&names(&["nope"]), &[]).unwrap_err();
        assert!(message.contains("No repos are configured."), "{message}");
    }

    #[test]
    fn selection_names() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("project");
        fs::create_dir_all(root.join("repos/plain")).unwrap();
        fs::create_dir_all(root.join("repos/named")).unwrap();
        fs::create_dir_all(root.join("repos/aliased")).unwrap();
        fs::create_dir_all(root.join("repos/own")).unwrap();

        fs::write(
            root.join("Kick.toml"),
            "[repo.\"repos/plain\"]\nurl = \"https://example.com/plain\"\n\
             [repo.\"repos/named\"]\nurl = \"https://example.com/named\"\nname = \"crate-name\"\n\
             [repo.\"repos/aliased\"]\nurl = \"https://example.com/aliased\"\nname = \"crate\"\nalias = \"short\"\n\
             [repo.\"repos/own\"]\nurl = \"https://example.com/own\"\nname = \"outer\"\n",
        )
        .unwrap();

        // A repo's own Kick.toml is more specific than the project's.
        fs::write(root.join("repos/own/Kick.toml"), "alias = \"inner\"\n").unwrap();

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        let paths = Paths {
            root: &root,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let loaded = config::load_all(paths, &templating, &defaults);
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);

        let name = |path: &str| {
            loaded
                .config
                .selection_name(&root, RelativePath::new(path))
                .map(|n| n.into_owned())
        };

        assert_eq!(name("repos/plain").as_deref(), Some("plain"));
        assert_eq!(name("repos/named").as_deref(), Some("crate-name"));
        assert_eq!(name("repos/aliased").as_deref(), Some("short"));
        assert_eq!(name("repos/own").as_deref(), Some("inner"));
        // `alias` does not change the crate name.
        let aliased = Repo::new(
            [RepoSource::Git],
            "repos/aliased".into(),
            url::Url::parse("https://example.com/aliased").unwrap(),
        );
        assert_eq!(loaded.config.name(&aliased), Some("crate"));
        // The root itself is named after its directory.
        assert_eq!(name("").as_deref(), Some("project"));
    }

    #[test]
    fn global_repos_by_name_from_inside_project() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let home = base.join("home");
        let project = home.join("project");

        fs::create_dir_all(project.join("repos/inner/src")).unwrap();
        fs::create_dir_all(project.join("repos/shared")).unwrap();
        fs::create_dir_all(home.join("src/other")).unwrap();
        fs::create_dir_all(home.join("src/aiocp")).unwrap();
        fs::create_dir_all(home.join("src/shared")).unwrap();
        fs::write(
            project.join("Kick.toml"),
            "[repo.\"repos/inner\"]\nurl = \"https://example.com/inner\"\n\
             [repo.\"repos/shared\"]\nurl = \"https://example.com/shared\"\n",
        )
        .unwrap();

        const GLOBAL: &str = "[repo.\"~/src/other\"]\nurl = \"https://example.com/other\"\n\
             [repo.\"~/src/aiocp\"]\nurl = \"https://example.com/aiocp\"\nname = \"iocp\"\n\
             [repo.\"~/src/shared\"]\nurl = \"https://example.com/shared\"\n";

        let global = global_config(&base, GLOBAL, true);

        let current = project.join("repos/inner/src");
        let (root, _, global_repos) = super::find_root(&current, Some(&home), Some(&global.path));
        assert!(!global_repos);

        let opts = |names: &[&str]| RepoOptions {
            names: super::tests::names(names),
            ..RepoOptions::default()
        };

        let switch = |opts: &RepoOptions| {
            selects_global_repos(&current, &root, Some(opts), &home, &global.path)
        };

        // Names only found among the global repos switch to them.
        assert!(switch(&opts(&["other"])));
        assert!(switch(&opts(&["iocp"])));
        // The directory name is not the name when `name` is set.
        assert!(!switch(&opts(&["aiocp"])));
        // Names in the project stay in the project, even if a global repo has
        // the same name.
        assert!(!switch(&opts(&["inner"])));
        assert!(!switch(&opts(&["shared"])));
        // Names matching nothing stay, and are reported as unknown later.
        assert!(!switch(&opts(&["nope"])));
        assert!(!switch(&opts(&[])));

        // Without global repos nothing changes.
        let empty = global_config(&base, "license = \"MIT\"\n", true);
        assert!(!selects_global_repos(
            &current,
            &root,
            Some(&opts(&["other"])),
            &home,
            &empty.path
        ));
    }

    #[test]
    fn project_root_repo_by_name() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().canonicalize().unwrap();
        let home = base.join("home");
        // A standalone project without `[repo]` sections, so the root is the
        // repo, named after its directory.
        let project = home.join("standalone");
        fs::create_dir_all(&project).unwrap();
        fs::create_dir_all(home.join("src/standalone")).unwrap();
        fs::write(project.join("Kick.toml"), "license = \"MIT\"\n").unwrap();

        let global = global_config(
            &base,
            "[repo.\"~/src/standalone\"]\nurl = \"https://example.com/standalone\"\n",
            true,
        );

        let opts = RepoOptions {
            names: names(&["standalone"]),
            ..RepoOptions::default()
        };

        assert!(!selects_global_repos(
            &project,
            &project,
            Some(&opts),
            &home,
            &global.path
        ));
    }
}
