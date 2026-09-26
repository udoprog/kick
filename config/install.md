Install configuration.

The `[install]` section configures the `kick install` action, which installs a
project locally by running a sequence of commands in the root of the
repository.

Commands are run in the order they are specified, and the first one which fails
stops the install of that repository. Each command is printed before it is run.

<br>

### No configuration needed

A Cargo project can be installed without any configuration:

```sh
kick install
```

Which runs `cargo install --path .`. If `cargo_toml` points to a manifest in a
subdirectory, the install uses that directory instead. A project which is not a
Cargo project and doesn't configure any commands is an error.

<br>

### `[install]` section

* `commands` a list of commands to run in order to install the project. When
  this is specified it replaces the default `cargo install --path .` entirely.

Commands are either a string, which is split on whitespace, or a list of
arguments in case an argument contains whitespace. This is the same form used
by `pre_build` and `build` in the [`[deploy]` section](./deploy.md#building).
Note that these are *not* run through a shell, so shell syntax such as `&&` or
`$VAR` is not available.

Like other sections, `[install]` can be specified in more than one
configuration layer, in which case the commands of each layer are appended in
order.

<br>

#### Examples

A project with a web frontend that needs to be built before the binary which
bundles it is installed:

```toml
[install]
commands = [
    "trunk build --release",
    ["cargo", "install", "--path", ".", "--locked"],
]
```

<br>

### Options

* `--dry-run` prints the commands which would be run instead of running them.
* `--command <command>`, or `-c <command>`, adds a command to run after the
  configured ones. It is split on whitespace, and can be used more than once.
  Commands added this way are also run after the default `cargo install --path
  .` if nothing is configured.

As with other actions, which repositories are installed is controlled by the
usual repository selection options.
