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
