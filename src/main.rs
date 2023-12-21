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
//! * `compress` - Compress files.
//! * `msi` - Build MSI packages.
//! * `rpm` - Build RPM packages.
//! * `define` - Define variables from versions.
//!
//! <br>
//!
//! ## The `rpm` action
//!
//! The `rpm` action builds an RPM package for each repo. It is configured with
//! the following section:
//!
//! ```toml
//! [[rpm.files]]
//! source = "desktop/se.tedro.JapaneseDictionary.desktop"
//! dest = "/usr/share/applications/"
//! mode = "600"
//!
//! [[rpm.requires]]
//! package = "tesseract-langpack-jpn"
//! version = ">= 4.1.1"
//! ```
//!
//! Note that:
//! * The default mode for files is inherited from the file.
//! * The default version specification is `*`.
//!
//! Available version specifications are:
//! * `*` - any version.
//! * `= 1.2.3` - exact version.
//! * `> 1.2.3` - greater than version.
//! * `>= 1.2.3` - greater than or equal to version.
//! * `< 1.2.3` - less than version.
//! * `<= 1.2.3` - less than or equal to version.
//!
//! <br>
//!
//! ## The `msi` action
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
//! ## Version selection
//!
//! Kick comes with a flexible version-selection mechanism, this is available
//! for the following actions:
//!
//! * `compress`
//! * `msi`
//! * `rpm`
//! * `define` (see [Defining variables from
//!   versions](#defining-variables-from-versions))
//!
//! The way it works is that the `--version` argument has a very flexible
//! parsing mechanism.
//!
//! The supported formats are:
//! * A version number potentially with a custom prerelease, like `1.2.3-pre1`.
//! * A simple naive date, like `2023-12-11`.
//! * An alphabetical name, like `nightly` which will result in a dated version
//!   number where version numbers are strictly required. A version suffixed
//!   with a number like `nightly1` will be treated as a pre-release.
//! * A date follow by a custom suffix, like `2023-12-11-nightly`.
//! * It is also possible to use a variable like `%date` to get the custom date.
//!   For available variable see below.
//!
//! A version can also take a simple kind of expression, where each candidate is
//! separated from left to right using double pipes ('||'). The first expression
//! for which all variables are defined, and results in a non-empty expansion
//! will be used.
//!
//! This means that with Github Actions, you can uses something like this:
//!
//! ```text
//! --version "${{github.event.inputs.release}} || %date-nightly"
//! ```
//!
//! In this example, the `release` input might be defined by a workflow_dispatch
//! job, and if undefined the version will default to a "nightly" dated release.
//!
//! Available variables:
//! * `%date` - The current date.
//! * `%{github.tag}` - The tag name from GITHUB_REF.
//! * `%{github.head}` - The branch name from GITHUB_REF.
//!
//! You can also define your own variables using `--define <key>=<value>`. If
//! the value is empty, the variable will be considered undefined.
//!
//! <br>
//!
//! ## Defining variables from versions
//!
//! The `define` command can be used to define a variable in a Github action to
//! extract the version being selected:
//!
//! ```yaml
//! - uses: udoprog/kick@nightly
//! - run: kick define --version "${{github.event.inputs.channel}} || %date" --github-action
//!   id: release
//! # echo the selected version
//! - run: echo ${{steps.release.outputs.version}}
//! # echo "yes" or "no" depending on if the version is a pre-release or not.
//! - run: echo ${{steps.release.outputs.pre}}
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
//! ## Working with repo sets
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
//! ## Configuration
//!
//! Configuration for kick is stores in a `Kick.toml` file. Whenever you run the
//! command it will look recursively for the `Kick.toml` that is in the
//! shallowest possible filesystem location.
//!
//! Configuration is loaded in a hierarchy, and each option can be extended or
//! overriden on a per-repo basis. This is usually done through a
//! `[repo."<name>"]` section.
//!
//! ```toml
//! [repo."repos/OxidizeBot"]
//! crate = "oxidize"
//!
//! [repo."repos/OxidizeBot".upgrade]
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
//! ### `crate`
//!
//! Overrides the detected crate name.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! [repo."repos/OxidizeBot"]
//! crate = "oxidize"
//! ```
//!
//! <br>
//!
//! ### `variables`
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
//! ### `job_name`
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
//! ### `workflow`
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
//! ### `license`
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
//! ### `authors`
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
//! ### `documentation`
//!
//! Defines the documentation link to use.
//!
//! <br>
//!
//! #### Examples
//!
//! ```toml
//! documentation = "{{docs_rs}}/{{package.name}}"
//! ```
//!
//! <br>
//!
//! ### `readme_badges`
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
//! ### `disabled`
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
//! src = "https://img.shields.io/badge/github-{{dash_escape package.repo}}-{{colors.github}}?style=for-the-badge&logo=github"
//! href = "{{github}}/{{package.repo}}"
//! height = "{{badge_height}}"
//!
//! [[badges]]
//! id = "crates.io"
//! alt = "crates.io"
//! src = "https://img.shields.io/crates/v/{{package.name}}.svg?style=for-the-badge&color={{colors.crates_io}}&logo=rust"
//! href = "https://crates.io/crates/{{package.name}}"
//! height = "{{badge_height}}"
//!
//! [[badges]]
//! id = "docs.rs"
//! alt = "docs.rs"
//! src = "https://img.shields.io/badge/docs.rs-{{dash_escape package.name}}-{{colors.docs_rs}}?style=for-the-badge&logoColor=white&logo={{docs_rs_image}}"
//! href = "{{docs_rs}}/{{package.name}}"
//! height = "{{badge_height}}"
//!
//! [[badges]]
//! id = "build"
//! alt = "build status"
//! src = "https://img.shields.io/github/actions/workflow/status/{{package.repo}}/ci.yml?branch=main&style=for-the-badge"
//! href = "{{github}}/{{package.repo}}/actions?query=branch%3Amain"
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
//! ### `lib_badges` and `readme_badges`
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
//! ### `lib` and `readme`
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
//! [repo."repos/rune"]
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
//! * `package.name` the name of the package.
//! * `package.repo.owner` the owner of the repository, as in `<owner>/<name>`
//!   (optional).
//! * `package.repo.name` the name of the repository, as in `<owner>/<name>`
//!   (optional).
//! * `package.description` the repo-specific description from its primary
//!   `Cargo.toml` manifest (optional).
//! * `package.rust_version` the rust-version read from its primary `Cargo.toml`
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
//! {{#if package.rust_version}}
//! Minimum support: Rust <b>{{package.rust_version}}+</b>.
//! <br>
//! {{/if}}
//! <br>
//! <a href="https://rune-rs.github.io"><b>Visit the site 🌐</b></a>
//! &mdash;
//! <a href="https://rune-rs.github.io/book/"><b>Read the book 📖</b></a>
//! <br>
//! <br>
//!
//! {{package.description}}
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
//! # {{package.name}}
//!
//! {{#each badges}}
//! {{literal this.html}}
//! {{/each}}
//! <br>
//! <br>
//!
//! {{package.description}}
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
//! ### `[[version]]`
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
//! # replaces versions in repo level comments that looks likle [dependencies] declarations
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
//! Generates a `README.md` based on what's in the top-level comments of your
//! primary crate's entrypoint. See the `readme` template above.
//!
//! ```rust,no_run
//! //! This is my crate!
//! //!
//! //! ```
//! //! let a = 42;
//! //! ```
//! ```
//!
//! Would become the following `README.md`:
//!
//! ````text
//! This is my crate!
//!
//! ```rust
//! let a = 42;
//! ```
//! ````
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
mod octokit;
mod process;
mod release;
mod repo_sets;
mod rust_version;
mod templates;
mod urls;
mod wix;
mod workspace;

