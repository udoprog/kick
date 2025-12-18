# kick

[<img alt="github" src="https://img.shields.io/badge/github-udoprog/kick-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/kick)
[<img alt="crates.io" src="https://img.shields.io/crates/v/kick.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/kick)
[<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-kick-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/kick)
[<img alt="build status" src="https://img.shields.io/github/actions/workflow/status/udoprog/kick/ci.yml?branch=main&style=for-the-badge" height="20">](https://github.com/udoprog/kick/actions?query=branch%3Amain)

Give your projects a good 🦶!

This is what I'd like to call an omnibus project management tool. I'm
building it to do everything I need when managing my own projects to ensure
that they all have a valid configuration, up-to-date dependencies and a
consistent style.

Even though it's a bit opinionated `kick` also happens to be highly
configurable, so you might want to try it out!

<br>

## Overview

This is an overview of the sections in the README:

* [The `Kick.toml` configuration][config]
* [Tour of commands](#tour-of-commands)
* [Run Github Workflows locally](#run-github-workflows-locally)
* [Maintaining Github Actions](#github-actions)
* [Staged changes](#staged-changes)
* [Running commands over repo sets](#repo-sets)
* [Easily package your project](#packaging)
* [Flexible version specifications](#version-specification)
* [Integrating with Github Actions](#integrating-with-github-actions)

<br>

## Introduction

Kick can also be used *without* configuration in any standalone repository.
This is really all you need to get started, I frequently make use of `kick`
commands in regular repositories. The only pre-requisite is that there is a
`.git` repo with an `origin` specified:

```sh
$> kick check
README.md:
31   > Note that kick uses a nondestructive approach, so running any command like
32   > `kick check` is completely safe. To apply any proposed changes they can
33   > either be reviewed later with `kick changes` or applied directly by
34  -> specifying `--save`.
    +> specifying `--save`. See [Staged changes](#staged-changes) for more.
35
36   The other alternative is to run kick over a collection of repositories. To
37   add a repo to kick you can add the following to a `Kick.toml` file:
2025-12-06T06:50:42.488966Z  INFO kick: Writing to changes.gz, use `kick changes` to review it later
```

> Note that kick uses a nondestructive approach, so running any command like
> `kick check` is completely safe. To apply any proposed changes they can
> either be reviewed later with `kick changes` or applied directly by
> specifying `--save`. See [Staged changes](#staged-changes) for more.

The other alternative is to run kick over a collection of repositories. To
add a repo to kick you can add the following to a `Kick.toml` file:

```toml
[repo."repos/OxidizeBot"]
url = "https://github.com/udoprog/OxidizeBot"
```

This can also be added as a git submodule, note that the important part is
what's in the `.gitmodules` file:

```bash
git submodule add https://github.com/udoprog/OxidizeBot repos/OxidizeBot
```

Once this is done, kick can run any command over a collection of repos:

```sh
$> kick gh status
repos/anything: https://github.com/udoprog/anything
  Workflow `ci` (success):
    git: *2ab2ad7 (main)
    time: 2025-11-28
repos/async-fuse: https://github.com/udoprog/async-fuse
  Workflow `ci` (success):
    git: *4062549 (main)
    time: 2025-11-29
repos/argwerk: https://github.com/udoprog/argwerk
  Workflow `ci` (success):
    git: *4b6377c (main)
    time: 2025-11-27
```

If you want a complete example of this setup, see [my `projects` repo]. For
documentation on how kick can be further configured, see the [configuration
documentation][config].

[my `projects` repo]: https://github.com/udoprog/projects

<br>

## Configuration

Kick optionally reads `Kick.toml`, for how to configure projects. See the
[configuration documentation][config].

<br>

## Tour of commands

This section details some of my favorite things that Kick can do for you.
For a complete list of options, make use of `--help`.

Kick can `check`, which performs a project-specific sanity such as checking
that READMEs are up-to-date with their corresponding sources, badges are
configured, github actions are correctly configured and much more.

Kick can effortlessly package your Rust projects using actions such
`gzip`,`zip`, or packaging systems such as `rpm`, `deb`, or `msi` preparing
them for distribution.

Kick can run custom commands over git modules using convenient filters.
Combined with [repo sets](#repo-sets). Performing batch maintenance over
many git projects has never been easier!
* Want to do something with every project that hasn't been released yet? Try
  `kick run --unreleased`.
* Want to do something with every project that is out-of-sync with their
  remote? Try `kick run --outdated`.

And much much more!

<br>

## Run Github Workflows locally

![Matrix and WSL integration](https://raw.githubusercontent.com/udoprog/kick/main/images/wsl.png)

Kick can run Github workflows locally using `kick run --job <job>`.

This tries to use system utilities which are available locally in order to
run the workflow on the appropriate operating system as specified through
`runs-on`.

This also comes with support for matrix expansion.

Supported integrations are:
* Running on the same operating system as where Kick is run (default).
* Running Linux on Windows through WSL.

<br>

## Maintaining Github Actions

Kick shines the brightest when used in combination with Github Actions. To
facilitate this, the Kick repo can be used in a job directly:

```yaml
jobs:
  build:
  - uses: udoprog/kick@nightly
  - run: kick --version
```

In particular it is useful to specify a global `KICK_VERSION` using the
[wobbly version specification][wobbly-versions] so that all kick commands
that run will use the same version number.

```yaml
# If the `version` input is not available through a `workflow_dispatch`, defaults to a dated release.
env:
  KICK_VERSION: "${{github.event.inputs.version}} || %date"
```

<br>

## Staged changes

If you specify `--save`, proposed changes that can be applied to a project
will be applied. If `--save` is not specified the collection of changes will
be saved to `changes.gz` (in the root) to be applied later using `kick
apply`.

```text
> kick check
repos/kick/README.md: Needs update
repos/kick/src/main.rs: Needs update
2023-04-13T15:05:34.162247Z  WARN kick: Not writing changes since `--save` was not specified
2023-04-13T15:05:34.162252Z  INFO kick: Writing to changes.gz, use `kick changes` to review it later
```

Applying the staged changes:

```text
> kick changes --save
repos/kick/README.md: Fixing
repos/kick/src/main.rs: Fixing
2023-04-13T15:06:23.478579Z  INFO kick: Removing ../changes.gz
```

<br>

## Repo sets

Commands can produce sets under certain circumstances, the sets are usually
called `good` and `bad` depending on the outcome when performing the work
over the repo.

There are a number of special sets you can use:
* `@all` - All repositories.
* `@dirty` - Repositories with uncommitted changes.
* `@outdated` - Repositories that are out-of-date with their remote.
* `@cached` - Repositories with cached changes staged.
* `@unreleased` - Repositories have checked out revisions which do not have
  a corresponding remote tag.

The `--set` parameter also supports simple set operations such as adding
`+`, subtracting `-`, and difference `^`, and intersecting `&` between sets.
Such as selecting all repos which *are not* outdated with `@all -
@outdated`.

If this is set during a run, it will store sets of repos, such as the set
for which a command failed. This set can then later be re-used through the
`--set <id>` switch.

For a list of available sets, you can simply list the `sets` folder:

```text
sets\bad
sets\bad-20230414050517
sets\bad-20230414050928
sets\bad-20230414051046
sets\good-20230414050517
sets\good-20230414050928
sets\good-20230414051046
```

> **Note** the three most recent versions of each set will be retained.

Set files are simply lists of repositories, which supports comments by
prefixing lines with `#`. They are intended to be edited by hand if needed.

```text
repos/kick
# ignore this for now
# repos/unsync
```

<br>

## Packaging

The following actions are packaging actions:
* `zip` - Build .zip archives.
* `gzip` - Build .tar.gz archives.
* `msi` - Build .msi packages using wix.
* `rpm` - Build .rpm packages (builtin method).
* `deb` - Build .deb packages (builtin method).

These all look at the `[package]` section in the configuration to determine
what to include in a given package. For example:

```toml
[[package.files]]
source = "desktop/se.tedro.JapaneseDictionary.desktop"
dest = "/usr/share/applications/"
mode = "600"
```

Note that:
* The default mode for files is 655.
* Where approproate, the default version specification is a wildcard version, or `*`.

When a version specification is used, it supports the following formats:
* `*` - any version.
* `= 1.2.3` - exact version.
* `> 1.2.3` - greater than version.
* `>= 1.2.3` - greater than or equal to version.
* `< 1.2.3` - less than version.
* `<= 1.2.3` - less than or equal to version.

<br>

### `rpm` specific settings

For the `rpm` action, you can specify requires to add to the generated
archive in `Kick.toml`:

```toml
[[package.rpm.requires]]
package = "tesseract-langpack-jpn"
version = ">= 4.1.1"
```

<br>

### `deb` specific settings

For the `deb` action, you can specify dependencies to add to the generated
archive in `Kick.toml`:

```toml
[[package.rpm.depends]]
package = "tesseract-ocr-jpn"
version = ">= 4.1.1"
```

<br>

### The `msi` action

The `msi` action builds an MSI package.

It is configured by a single `wix/<main>.wsx` file in the repo. For an
example, [see the `jpv` project].

When building a wix package, we define the following variables that should
be used:
* `Root` - The root directory of the project. Use this for all files
  referenced.
* `Version` - The version of the package being build in the correct format
  the MSI expects.
* `Platform` - The platform the package is being built for. Either `x86` or
  `x64`. This is simply expected to be passed along to the `Platform`
  attribute in the `Package` element.
* `Win64` - Is either `x86_64` or `x86`. This is simply expected to be
  passed along to any elements with a `Win64` attribute.
* `ProgramFilesFolder` - The directory that corresponds to the
  platform-specific program files folder to use.
* `BinaryName` - The name of the main binary.
* `BinaryPath` - The path to the main binary. Should not be `Root` prefixed.

[see the `jpv` project]: https://github.com/udoprog/jpv/tree/main/wix

<br>

## Version specification

Some actions need to determine a version to use, such as when creating a
github release or building a package.

For these you can:
* Provide the version through the `--version <version>` switch.
* Defining the `KICK_VERSION` environment variable.

This primarily supports plain versions, dates, or tags, such as `1.2.3`,
`2021-01-01`, or `nightly1` and will be coerced as appropriate into a
target version specification depending in which type of package is being
built.

This also supports simple expressions such as `$VALUE || %date` which are
evaluated left-to-right and picks the first non-empty version defined.

For a full specification of the supported format, see the [wobbly version
specification][wobbly-versions].

<br>

## Integrating with Github Actions

Sometimes you want to export information from Kick so that it can be used in
other Github Actions, most commonly this involves the resolved version from
a [version specification](#version-specification).

The `define` command can easily be used to achieve this:

```yaml
# If the `version` input is not available through a `workflow_dispatch`, defaults to a dated release.
env:
  KICK_VERSION: "${{github.event.inputs.version}} || %date"

jobs:
  build:
  - uses: udoprog/kick@nightly
  - run: kick define --github-action
    id: release
  # echo the selected version
  - run: echo ${{steps.release.outputs.version}}
  # echo "yes" or "no" depending on if the version is a pre-release or not.
  - run: echo ${{steps.release.outputs.pre}}
```

Note that version information is exported by default when specifying
`--github-action`. For other information that can be exported, see `define
--help`.

[config]: https://github.com/udoprog/kick/blob/main/config.md
[wobbly-versions]: https://github.com/udoprog/kick/blob/main/WOBBLY_VERSIONS.md
