## Configuring Kick

Configuration for kick is stores in a `Kick.toml` file. Whenever you run the
command it will look recursively for the `Kick.toml` that is in the
shallowest possible filesystem location.

Configuration is loaded in a hierarchy, and each option can be extended or
overriden on a per-repo basis. This is usually done through a
`[repo."<name>"]` section.

```toml
[repo."repos/OxidizeBot"]
crate = "oxidize"

[repo."repos/OxidizeBot".upgrade]
exclude = [
   # We avoid touching this dependency since it has a complex set of version-dependent feature flags.
   "libsqlite3-sys"
]
```

The equivalent would be to put the following inside of
`repos/OxidizeBot/Kick.toml`, but this is usually not desirable since you
might not want to contaminate the project folder with a random file nobody
knows what it is.

```toml
# repos/OxidizeBot/Kick.toml
crate = "oxidize"

[upgrade]
exclude = [
   # We avoid touching this dependency since it has a complex set of version-dependent feature flags.
   "libsqlite3-sys"
]
```

Any option defined in the following section can be used either as an
override, or as part of its own repo-specific configuration.

<br>

### `crate`

Overrides the detected crate name.

<br>

#### Examples

```toml
[repo."repos/OxidizeBot"]
crate = "oxidize"
```

<br>

### `variables`

Defines an arbitrary collection of extra variables that can be used in templates.

These are overriden on a per-repo basis in the following ways:

* Arrays are extended.
* Maps are extended, so conflicting keys are overriden.

<br>

#### Examples

```toml
[variables]
github = "https://github.com"
docs_rs = "https://docs.rs"
docs_rs_image = "data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K"
colors = { github = "8da0cb", crates_io = "fc8d62", docs_rs = "66c2a5" }
badge_height = 20
```

<br>

### `job_name`

Defines the name of the default workflow, this can be used to link to in
badges later and is also validated by the `ci` module.

<br>

#### Examples

```toml
job_name = "CI"
```

<br>

### `workflow`

Defines the default workflow template to use when a workflow is missing.

<br>

#### Examples

```toml
workflow = "data/workflow.yml"
```

<br>

### `license`

Defines the license for the current repo.

<br>

#### Examples

```toml
license = "MIT/Apache-2.0"
```

<br>

### `authors`

Defines a list of authors that should be present wherever appropriate, such
as in a `Cargo.toml`.

Per-repo options will extend this list.

<br>

#### Examples

```toml
authors = ["John-John Tedro <udoprog@tedro.se>"]
```

<br>

### `documentation`

Defines the documentation link to use.

<br>

#### Examples

```toml
documentation = "{{docs_rs}}/{{package.name}}"
```

<br>

### `readme_badges`

Defines a set of badges to use for readmes.

<br>

#### Examples

```toml
readme_badges = ["+build"]
```

<br>

### `disabled`

A list of kick modules to disable.

```toml
disabled = ["ci"]
```

<br>

## badges

Defines a list of *available* badges to use, with the following options:

* `id` the identifier of the badge, used in `lib_badges` or `readme_badges`
  (see below).
* `alt` the alt text of the badge.
* `src` the source image of the badge.
* `href` the link of the badge.
* `height` the height of the badge.
* `enabled` whether or not the badge should be enabled by default. This can
  be overrided in `lib_badges` and `readme_badges` (see below).

<br>

#### Examples

```toml
[[badges]]
alt = "github"
src = "https://img.shields.io/badge/github-{{dash_escape package.repo}}-{{colors.github}}?style=for-the-badge&logo=github"
href = "{{github}}/{{package.repo}}"
height = "{{badge_height}}"

[[badges]]
id = "crates.io"
alt = "crates.io"
src = "https://img.shields.io/crates/v/{{package.name}}.svg?style=for-the-badge&color={{colors.crates_io}}&logo=rust"
href = "https://crates.io/crates/{{package.name}}"
height = "{{badge_height}}"

[[badges]]
id = "docs.rs"
alt = "docs.rs"
src = "https://img.shields.io/badge/docs.rs-{{dash_escape package.name}}-{{colors.docs_rs}}?style=for-the-badge&logoColor=white&logo={{docs_rs_image}}"
href = "{{docs_rs}}/{{package.name}}"
height = "{{badge_height}}"

[[badges]]
id = "build"
alt = "build status"
src = "https://img.shields.io/github/actions/workflow/status/{{package.repo}}/ci.yml?branch=main&style=for-the-badge"
href = "{{github}}/{{package.repo}}/actions?query=branch%3Amain"
height = "{{badge_height}}"
enabled = false

[[badges]]
id = "discord"
alt = "chat on discord"
src = "https://img.shields.io/discord/558644981137670144.svg?logo=discord&style=flat-square"
href = "https://discord.gg/v5AeNkT"
height = "{{badge_height}}"
enabled = false
```

