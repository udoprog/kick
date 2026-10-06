Template syntax.

Kick renders user-written templates in three places, and they don't all use the
same syntax. This page says which one applies where.

| Where | Engine | Syntax | Undefined variables |
|-|-|-|-|
| Values in `Kick.toml`: [badges], [`lib` and `readme`][lib-readme], [`documentation`][documentation], workflow `template` files, with [variables] | [handlebars] | `{{name}}`, `{{helper arg}}` | Render as empty |
| [systemd unit templates][unit-templates] in `[deploy]` | [minijinja] (jinja2) | `{{ name }}`, `{% if %}`, `{{ x \| filter }}` | Error |
| The archive name given to [`kick compress --name`](#archive-names) (`kick zip`, `kick gzip`) | [minijinja] with single-brace variables | `{name}`, `{% if %}`, `{name \| filter}` | Error |

<br>

## Handlebars

Values in `Kick.toml` which are expanded with the project's [variables] use
[handlebars]. Kick registers two helpers on top of the standard ones, which are
called with helper syntax, as in `{{dash_escape package.repo}}`:

* `dash_escape` replaces every `-` with `--`, which is how shields.io badges
  escape a dash in a badge label.
* `literal` outputs its argument unescaped, like `{{{value}}}`. It is used to
  insert pre-rendered html or markdown such as `{{literal body}}`.

<br>

## Jinja

[systemd unit templates][unit-templates] use [minijinja], so they follow jinja2
syntax with its filters, tests, conditionals and loops. Output is never escaped,
and referring to a variable which hasn't been defined is an error.

<br>

## Archive names

The `--name` option to `kick zip` and `kick gzip` is a [minijinja] template
whose variables use single braces instead of double ones, so that the default
`{project}-{release}-{arch}-{os}` reads as it did before. The following
variables are available:

* `project` the name of the primary package.
* `release` the release version, see `--version` and related options.
* `arch` the architecture, set with `--arch` or defaulting to the one `kick` was
  built for.
* `os` the operating system, set with `--os` or defaulting to the one `kick` was
  built for.

Everything else is jinja: `{os | upper}` applies a filter, blocks are written
`{% ... %}` and comments `{# ... #}`. Since `{` opens a variable, write `{{` to
get a literal `{`. A lone `}` is literal as is. Referring to an undefined
variable, or leaving one unclosed, is an error.

<br>

#### Examples

```sh
kick zip --name "{project}-{release}-{os}"
kick gzip --name "{project}{% if os == 'linux' %}-gnu{% endif %}-{arch}"
```

[badges]: ./badges.md
[documentation]: ./toplevel.md#documentation
[handlebars]: https://handlebarsjs.com/guide/
[lib-readme]: ./toplevel.md#lib-and-readme
[minijinja]: https://docs.rs/minijinja
[unit-templates]: ./deploy.md#templates
[variables]: ./variables.md
