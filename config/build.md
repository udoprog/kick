Build configuration.

The `[build]` section describes how the project is built, and is shared by
[`kick install`](./install.md) and [`kick deploy`](./deploy.md). Both build the
project in the same way and then install the binary it produced, so how the
binary comes about is only said once.

<br>

### No configuration needed

A Cargo project builds without any configuration with:

```sh
cargo build --release
```

After which `target/release/<binary>` is installed, where the binary is named
after the primary crate of the project. If `cargo_toml` points to a manifest in
a subdirectory, cargo is run in that directory instead. The target directory is
the one cargo reports, so a workspace, `CARGO_TARGET_DIR` and the
`build.target-dir` setting are all taken into account.

<br>

### `[build]` section

* `binary` the name of the binary which is installed. Defaults to `package` if
  that is set, and otherwise to the name of the primary crate of the project,
  which is the crate named after the project or else the package at the root of
  the repo. Set it for a workspace whose binary is in a member crate.
* `package` the cargo package to build, passed as `--package <package>`. By
  default cargo builds whatever it builds from the root, which for a workspace
  are its `default-members`.
* `profile` the cargo profile to build with. Defaults to `release`, which builds
  with `--release`, while any other profile builds with `--profile <profile>`
  and is picked up from the directory cargo puts that profile in.
* `features` a list of features to enable, passed as `--features`.
* `pre_build` a list of [commands](#commands) run before the build, for
  anything cargo doesn't do on its own like building a web frontend which the
  binary embeds.
* `commands` a list of [commands](#commands) which *replace* the generated
  `cargo build`, for when the build is not a plain `cargo build`. `pre_build`
  still runs first, `features` is ignored with a warning, and the binary is
  still picked up from the target directory.

<br>

#### Layering

The `[build]` section is shared, but an install or a deployment can adjust it
with a `build` table of its own, which is layered over it: what it sets wins,
and lists like `features` and `pre_build` are added to. A
[profile](./deploy.md#profiles) can do the same, so the layers are, from least
to most specific:

* `[build]`
* `[install.build]` or `[deploy.build]`
* `[install.profiles.<name>.build]` or `[deploy.profiles.<name>.build]`

As with every other section, `[repo."<name>".build]` overrides it for a single
repo.

```toml
[build]
binary = "track"
pre_build = ["trunk build --release"]
features = ["bundle"]

# A development deployment which builds quickly.
[deploy.profiles.dev.build]
profile = "dev"
```

<br>

#### Commands

Commands are configured in one way everywhere in the `[build]`, `[install]` and
`[deploy]` sections. An entry is either:

* A string, such as `"trunk build --release"`.
* A list of arguments, such as `["sh", "-c", "echo hello"]`, in case an argument
  contains whitespace.
* A table with the command as `command`, in either of the forms above, and
  `sudo = true` to run it under the `sudo` prefix of the install script. This is
  only available for the [commands run on the target](./deploy.md#target-commands).

Where a command runs decides how a string is interpreted:

* Commands which `kick` runs itself, which are `pre_build`, `commands` here and
  [`commands`](./install.md#install-section) in `[install]`, are run in the root
  of the repo and are *not* run through a shell. A string is split on whitespace
  and shell syntax such as `&&` or `$VAR` is not available.
* Commands which are part of the install script, which are `post_install` and
  `post_start`, are command lines for the shell running that script, so a string
  is used exactly as written. A list of arguments is quoted for the shell.

<br>

### Options

Both `kick install` and `kick deploy` take the following options:

* `<binary>` overrides `binary`.
* `--package <package>` overrides `package`.
* `--profile <profile>` overrides `profile`. Note that this is the cargo build
  profile, the install or deploy profile is selected with `--to`.
* `--features <features>` adds to `features`, can be used more than once and
  takes comma-separated lists.
* `--pre-build <command>` adds to `pre_build`, can be used more than once.
* `--no-build` skips building altogether, which is what you want when the
  binary has already been built.

<br>

### Examples

A project whose web frontend is built by `trunk` and embedded in the binary:

```toml
[build]
pre_build = ["trunk build --release"]
features = ["bundle"]
```

Which builds with:

```sh
trunk build --release
cargo build --release --features bundle
```

A workspace whose binary is in `crates/kanban`, built with a custom profile:

```toml
[build]
binary = "kanban"
package = "kanban"
profile = "release-lto"
```

Which builds with `cargo build --profile release-lto --package kanban` and
installs `target/release-lto/kanban`.

Replacing the build command outright:

```toml
[build]
pre_build = ["trunk build --release"]
commands = [["cargo", "xtask", "build", "--dist"]]
```