<br>

### `lib_badges` and `readme_badges`

Defines set of badges to use, either for the `lib` or `readme` file (see
below).

Badges are either included by prefixing them with a `+`, or excluded by
prefixing them with a `-`. This overrides their default option (see above).

<br>

#### Examples

```toml
lib_badges = ["-docs.rs", "-crates.io", "+discord"]
readme_badges = ["-docs.rs", "-crates.io", "+discord"]
```

<br>

### `lib` and `readme`

Path to templates:
* `lib` generates the documentation header for `main.rs` or `lib.rs` entrypoints.
* `readme` generated the `README.md` file.

<br>

#### Examples

```toml
[repo."repos/rune"]
lib = "data/rune.lib.md"
readme = "data/rune.readme.md"
```

The following variables are available for expansion:

* `body` the rest of the comment, which does not include the generated
  header.
* `badges` a list of badges, each `{html: string, markdown: string}`.
* `header_marker` a header marker that can be used to indicate the end of a
  generated documentation header, this will only be set if there's a
  trailing documentation section in use (optional).
* `job_name` the name of the CI job, as specified in the `job_name` setting.
* `rust_versions.rustc` the rust version detected by running `rustc
  --version` (optional).
* `rust_versions.edition_2018` the rust version that corresponds to the 2018
  edition.
* `rust_versions.edition_2021` the rust version that corresponds to the 2021
  edition.
* `package.name` the name of the package.
* `package.repo.owner` the owner of the repository, as in `<owner>/<name>`
  (optional).
* `package.repo.name` the name of the repository, as in `<owner>/<name>`
  (optional).
* `package.description` the repo-specific description from its primary
  `Cargo.toml` manifest (optional).
* `package.rust_version` the rust-version read from its primary `Cargo.toml`
  manifest (optional).

The following is an example `lib` template stored in `data/rune.lib.md`,
note that the optional `header_marker` is used here in case there is a
trailing comment in use.

```md
<img alt="rune logo" src="https://raw.githubusercontent.com/rune-rs/rune/main/assets/icon.png" />
<br>
{{#each badges}}
{{literal this.html}}
{{/each}}
<br>
{{#if package.rust_version}}
Minimum support: Rust <b>{{package.rust_version}}+</b>.
<br>
{{/if}}
<br>
<a href="https://rune-rs.github.io"><b>Visit the site 🌐</b></a>
&mdash;
<a href="https://rune-rs.github.io/book/"><b>Read the book 📖</b></a>
<br>
<br>

{{package.description}}
{{#if body}}

<br>

{{literal header_marker~}}
{{literal body}}
{{/if}}
```

The following is an example `readme` template stored in `data/rune.readme.md`:

```md
<img alt="rune logo" src="https://raw.githubusercontent.com/rune-rs/rune/main/assets/icon.png" />
<br>
<a href="https://rune-rs.github.io"><b>Visit the site 🌐</b></a>
&mdash;
<a href="https://rune-rs.github.io/book/"><b>Read the book 📖</b></a>

# {{package.name}}

{{#each badges}}
{{literal this.html}}
{{/each}}
<br>
<br>

{{package.description}}
{{#if body}}

<br>

{{literal body}}
{{/if}}
```

<br>

### `[[version]]`

Defines a list of files for which we match a regular expression for version replacements.

Available keys are:
* `[[version]].paths` - list of patterns to match when performing a version
  replacement.
* `[[version]].pattern` - the regular expression which performs the
  replacement. Use the `?P<version>` group name to define what is being
  replaced.

<br>

#### Examples

```toml
[[version]]
paths = ["src/**/*.rs"]
# replaces versions in repo level comments that looks likle [dependencies] declarations
pattern = "//!\\s+[a-z-]+\\s*=\\s*.+(?P<version>[0-9]+\\.[0-9]+\\.[0-9]+).+"
```

<br>

## Modules

Modules provide optional functionality that can be disabled on a per-repo
basis through the `disabled` option above.

<br>

### `ci` module

This looks for the github workflow configuration matching `{{job_name}}` and
performs some basic validation over them, such as:

* Making sure there is a build for your `rust-version` (if defined in
  `Cargo.toml`).
* Updates `rust-version` if it mismatches with the build.
* Rejects outdates actions, such as `actions-rs/cargo` in favor of simple
  `run` commands (exact list can't be configured yet).

To disable, specify:

```toml
disabled = ["ci"]
```

<br>

### `readme` module

Generates a `README.md` based on what's in the top-level comments of your
primary crate's entrypoint. See the `readme` template above.

```rust
//! This is my crate!
//!
//! ```
//! let a = 42;
//! ```
```

Would become the following `README.md`:

````text
This is my crate!

```rust
let a = 42;
```
````

To disable, specify:

```toml
disabled = ["readme"]
```
