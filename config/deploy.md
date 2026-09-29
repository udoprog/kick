Deployment configuration.

The `[deploy]` section configures the `kick deploy` action, which is a
lightweight way to deploy a project to a server over ssh, or to the machine
`kick` is running on through a [local deployment](#local-deployments). A project
which is deployed in more than one way defines [profiles](#profiles).

Deploying performs the following steps:

* Every remote host is [checked for access](#access-check) before any work is
  done.
* The project is [built](#building) locally, once regardless of how many hosts
  are being deployed to.
* The binary to deploy is located in `target/<profile>/<binary>`.
* Then, for each host in turn:
  * The binary, any [extra files](#deployfiles) and the rendered
    [systemd unit](#systemd) are uploaded with `scp` to the
    [staging directory](#deploy-section) on the remote host.
  * A single `ssh` invocation stops the service, installs everything into
    place, and starts the service again. The unit is only written and systemd
    only reloaded if the unit actually changed.

Nothing is installed remotely unless the upload succeeded, and the staged files
are removed once they've been installed.

Hosts are deployed to in the order they are listed, and the first one which
fails ends the deployment, so the hosts after it are left as they were.

A [local deployment](#local-deployments) skips the access check and the upload,
and runs the same install script through the local shell instead.

<br>

### No configuration needed

The defaults are picked so that a project which doesn't need anything special
can be deployed with a single command and no `Kick.toml` at all:

```sh
kick deploy --host moore docular
```

This builds the project with `cargo build --release`, installs
`target/release/docular` as `/usr/local/bin/docular`, and installs and starts a
`docular.service` built from the [built-in unit
template](#the-built-in-template).

There is no way to figure out which user a service should run as or what it
should be started with, so a deployment which doesn't run as `root` with no
arguments has to say so:

```sh
kick deploy --host moore docular --service-user docular --args "--bind 0.0.0.0:3004"
```

Which runs the service as `docular:docular`, since `--group` defaults to
`--service-user`. Everything else the unit needs is configured through
[variables](#template-variables) in the `[deploy.systemd]` section.

Note that `--service-user` is the user the *service* runs as. The user the
deployment itself logs in as is `--user`, and the two are rarely the same, since
one needs `sudo` and the other should not:

```sh
kick deploy --host moore --user integration docular --service-user docular
```

Anything else the build needs can be passed along:

```sh
kick deploy --host moore docular --pre-build "trunk build --release" --build-features bundle
```

Which builds with `trunk build --release` followed by
`cargo build --release --features bundle`.

Put the same things in a `[deploy]` section once they stop fitting on a command
line, or when they are a property of the project rather than of one deployment:

```toml
[deploy]
host = "moore"
user = "integration"
pre_build = ["trunk build --release"]
build_features = ["bundle"]
```

Use `kick deploy --dry-run` to print the unit which would be installed and every
command which would be run without changing anything on the remote host, or
`kick deploy --verbose` to see the same information for a deployment which is
actually being performed.

<br>

### `[deploy]` section

The following options are available:

* `kind` how the deployment reaches the machine it installs on, either `ssh`
  (the default) or `local`. See [local deployments](#local-deployments).
* `default_profile` the [profile](#profiles) deployed when none is selected with
  `--to`.
* `profiles` named [profiles](#profiles), as `[deploy.profiles.<name>]`
  sections.
* `host` the host to deploy to, such as `moore`. This is required, but can also
  be specified with `--host <host>`. A list deploys to
  [several hosts](#deploying-to-more-than-one-host) in turn.
* `user` the user to log into the hosts as, such as `integration`. This is the
  user the deployment is performed as, not the user the deployed service runs
  as, which is a [variable](#template-variables) in the `[deploy.systemd]`
  section. Also available as `--user <user>`.
* `port` the port to connect over. Passed to `ssh` with `-p` and to `scp` with
  `-P`.
* `identity_file` the identity file used to authenticate. Passed with `-i`.
* `options` a list of extra options passed to `ssh` and `scp` with `-o`.
* `sudo` whether privileged remote commands are prefixed with `sudo`. Defaults
  to `true` over ssh and `false` for a local deployment, set it to `false` when
  deploying as `root`. See [sudo and interactivity](#sudo-and-interactivity),
  since sudo must not require a password over ssh.
* `bin_dir` the directory the binary is installed into. Defaults to
  `/usr/local/bin`. A leading `~` [expands](#home-directories) to the home
  directory of the user deploying.
* `unit_dir` the directory the systemd unit is installed into. Defaults to
  `/etc/systemd/system`, or `~/.config/systemd/user` for a
  [user unit](#user-units).
* `staging_dir` the remote directory files are uploaded to before they are
  installed. Defaults to `.kick-deploy`, which is relative to the home directory
  of the user being logged in as. Not used by a local deployment.
* `binary` the name of the binary to deploy. Defaults to the name of the primary
  crate in the project, and can be given as an argument to `kick deploy`.
* `profile` the *cargo* build profile the binary is picked up from. Defaults to
  `release`, and can be overridden with `--profile <profile>`. This has nothing
  to do with [deploy profiles](#profiles), which are selected with `--to`.
* `pre_build`, `build` and `build_features` control how the project is
  [built](#building).
* `files` a list of [extra files](#deployfiles) to install.
* `systemd` the [systemd unit](#systemd) to install, along with the
  [variables](#template-variables) it is rendered with. Defaults to the built-in
  unit template.

<br>

### Deploying to more than one host

The `host` option takes a list when a project runs on more than one machine:

```toml
[deploy]
host = ["moore", "dahl", "hilbert"]
user = "integration"
```

The project is built once, and each host is then deployed to in turn with the
same binary and the same set of files. The first host which fails ends the
deployment, so the hosts listed after it are left as they were, and `kick deploy`
exits non-zero.

Every host is [checked for access](#access-check) up front rather than as it is
reached, so a fleet one member of which cannot be logged into says so before the
first host is touched.

Since the unit is rendered per host, the [`host` variable](#template-variables)
refers to the host the unit is being installed on, which is how one template
covers a fleet:

```jinja
Description=track on {{ host }}
```

The same works on the command line, where `--host` can be used more than once:

```sh
kick deploy --host moore --host dahl --user integration track
```

Note that `--host` *replaces* the configured hosts rather than adding to them,
which is what you want when deploying somewhere other than where the project
usually goes.

<br>

#### The login user

The user being logged in as comes from the `user` option, so it doesn't have to
be repeated for every host. It can still be spelled out as part of a host, which
wins over `user` for that host alone:

```toml
[deploy]
host = ["moore", "dahl", "root@legacy"]
user = "integration"
```

Which logs into `moore` and `dahl` as `integration`, and into `legacy` as
`root`.

<br>

### Profiles

A project which is deployed in more than one way, such as to a server and to the
machine you are working on, defines each way as a named profile in a
`[deploy.profiles.<name>]` section. A profile takes every option `[deploy]` does
other than `default_profile` and `profiles`, and is layered over the rest of the
`[deploy]` section: what the profile sets wins, lists like `pre_build` and
`files` are added to, and `[deploy.profiles.<name>.systemd]` overrides
individual options and variables of `[deploy.systemd]`. So `[deploy]` holds what
the profiles share, and each profile what is particular to it:

```toml
[deploy]
binary = "track"
pre_build = ["trunk build --release"]
build_features = ["bundle"]

[deploy.profiles.production]
host = ["moore", "dahl"]
user = "integration"

[deploy.profiles.staging]
host = "hilbert"
user = "integration"

[deploy.profiles.local]
kind = "local"
bin_dir = "~/.local/bin"
systemd = { scope = "user" }
```

The profile being deployed is picked as follows:

* `--to <name>` selects a profile by name.
* Otherwise the `default_profile` option in `[deploy]` is used.
* Otherwise, if only one profile is defined, it is used.
* Otherwise, when running in a terminal, you are asked which profile to deploy.
* Otherwise `kick deploy` fails with an error listing the profiles, since there
  is nobody to ask.

Naming a profile which doesn't exist is an error which lists the ones which do,
and `--dry-run` prints the profile being deployed along with every profile
defined, so `kick deploy --dry-run` is also a way to see which profiles there
are.

A `[deploy]` section without any profiles is deployed as it is, which is how
every deployment worked before profiles existed.

Note that `--to` and `default_profile` refer to deploy profiles, while
`--profile` and the `profile` option are the cargo build profile the binary is
picked up from.

<br>

### Local deployments

A deployment with `kind = "local"` installs on the machine `kick` is running on
rather than over ssh:

* There is no [access check](#access-check), nothing is uploaded, and there is
  no staging directory. The binary, any extra files and the rendered unit are
  installed from where they are.
* The same script which would otherwise run over ssh runs through the local
  shell with `sh -c`, so the service is still stopped, installed, and started
  again in the same way.
* `host`, `user`, `port`, `identity_file`, `options` and `staging_dir` don't
  apply, and are ignored with a warning if the local profile sets them, as are
  `--host`, `--user` and `--no-check`.
* `sudo` defaults to `false`, since installing locally usually means installing
  into your own home directory. When it is enabled, sudo is run without `-n`,
  since there is a terminal to prompt on.

Local deployments pair naturally with a [user unit](#user-units), which needs
neither root nor sudo.

<br>

#### Example: the kanban board

The kanban board runs as a user service on the machine it is developed on. Its
frontend is built by `trunk` and bundled into the binary, which is installed
into `~/.cargo/bin` and started with `kanban serve` from the checkout:

```toml
[deploy]
binary = "kanban"
pre_build = ["trunk build --release"]
build_features = ["bundle"]

[deploy.profiles.local]
kind = "local"
bin_dir = "~/.cargo/bin"

[deploy.profiles.local.systemd]
scope = "user"
description = "Kanban Web UI"
after = "network.target"
working_directory = "~/repo/kanban"
args = ["serve"]
restart = "on-failure"
```

Running `kick deploy --to local`, or plain `kick deploy` since it is the only
profile, builds with:

```sh
trunk build --release
cargo build --release --features bundle
```

And then runs the following through the local shell, with `~` expanded to the
home directory of the user running `kick`:

```sh
set -eu
systemctl --user stop kanban 2>/dev/null || true
mkdir -p ~/.cargo/bin
install -m 0755 <repo>/target/release/kanban ~/.cargo/bin/kanban
mkdir -p ~/.config/systemd/user
if ! cmp -s <rendered unit> ~/.config/systemd/user/kanban.service; then
  install -m 0644 <rendered unit> ~/.config/systemd/user/kanban.service
  systemctl --user daemon-reload
fi
systemctl --user enable kanban
systemctl --user start kanban
```

Which installs the following unit into `~/.config/systemd/user/kanban.service`:

```text
[Unit]
Description=Kanban Web UI
After=network.target

[Service]
Type=simple
WorkingDirectory=/home/me/repo/kanban
ExecStart=/home/me/.cargo/bin/kanban serve
Restart=on-failure
RestartSec=5

[Install]
WantedBy=default.target
```

Use `kick deploy --dry-run --to local` to see all of it without changing
anything.

<br>

### Home directories

A leading `~`, `$HOME` or `${HOME}` in `bin_dir`, `unit_dir` and the `dest` of
[`[[deploy.files]]`](#deployfiles) expands to the home directory of the user
deploying. For a local deployment that is the user running `kick`. Over ssh it
is the user being logged in as, which the [access check](#access-check) finds
out, so a path which needs expanding is an error with `--no-check`.

The variables of a [user unit](#user-units), such as `working_directory`, are
expanded the same way, since a user unit runs as the user deploying. The
variables of a system unit are not, since systemd itself resolves a `~` in
`WorkingDirectory=` against the user the unit runs as, which is rarely the user
deploying.

Only a leading `~` followed by `/` or nothing at all is expanded, so `~other`
is left alone.

<br>

### Building

Unless told otherwise, `kick deploy` builds the project with `cargo build`
before deploying it. Three options shape what that means:

* `pre_build` a list of commands run before the build, for anything cargo
  doesn't do on its own. Also available as `--pre-build <command>`, which can be
  used more than once.
* `build_features` a list of features to enable in the build. Also available as
  `--build-features <features>`, which can be used more than once and accepts
  comma-separated lists.
* `build` a list of commands which *replaces* the generated build command, for
  when the build is not a plain `cargo build`. `pre_build` still runs, and
  `build_features` is ignored with a warning.

The generated build command follows the `profile` option, so the default
`release` profile builds with `cargo build --release`, and any other profile
builds with `cargo build --profile <profile>`.

Building is skipped entirely with `--no-build`, which is what you want when the
binary has already been built by something else.

Commands in `pre_build` and `build` are either a string, which is split on
whitespace, or a list of arguments in case an argument contains whitespace. Note
that these are *not* run through a shell, so shell syntax such as `&&` or `$VAR`
is not available.

<br>

#### Examples

Deploying a project whose frontend is built by `trunk` and whose binary bundles
the result:

```toml
[deploy]
pre_build = ["trunk build --release"]
build_features = ["bundle"]
```

Which builds with:

```sh
trunk build --release
cargo build --release --features bundle
```

Replacing the build command outright:

```toml
[deploy]
pre_build = ["trunk build --release"]
build = [["cargo", "xtask", "build", "--dist"]]
```

<br>

### `[[deploy.files]]`

Extra files to install alongside the binary.

An entry in the array supports the following fields:
* `source` the local source path being copied, relative to the repo. This can
  also be a wildcard.
* `dest` the absolute path on the remote host the file is installed to. If this
  ends with a `/` the file name of the source is appended to it, which is what
  you want when the source is a wildcard.
* `mode` the file mode being applied, by default this uses the mode of the local
  file.

Note that files are staged by their file name, so two sources which have the
same file name cannot be deployed at the same time.

<br>

#### Examples

```toml
[[deploy.files]]
source = "assets/*"
dest = "/usr/share/track/"
mode = "644"

[[deploy.files]]
source = "config/track.toml"
dest = "/etc/track.toml"
```

<br>

### `systemd`

The unit which is installed and restarted as part of the deployment.

It defaults to the built-in unit template, which is enough to get a service
running without writing any unit file at all. Setting it to `true` says the same
thing explicitly:

```toml
[deploy]
systemd = true
```

Point it at a file to use your own template instead:

```toml
[deploy]
systemd = "systemd/track.service"
```

The unit is installed as `<unit_dir>/<name>.service`, after which `systemctl
enable <name>` and `systemctl start <name>` are run.

The rendered unit is compared against the one which is already installed, and is
only written if the two differ. `systemctl daemon-reload` is run only when it was
written, so a deployment which only changes the binary leaves the unit file and
its modification time alone.

The built-in template is also what applies when this option is absent, so a
deployment installs and starts a service unless it is told not to. Set it to
`false` when the binary is not a service, which puts it into place and stops
there. A single deployment can skip it with `--no-systemd`.

A table can be used when the defaults need to be changed:

```toml
[deploy.systemd]
template = "systemd/track.service"
name = "track"
enable = false
```

* `template` the path to a unit template, relative to the repo. Defaults to the
  built-in template.
* `name` the name of the unit. Defaults to the name of the binary.
* `enable` whether `systemctl enable` is run so that the service starts on boot.
  Defaults to `true`.
* `scope` which systemd instance the unit is installed into, either `system`
  (the default) or `user`. See [user units](#user-units).

Every other key in the section is a [variable](#template-variables) the unit is
rendered with, which is where directives like `User=` or `Environment=` come
from:

```toml
[deploy.systemd]
name = "track"
user = "track"
environment = { RUST_LOG = "info" }
```

Since anything which isn't one of the options above is taken as a
variable, this is the one section which cannot tell you that you misspelled an
option. Writing `enabel = true` defines a variable named `enabel` which the
template doesn't use, and the unit is installed as if you hadn't written it at
all.

<br>

#### User units

With `scope = "user"` the unit is installed into the user instance of systemd
belonging to the user deploying, rather than into the system instance:

* It is managed with `systemctl --user`, and neither it nor the unit file is
  handled with sudo even if `sudo` is enabled, since the unit belongs to the
  user deploying. The binary and any extra files still use sudo if it is
  enabled.
* `unit_dir` defaults to `~/.config/systemd/user`.
* The built-in template leaves out `After=` and `Wants=` unless they are set,
  since `network-online.target` belongs to the system instance, and defaults
  `wanted_by` to `default.target`.
* No `User=` or `Group=` is written unless they are set, as with system units,
  and they rarely should be for a user unit.
* Its variables have a leading `~` [expanded](#home-directories).

The scope is independent of the [kind](#local-deployments) of deployment, so a
user unit can be installed over ssh too. Either way, note that the user instance
of systemd only runs while the user is logged in unless lingering is enabled
with `loginctl enable-linger <user>`.

<br>

#### Templates

Unit templates are rendered with [minijinja], so they use jinja2 syntax. The
template is otherwise used verbatim, it is a regular unit file which happens to
have some holes in it, and can be linted with `systemd-analyze verify` once
rendered.

Templates are compiled when the configuration is loaded, so a syntax error is
reported by any `kick` command rather than in the middle of a deployment.

Referring to a variable which hasn't been defined is an error. Quietly
substituting an empty string would happily produce something like `ExecStart=/usr/local/bin/track --bind`
and install it on a server, so anything optional has to say so with `default()`
or `is defined`:

```jinja
Description={{ description | default(name ~ " service") }}
{%- if user is defined %}
User={{ user }}
{%- endif %}
```

[minijinja]: https://docs.rs/minijinja

<br>

#### Template variables

Templates are rendered with the keys defined in the [`systemd`](#systemd)
section which aren't one of its options, so anything which differs between
projects or deployments is defined there:

```toml
[deploy.systemd]
description = "Track Service"
user = "track"
args = ["--bind", "0.0.0.0:3004"]
```

These are scoped to the unit rather than being taken from the global
[`[variables]`](./variables.md) section, since a `User=` directive is not
something the rest of the configuration has any use for. A variable defined in
`[variables]` is *not* visible to a unit template.

They are layered per repo the same way everything else in `Kick.toml` is, so a
`[repo."<name>".deploy.systemd]` section overrides individual variables without
having to repeat the rest of them.

In addition to what you define, the following are always available:

* `name` the name of the unit.
* `binary` the name of the binary being deployed.
* `exec` the installed path of the binary, which is what `ExecStart` wants. This is
  `bin_dir` and `binary` joined together.
* `bin_dir` the directory the binary is installed into.
* `unit_dir` the directory the unit is installed into.
* `host` the host being deployed to, without the login user. When several hosts
  are being deployed to, the unit is rendered once per host, so this is the host
  it is being installed on. For a local deployment this is `localhost`.
* `scope` the [scope](#user-units) the unit is installed into, `system` or
  `user`.

<br>

#### The built-in template

```jinja
[Unit]
Description={{ description | default(name ~ " service") }}
{%- if after is defined or scope | default("system") != "user" %}
After={{ after | default("network-online.target") }}
{%- endif %}
{%- if wants is defined or scope | default("system") != "user" %}
Wants={{ wants | default("network-online.target") }}
{%- endif %}
{%- if requires is defined %}
Requires={{ requires }}
{%- endif %}
{%- if start_limit_interval_sec is defined %}
StartLimitIntervalSec={{ start_limit_interval_sec }}
{%- endif %}
{%- if start_limit_burst is defined %}
StartLimitBurst={{ start_limit_burst }}
{%- endif %}

[Service]
Type={{ type | default("simple") }}
{%- if user is defined %}
User={{ user }}
{%- endif %}
{%- if group is defined %}
Group={{ group }}
{%- endif %}
{%- if working_directory is defined %}
WorkingDirectory={{ working_directory }}
{%- endif %}
{%- if kill_signal is defined %}
KillSignal={{ kill_signal }}
{%- endif %}
{%- for key, value in environment | default({}) | items %}
Environment={{ key }}={{ value }}
{%- endfor %}
{%- if environment_file is defined %}
EnvironmentFile={{ environment_file }}
{%- endif %}
ExecStart={{ exec }}{% if args is defined %} {{ args | join(" ") }}{% endif %}
Restart={{ restart | default("always") }}
RestartSec={{ restart_sec | default(5) }}
{%- if timeout_stop_sec is defined %}
TimeoutStopSec={{ timeout_stop_sec }}
{%- endif %}

[Install]
WantedBy={{ wanted_by | default("default.target" if scope | default("system") == "user" else "multi-user.target") }}
```

Every variable it uses beyond the ones above is optional, and defining one in
the `[deploy.systemd]` section fills in the corresponding directive:

* `description`, defaults to `<name> service`.
* `after` and `wants`, both default to `network-online.target` for a system
  unit, and are left out of a [user unit](#user-units) unless set.
* `requires`.
* `start_limit_interval_sec` and `start_limit_burst`.
* `type`, defaults to `simple`.
* `user` and `group`, which can also be set with `--service-user <user>` and
  `--group <group>`. The options override the variables, and `--service-user` on
  its own also defines `group` unless the variable is set. Note that this is the
  user the *service* runs as, and has nothing to do with the `user` option in
  the `[deploy]` section, which is the user being logged in as.
* `working_directory`.
* `kill_signal`.
* `environment`, a table which becomes one `Environment=` per entry.
* `environment_file`.
* `args`, a list which is appended to `ExecStart`. Can also be set with
  `--args <args>`, which overrides the variable. Values are taken as they are
  given, so `--args --user x` passes `--user x` to the service, and an argument
  which itself contains whitespace has to be specified through the variable.
* `restart`, defaults to `always`.
* `restart_sec`, defaults to `5`.
* `timeout_stop_sec`.
* `wanted_by`, defaults to `multi-user.target`, or `default.target` for a
  [user unit](#user-units).

If you need something the built-in template doesn't cover, copy it out of
`src/systemd/default.service` in the [kick repo] and point `template` at your own
copy.

[kick repo]: https://github.com/udoprog/kick/blob/main/src/systemd/default.service

<br>

#### Examples

The following configuration:

```toml
[deploy]
host = "moore"
user = "integration"
binary = "track"

[deploy.systemd]
description = "Track Service"
user = "track"
group = "track"
working_directory = "~"
kill_signal = "SIGINT"
start_limit_interval_sec = "60s"
start_limit_burst = 3
timeout_stop_sec = "5min"
args = ["--bind", "0.0.0.0:3004"]
```

Installs the following unit into `/etc/systemd/system/track.service`:

```text
[Unit]
Description=Track Service
After=network-online.target
Wants=network-online.target
StartLimitIntervalSec=60s
StartLimitBurst=3

[Service]
Type=simple
User=track
Group=track
WorkingDirectory=~
KillSignal=SIGINT
ExecStart=/usr/local/bin/track --bind 0.0.0.0:3004
Restart=always
RestartSec=5
TimeoutStopSec=5min

[Install]
WantedBy=multi-user.target
```

<br>

### Access check

Before anything is built or uploaded, `kick deploy` logs into every remote host
once to make sure the deployment can actually go through. For each host it checks
that:

* We can log in over ssh at all, and that we end up as the user the `user`
  option asks for. Ending up as a different user is a warning, not an error.
* `install` is available, along with `systemctl` and `cmp` if a unit is being
  installed.
* `sudo` can be used without being prompted for a password, when `sudo` is
  enabled. This is an error, see
  [sudo and interactivity](#sudo-and-interactivity) below.

The check also finds out the home directory of the user being logged in as,
which is what a leading `~` [expands](#home-directories) to.

The check only reads state, it doesn't modify the remote host, so it is
performed for `--dry-run` as well. It can be skipped with `--no-check`. A
[local deployment](#local-deployments) has no access check.

<br>

### sudo and interactivity

The script which installs the deployment is handed to `ssh` as a single command.
No terminal is allocated for it, which means it runs non-interactively and there
is nowhere for a `sudo` password prompt to be answered.

Because of this every privileged command in the script is run with `sudo -n`, so
sudo fails immediately with `sudo: a password is required` instead of blocking on
a prompt which can never be answered. The [access check](#access-check) tests the
same thing up front with `sudo -n true` and refuses to deploy when it fails,
which means you find out before anything is built rather than part-way through
installing.

If you hit this, either:

* Give the deploying user a `NOPASSWD` entry in sudoers for the commands being
  used, which is what you want for anything which is deployed regularly or from
  CI. For example, in `/etc/sudoers.d/track`:

  ```text
  integration ALL=(ALL) NOPASSWD: /usr/bin/install, /usr/bin/cmp, /usr/bin/mkdir, /usr/bin/systemctl
  ```

* Or set `sudo = false` in the `[deploy]` section if you deploy as `root` and
  don't need to elevate at all.

Note that this only applies to `sudo`. Authenticating to the host itself is
handled by `ssh`, which reads prompts from the terminal directly, so an ssh key
passphrase or a login password still works as usual.

<br>

### Options

The `kick deploy` action takes the name of the binary to deploy as an argument,
which overrides the `binary` option, along with the following options:

* `--to <profile>` selects the [profile](#profiles) to deploy, overriding the
  `default_profile` option.

* `--host <host>` replaces the `host` option, can be used more than once to
  deploy to [several hosts](#deploying-to-more-than-one-host).
* `--user <user>` overrides the `user` option, which is the user being logged in
  as. Ignored for a host which spells out a user of its own.
* `--service-user <user>` sets the `user` variable, which the unit installs as a
  `User=` directive. Without it the service runs as `root`. This is the user the
  service runs as, not the user the deployment is performed as, which is
  `--user`.
* `--group <group>` sets the `group` variable, which the unit installs as a
  `Group=` directive. Defaults to `--service-user`, unless `group` is set in
  `[deploy.systemd]`.
* `--args <args>` sets the `args` variable, which the unit appends to
  `ExecStart`. Can be used more than once, and each use is split on whitespace.
* `--profile <profile>` overrides the `profile` option, which is the cargo
  build profile and not a [deploy profile](#profiles).
* `--pre-build <command>` adds a command to `pre_build`, can be used more than
  once.
* `--build-features <features>` adds features to `build_features`, can be used
  more than once.
* `--no-check` skips the [access check](#access-check).
* `--no-build` skips [building](#building) altogether.
* `--no-systemd` skips installing the systemd unit, and by extension restarting
  the service.
* `--no-restart` installs everything without stopping or starting the service.
* `--dry-run` prints the profile being deployed, the unit which would be
  installed and every command which would be run without changing anything.
* `--verbose` / `-V` prints the deployment plan, the unit being installed and
  the script which is run remotely, and traces the remote script as it
  executes. Passing it twice (`-VV`) also prints the [access
  check](#access-check) and makes `ssh` and `scp` verbose.

<br>

### Requirements

A local deployment needs `install`, and `systemctl` and `cmp` if a unit is
being installed, on the machine `kick` is running on.

Every remote host being deployed to is expected to:

* Be reachable over `ssh`, either without a prompt or by prompting on the
  terminal `kick` is being run from.
* Allow `sudo` without a password when `sudo` is enabled, see
  [sudo and interactivity](#sudo-and-interactivity).
* Use a POSIX-compatible login shell.
* Have `install` available, which is part of coreutils.
* Have `systemd` and `cmp` available if the `systemd` option is in use. The
  latter is part of diffutils.
