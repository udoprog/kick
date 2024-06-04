The following section documents toplevel configuration available for use in
`Kick`.

### `name`

Set the name of the primary crate.

By default the name is derived as the last component of the repo it belongs to,
so if an origin matching `https://github.com/udoprog/awesome` the name would be
`awesome`.

This is primarily used to figure out which is the primary crate in a workspace.

<br>

#### Examples

```toml
[repo."repos/OxidizeBot"]
name = "oxidize"
```

<br>

### `lib_badges` and `readme_badges`

Defines set of badges to use, either for the `lib` or `readme` file.

Badges are either included by prefixing them with a `+`, or excluded by
prefixing them with a `-`. This overrides their default option (see above).

The identifier used is the one specified in the [badges](./badges.md)
configuration.

<br>

#### Examples

```toml
[repo."repos/OxidizeBot"]
lib_badges = ["-docs.rs", "-crates.io", "+discord"]
readme_badges = ["-docs.rs", "-crates.io", "+discord"]
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
[repo."repos/OxidizeBot"]
authors = ["John-John Tedro <udoprog@tedro.se>"]
```

<br>

### `documentation`

Defines the documentation link to use.

<br>

#### Examples

```toml
[repo."repos/OxidizeBot"]
documentation = "{{docs_rs}}/{{package.name}}"
```

<br>

### `lib` and `readme`

Path to templates:
* `lib` generates the documentation header for `main.rs` or `lib.rs`
  entrypoints.
* `readme` generated the `README.md` file.

<br>

<br>

#### Examples

```toml
[repo."repos/rune"]
lib = "data/rune.lib.md"
readme = "data/rune.readme.md"
```

The following variables are available for expansion, beyond what's defined in
the [`variables`](./variables.md) section:

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


### `disabled`

A list of kick modules to disable.

#### Examples

```toml
disabled = ["ci"]
```

<br>

## Modules

Modules provide optional functionality that can be disabled on a per-repo basis
through the `disabled` option above.

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