use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, FromArgMatches, Parser, Subcommand};

use actions::Actions;
use relative_path::{RelativePath, RelativePathBuf};
use tracing::metadata::LevelFilter;

use crate::{glob::Fragment, model::Repo};

/// Name of project configuration files.
const KICK_TOML: &str = "Kick.toml";
/// User agent to use for http requests.
static USER_AGENT: reqwest::header::HeaderValue =
    reqwest::header::HeaderValue::from_static("kick/0.0");

#[derive(Subcommand)]
enum Action {
    /// Collect and define release variables.
    Define(SharedAction<cli::define::Opts>),
    /// Apply staged changes which have previously been saved by `check` unless
    /// `--save` was specified.
    Changes(SharedOptions),
    /// Manage sets.
    Set(SharedAction<cli::set::Opts>),
    /// Checks each repo (default action).
    Check(SharedAction<cli::check::Opts>),
    /// Run a custom command for each repo.
    For(SharedAction<cli::r#for::Opts>),
    /// Fetch github actions build status for each repo.
    Status(SharedAction<cli::status::Opts>),
    /// Find the minimum supported rust version for each repo.
    Msrv(SharedAction<cli::msrv::Opts>),
    /// Update package version.
    Version(SharedAction<cli::version::Opts>),
    /// Publish packages in reverse order of dependencies.
    Publish(SharedAction<cli::publish::Opts>),
    /// Perform repo-aware `cargo upgrade`.
    Upgrade(SharedAction<cli::upgrade::Opts>),
    /// Build a wix-based installer.
    Msi(SharedAction<cli::msi::Opts>),
    /// Build an rpjm-based installer.
    Rpm(SharedAction<cli::rpm::Opts>),
    /// Build a compressed artifact (like a zip or tar.gz).
    Compress(SharedAction<cli::compress::Opts>),
    /// Build a github release.
    GithubRelease(SharedAction<cli::github_release::Opts>),
}

impl Action {
    fn shared(&self) -> &SharedOptions {
        match self {
            Action::Define(action) => &action.shared,
            Action::Changes(shared) => shared,
            Action::Set(action) => &action.shared,
            Action::Check(action) => &action.shared,
            Action::For(action) => &action.shared,
            Action::Status(action) => &action.shared,
            Action::Msrv(action) => &action.shared,
            Action::Version(action) => &action.shared,
            Action::Publish(action) => &action.shared,
            Action::Upgrade(action) => &action.shared,
            Action::Msi(action) => &action.shared,
            Action::Rpm(action) => &action.shared,
            Action::Compress(action) => &action.shared,
            Action::GithubRelease(action) => &action.shared,
        }
    }

