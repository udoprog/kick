Version replacement in Kick.

Sometimes you want to reference the specific version of the package being
replaced. The `[[version]]` array allows you to define files and patterns that
should be replaced with newly updated version.

Note that replacement will be performed when a version is bumped, and only
patterns which matches the version you previously bumped *from* will be
replaced.

### `[[version]]`

Defines a list of files for which we match a regular expression for version
replacements.

Available fields are:

* `paths` - Array of patterns to match when performing a version replacement.
* `pattern` - A regular expression which performs the replacement. Use the
  `?P<version>` group name to define what is being replaced.

<br>

#### Examples

```toml
[[version]]
paths = ["src/**/*.rs"]
# Replace any version references in crate-level documentation.
pattern = "//!\\s+[a-z-]+\\s*=\\s*.+(?P<version>[0-9]+\\.[0-9]+\\.[0-9]+).+"
```

### `[[version_group]]`

Defines a group of crates in the workspace which share a single version. A
crate can belong to at most one group, and naming a crate which is not part of
the workspace is an error. Crates which have `publish = false` are ignored.

Available fields are:

* `crates` - Array of package names of the crates in the group.

When a crate in a group is selected with `kick version`, every crate in the
group is changed to the same version:

* When bumping (`--major`, `--minor`, `--patch` or `--pre`), the bump is based
  on the highest current version among the members of the group, so a group
  whose versions have drifted apart converges on one version.
* An `--override <crate>=<version>` for a crate in a group applies to the whole
  group. Conflicting overrides for members of the same group are an error.
* Pass `--no-group` to ignore groups and only change the selected crates.

Crates which inherit their version through `version.workspace = true` are
changed by updating `[workspace.package] version`. Since every crate which
inherits the workspace version shares it, they all change together.

`kick check` reports members of a group whose version differs from the highest
version in the group, and with `--save` updates them to match. Members which
inherit their version from the workspace are only reported, since fixing them
means changing `[workspace.package] version`.

<br>

#### Examples

```toml
[[version_group]]
crates = ["foo", "foo-macros", "foo-core"]
```
