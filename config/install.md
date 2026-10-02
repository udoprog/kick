Install configuration.

The `[install]` section configures the `kick install` action, which installs a
project on the machine `kick` is running on.

An install is the simple case of a [deployment](./deploy.md): the project is
[built](./build.md) as described by the `[build]` section, and the binary is
installed into `~/.cargo/bin`. It runs the same install script as a [local
deployment](./deploy.md#local-deployments) through the local shell, and takes
the same options, so anything a deployment can install, an install can too.
Only the defaults differ.

<br>

### No configuration needed

A Cargo project can be installed without any configuration:

```sh
kick install
```

Which runs:

```sh
cargo build --release
mkdir -p ~/.cargo/bin
install -m 0755 target/release/<binary> ~/.cargo/bin/<binary>
```

The binary is named after the primary crate of the project, see
[`[build]`](./build.md#build-section). `~/.cargo/bin` follows `CARGO_HOME` if it
is set, which is where `cargo install` puts binaries too, so it is already in
your `PATH`.

A project which builds its binary in some other way says so in the `[build]`
section, which is shared with `kick deploy`:

```toml
[build]
binary = "kanban"
pre_build = ["trunk build --release"]
features = ["bundle"]
```

<br>

### `[install]` section

The `[install]` section takes the same options as the
[`[deploy]` section](./deploy.md#deploy-section), except for the ones which only
apply over ssh: `kind`, `host`, `user`, `port`, `identity_file`, `options` and
`staging_dir`. The ones you are most likely to use are:

* `bin_dir` the directory the binary is installed into. Defaults to
  `$CARGO_HOME/bin`, or `~/.cargo/bin`.
* `post_install` and `post_start` [commands](./deploy.md#target-commands) run
  once everything has been installed, such as a command which refreshes
  something with the new binary.
* `files` [extra files](./deploy.md#deployfiles) to install.
* `systemd` a [systemd unit](./deploy.md#systemd) to install. Unlike a
  deployment, an install has no unit unless the section is present, since most
  things which are installed are not services. The unit is a
  [user unit](./deploy.md#user-units) unless `scope` says otherwise.
* `sudo` whether privileged commands use `sudo`, which defaults to `false`.
* `build` a table which is [layered](./build.md#layering) over the `[build]`
  section for installs alone.
* `default_profile` and `profiles` work like
  [deploy profiles](./deploy.md#profiles), and are selected with `--to`.

In addition `[install]` takes:

* `commands` a list of commands which replace building and installing the
  binary. When set, `pre_build` still runs, followed by these commands in the
  root of the repo, and kick doesn't build or install the binary itself. Any
  `files`, unit and `post_install` and `post_start` commands are still
  installed and run afterwards. This is the escape hatch for a project which
  doesn't build a binary with cargo, or which wants to use `cargo install`.

Commands are [configured](./build.md#commands) as a string, a list of arguments
or a table, the same as everywhere else.

Like other sections, `[install]` can be specified in more than one
configuration layer, in which case lists are appended to and everything else is
overridden.

<br>

#### Examples

A project which embeds its web frontend, and refreshes something with the
installed binary afterwards:

```toml
[build]
binary = "kanban"
pre_build = ["trunk build --release"]
features = ["bundle"]

[install]
post_install = ["kanban install"]
```

Which runs:

```sh
trunk build --release
cargo build --release --features bundle
mkdir -p ~/.cargo/bin
install -m 0755 target/release/kanban ~/.cargo/bin/kanban
kanban install
```

Installing into a system directory instead:

```toml
[install]
bin_dir = "/usr/local/bin"
sudo = true
```

Keeping `cargo install`:

```toml
[install]
commands = ["cargo install --path . --locked"]
```

<br>

### Options

`kick install` takes the [build options](./build.md#options), and the
following options which work the same as they do for
[`kick deploy`](./deploy.md#options):

* `--to <profile>` selects the [profile](./deploy.md#profiles) to install.
* `--bin-dir <dir>` overrides `bin_dir`.
* `--no-systemd`, `--no-restart`, `--service-user <user>`, `--group <group>`
  and `--args <args>` for an install with a [systemd unit](./deploy.md#systemd).
* `--dry-run` prints what would be built, installed and run instead of doing
  it. The script is printed in full, and the command which runs it refers to
  it by size, as in `sh -c <local script, 1446 bytes>`.
* `--verbose` / `-V` prints the same while doing it.

As with other actions, which repositories are installed is controlled by the
usual repository selection options.

<br>

### Migrating

The `[install]` section used to be a list of commands which defaulted to
`cargo install --path .`. A project which ran `cargo install` with extra
arguments now describes the build in `[build]` instead, and anything it ran
afterwards goes in `post_install`:

```toml
# Before.
[install]
commands = [
    "trunk build --release",
    ["cargo", "install", "--path", "crates/kanban", "--features", "bundle", "--locked"],
    "kanban install",
]

# After.
[build]
binary = "kanban"
pre_build = ["trunk build --release"]
features = ["bundle"]

[install]
post_install = ["kanban install"]
```

`commands` is still available as an escape hatch, but it no longer replaces
everything: kick still installs any files and unit, and runs `post_install` and
`post_start` afterwards. `--command` / `-c` is gone, use `--pre-build`
instead.
