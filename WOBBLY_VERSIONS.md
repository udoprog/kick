# The wobbly version specification

Kick supports "wobbly version specification", which is a very flexible format
for specifying which version to select when building a package.

In its simplest expression, versions are specified either as plain version
strings, dates, or just tags, such as `1.2.3`, `2023-12-11`, or `nightly1`.

But certain environments might need to pick between multiple different candidate
sources for a version. Such as when using a variable defined in a Github Action
which by default will simply result in an empty string.

To support this well, a wobbly version expresssion is supported which are
version candidates separated by `||`, such
as:

```text
%custom || ${{github.event.inputs.release}} || %date
```

If used in a github action, this will evaluate the first non-empty result. If a
variable is missing (like `%custom` above), it is considered empty. The use of
`${{github.event.inputs.release}}` inside of a Github action is an example of a
variable which externally might evaluate to an empty value. For a
workflow_dispatch job it might be used define like this:

```yaml
on:
  schedule:
  - cron: '0 0 * * *'
  workflow_dispatch:
    inputs:
      version:
        description: 'Version to release'
        required: true
        default: 'nightly'
        type: choice
        options:
        - nightly
        - "%date"

# If the `version` input is not available through a `workflow_dispatch`, defaults to a dated release.
env:
  KICK_VERSION: "${{github.event.inputs.version}} || %date"
  RUST_LOG: kick=trace
```

For the version itself, a number of formats are supported:
 * A version number potentially with custom tags, like `1.2.3-pre1`.
 * A simple naive date with custom tags, like `2023-12-11-pre1`.
 * An alphabetical name, like `nightly` which will result in a dated version
   number where version numbers are strictly required. A version suffixed with a
   number like `nightly1` will be treated as a pre-release.

Note that if a tag itself contains a variable and the variable is missing like
`1.2.3-%custom`, only the tag will be omitted.

By default, the following variables are define:
 * `%date` - The current date.
 * `%{github.tag}` - The tag name from GITHUB_REF if available.
 * `%{github.head}` - The branch name from GITHUB_REF if available.

Finally you can define your own variables using `--define <key>=<value>`. If the
value is empty, the variable will be considered undefined.