    fn repo(&self) -> Option<&RepoOptions> {
        match self {
            Action::Define(..) => None,
            Action::Changes(..) => None,
            Action::Set(action) => Some(&action.repo),
            Action::Check(action) => Some(&action.repo),
            Action::For(action) => Some(&action.repo),
            Action::Status(action) => Some(&action.repo),
            Action::Msrv(action) => Some(&action.repo),
            Action::Version(action) => Some(&action.repo),
            Action::Publish(action) => Some(&action.repo),
            Action::Upgrade(action) => Some(&action.repo),
            Action::Msi(action) => Some(&action.repo),
            Action::Rpm(action) => Some(&action.repo),
            Action::Compress(action) => Some(&action.repo),
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
    /// Test if the repository is outdated.
    #[arg(long)]
    outdated: bool,
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

#[derive(Default, Parser)]
#[command(author, version, about, long_about = None)]
struct Opts {
    /// Action to perform. Defaults to `check`.
    #[command(subcommand, name = "action")]
    action: Option<Action>,
}

#[tokio::main]
async fn main() -> Result<ExitCode> {
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

async fn entry() -> Result<ExitCode> {
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

    tracing::trace!(
        root = root.display().to_string(),
        ?current_path,
        "Using project root"
    );

    let changes_path = root.join("changes.gz");

    if let Action::Define(opts) = &action {
        cli::define::entry(&opts.action)?;
        return Ok(ExitCode::SUCCESS);
    };

    let github_auth = root.join(".github-auth");

    let github_auth = match fs::read_to_string(&github_auth) {
        Ok(auth) => Some(auth.trim().to_owned()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::trace!("no .github-auth found, heavy rate limiting will apply");
            None
        }
        Err(e) => {
            return Err(anyhow::Error::from(e)).with_context(|| github_auth.display().to_string())
        }
    };

    let git = git::Git::find()?;

    let templating = templates::Templating::new()?;
    let repos = model::load_gitmodules(&root)?;

    let defaults = config::defaults();
    let config = config::load(
        &root,
        &templating,
        repos.as_deref().unwrap_or_default(),
        &defaults,
    )?;

    let repos = match repos {
        Some(repos) => repos,
        None => model::load_from_git(&root, git.as_ref())?,
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

    if let Some(repo_opts) = repo_opts {
        let current_path = if let Some(current_path) = current_path.as_ref() {
            if !repo_opts.all && repos.iter().any(|m| current_path.starts_with(m.path())) {
                Some(current_path.as_ref())
            } else {
                None
            }
        } else {
            None
        };

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

        filter_repos(
            &root,
            repo_opts,
            git.as_ref(),
            &repos,
            &filters,
            current_path,
            set.as_ref(),
        )?;
    }

    let mut cx = ctxt::Ctxt {
        root: &root,
        current_path: current_path.as_deref(),
        config: &config,
        actions: &actions,
        repos: &repos,
        github_auth,
        rustc_version: ctxt::rustc_version(),
        git,
        warnings: RefCell::new(Vec::new()),
        changes: RefCell::new(Vec::new()),
        sets: &mut sets,
    };

    match &action {
        Action::Changes(shared) => {
            cli::changes::entry(&mut cx, shared, &changes_path)?;
            return Ok(ExitCode::SUCCESS);
        }
        Action::Set(opts) => {
            cli::set::entry(&mut cx, &opts.action)?;
        }
        Action::Check(opts) => {
            cli::check::entry(&mut cx, &opts.action).await?;
        }
        Action::For(opts) => {
            cli::r#for::entry(&mut cx, &opts.action)?;
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
        Action::Compress(opts) => {
            cli::compress::entry(&mut cx, &opts.action)?;
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
    root: &Path,
    repo_opts: &RepoOptions,
    git: Option<&git::Git>,
    repos: &[model::Repo],
    filters: &[Fragment<'_>],
    current_path: Option<&RelativePath>,
    set: Option<&HashSet<RelativePathBuf>>,
) -> Result<()> {
    // Test if repo should be skipped.
    let should_disable = |repo: &Repo| -> bool {
        if let Some(set) = set {
            if !set.contains(repo.path()) {
                return true;
            }
        }

        if filters.is_empty() {
            if let Some(path) = current_path {
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

        if repo_opts.needs_git() {
            let git = git.context("no working git command found")?;
            let repo_path = repo.path().to_path(root);

            let cached = git.is_cached(&repo_path)?;
            let dirty = git.is_dirty(&repo_path)?;

            let span = tracing::trace_span!("git", ?cached, ?dirty, repo = repo.path().to_string());
            let _enter = span.enter();

            if repo_opts.outdated && !git.is_outdated(&repo_path)? {
                tracing::trace!("Directory is not outdated");
                repo.disable();
            }

            if repo_opts.dirty && !dirty {
                tracing::trace!("Directory is not dirty");
                repo.disable();
            }

            if repo_opts.cached && !cached {
                tracing::trace!("Directory has no cached changes");
                repo.disable();
            }

            if repo_opts.cached_only && (!cached || dirty) {
                tracing::trace!("Directory has no cached changes");
                repo.disable();
            }

            if repo_opts.unreleased {
                if let Some((tag, offset)) = git.describe_tags(&repo_path)? {
                    if offset.is_none() {
                        tracing::trace!("No offset detected (tag: {tag})");
                        repo.disable();
                    }
                } else {
                    tracing::trace!("No tags to describe");
                    repo.disable();
                }
            }
        }
    }

    Ok(())
}

/// Find root path to use.
fn find_from_current_dir(current_dir: &Path) -> Option<(PathBuf, RelativePathBuf)> {
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
                first_git = Some((path.clone(), relative.iter().rev().collect()));
            }
        }

        let kick_toml = parent.join(KICK_TOML);

        if kick_toml.is_file() {
            tracing::trace!("Found {KICK_TOML} in {}", kick_toml.display());
            last_kick_toml = Some((path.clone(), relative.iter().rev().collect()));
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

    let Some((first_git, relative_path)) = first_git else {
        return None;
    };

    Some((first_git, relative_path))
}
