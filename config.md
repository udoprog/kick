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

Any option defined in the following section can be used either as an
override, or as part of its own repo-specific configuration.

See the following sections for documentation on the various configuration sections.

* [Repository configuration](./config/toplevel.md)
* [Defining re-usable `[variables]`](./config/variables.md)
* [Managing `[workflows]`](./config/workflows.md)
* [Managing `[badges]`](./config/badges.md)
* [Managing GitHub `[actions]`](./config/actions.md)
* [Building packages using `[package]`](./config/package.md)
* [Describing how projects are built using `[build]`](./config/build.md)
* [Deploying projects using `[deploy]`](./config/deploy.md)
* [Installing projects locally using `[install]`](./config/install.md)
* [Keeping version strings up to date with `[version]`](./config/versions.md)
