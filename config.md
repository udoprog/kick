## Configuring Kick

Configuration for kick is stores in a `Kick.toml` file. Whenever you run the
command it will look recursively for the `Kick.toml` that is in the shallowest
possible filesystem location.

Configuration is loaded in a hierarchy, and each option can be extended or
overriden on a per-repo basis. This is usually done through a `[repo."<name>"]`
section.

Run `kick inspect` to see the hierarchy kick loaded from the current directory:
every `Kick.toml` in load order and what each one sets, load errors, the repos
and whether they are selected, and the effective configuration of the selected
repos. Use `kick inspect --json` for a machine-readable report.

### Global configuration

A user-global `Kick.toml` in kick's user configuration directory
(`~/.config/kick/Kick.toml` on Linux) is loaded after the project hierarchy as
the least specific layer, so project and repo configuration takes precedence
over it. Path-valued settings and `[repo."<path>"]` keys in it may be absolute,
start with `~/`, or be relative to the home directory, and environment
variables are interpolated in them (see [Paths and environment
variables](#paths-and-environment-variables)).

Outside of a project (no `Kick.toml` or git checkout in the current directory
or its parents) kick acts on the repos declared in the global configuration.
Inside of a project its `[repo]` sections are ignored. Use `kick project add`,
`kick project list` and `kick project remove` to manage them.

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
`repos/OxidizeBot/Kick.toml`, but this is usually not desirable since you might
not want to contaminate the project folder with a random file nobody knows what
it is.

```toml
# repos/OxidizeBot/Kick.toml
crate = "oxidize"

[upgrade]
exclude = [
   # We avoid touching this dependency since it has a complex set of version-dependent feature flags.
   "libsqlite3-sys"
]
```

### Paths and environment variables

Path-valued settings, such as `[repo."<path>"]` keys, template paths (`lib`,
`readme`, workflow and systemd unit `template`s), `cargo_toml`, `[[version]]`
paths and `[package]` and `[deploy]` file sources, are interpolated when the
configuration is loaded:

* A leading `~` (the whole value, or `~/` at its start) expands to your home
  directory. `~user` is not supported, and a `~` anywhere else, or one that
  comes from the value of a variable, is kept as-is.
* `$VAR` and `${VAR}` expand to the value of the environment variable `VAR`. A
  name is made up of letters, digits and `_`, and does not start with a digit.
* `${VAR:-default}` expands to `default` if `VAR` is unset or empty. The default
  may itself contain `$OTHER` or `${OTHER:-x}`, which are only expanded if the
  default is used. A `~` in a default is not expanded, use `$HOME` instead.
* `$$` is a literal `$`. Any other `$` is an error.

Referencing a variable which is not set without a `:-` fallback is an error
which names the variable, the key and the file it is in. It never silently
expands to an empty string. A variable which is set to an empty string expands
to an empty string.

```toml
lib = "${KICK_TEMPLATES:-$HOME/.config/kick/templates}/lib.md"

[repo."$WORK/OxidizeBot"]
url = "https://github.com/udoprog/OxidizeBot"

[repo."~/src/kick"]
url = "https://github.com/udoprog/kick"
cargo_toml = "costs-$$5/Cargo.toml"  # a directory named `costs-$5`
```

In a project `Kick.toml`, template paths and `[repo."<path>"]` keys may be
absolute or start with `~`, and otherwise are relative to the directory of the
`Kick.toml`. Other paths, like `cargo_toml`, must still be relative to the repo
once interpolated. In the global configuration relative paths are resolved
against the home directory. The `dest` of a `[deploy]` file is a path on the
remote host, so it is not interpolated.

Values are interpolated when they are loaded, not when they are written:
`kick project add` stores keys as `~/...`, and keys you write by hand are kept
as you wrote them.

Any option defined in the following section can be used either as an
override, or as part of its own repo-specific configuration.

See the following sections for documentation on the various configuration sections.

* [Repository configuration](./config/toplevel.md)
* [Defining re-usable `[variables]`](./config/variables.md)
* [Template syntax used in `Kick.toml`, unit templates and archive names](./config/templates.md)
* [Managing `[workflows]`](./config/workflows.md)
* [Managing `[badges]`](./config/badges.md)
* [Managing GitHub `[actions]`](./config/actions.md)
* [Building packages using `[package]`](./config/package.md)
* [Describing how projects are built using `[build]`](./config/build.md)
* [Deploying projects using `[deploy]`](./config/deploy.md)
* [Installing projects locally using `[install]`](./config/install.md)
* [Keeping version strings up to date with `[version]`](./config/versions.md)
* [Versioning crates together with `[version_group]`](./config/versions.md#version_group)
