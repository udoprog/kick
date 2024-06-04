The `[[badges]]` array defines all possible badges which can be used by a repo.

The exact set of badges being used is defined in the [`lib_badges` or
  `readme_badges`][lib-readme-badges] repo settings.

[lib-readme-badges]: ./toplevel.md#lib_badges-and-readme_badges

## `[[badges]]`

Defines a list of *available* badges to use, with the following options:

* `id` the identifier of the badge, used in [`lib_badges` or
  `readme_badges`][lib-readme-badges].
* `alt` the alt text of the badge.
* `src` the source image of the badge.
* `href` the link of the badge.
* `height` the height of the badge.
* `enabled` whether or not the badge should be enabled by default. This can be
  overrided in [`lib_badges` and `readme_badges`][lib-readme-badges].

[lib-readme-badges]: ./toplevel.md#lib_badges-and-readme_badges

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
