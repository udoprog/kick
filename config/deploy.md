Deployment configuration.

The `[deploy]` section configures the `kick deploy` action, which is a
lightweight way to deploy a project to a server over ssh, or to the machine
`kick` is running on through a [local deployment](#local-deployments). A project
which is deployed in more than one way defines [profiles](#profiles).

`kick deploy` shares how the project is built with
[`kick install`](./install.md) through the [`[build]` section](./build.md), and
an install is the simple case of a local deployment, so the two take the same
options. Everything here applies to `[install]` too, unless it says otherwise.

Deploying performs the following steps:

* Every remote host is [checked for access](#access-check) before any work is
  done.
* The project is [built](./build.md) locally, once regardless of how many hosts
  are being deployed to.
* The binary to deploy is located in `<target>/<profile>/<binary>`, where
  `<target>` is the target directory cargo reports.
* Then, for each host in turn, a single `ssh` invocation:
  * Receives the binary, any [extra files](#deployfiles) and the rendered
    [systemd unit](#systemd), along with its [socket unit](#socket-units) if it
    has one, as a `tar` archive streamed over its stdin, and
    unpacks them into the [staging directory](#deploy-section) on the remote
    host.
  * Stops the service, installs everything into place, and starts the service
    again. The unit is only written and systemd only reloaded if the unit
    actually changed.

Nothing is installed remotely unless the upload succeeded, and the staged files
are removed once they've been installed. See
[connections](#connections) for how many times each host is logged into.

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
kick deploy --host moore docular --pre-build "trunk build --release" --features bundle
```

Which builds with `trunk build --release` followed by
`cargo build --release --features bundle`.

Put the same things in the configuration once they stop fitting on a command
line, or when they are a property of the project rather than of one deployment.
How the project is built goes in the [`[build]` section](./build.md), which
`kick install` uses too, and where it is deployed to in `[deploy]`:

```toml
[build]
pre_build = ["trunk build --release"]
features = ["bundle"]

[deploy]
host = "moore"
user = "integration"
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
* `host` the host to deploy to, such as `moore`. A list deploys to
  [several hosts](#deploying-to-more-than-one-host) in turn. `--host <host>`
  replaces it, and an ssh deployment which doesn't set it takes its host from
  the command line, see
  [leaving the host to the command line](#leaving-the-host-to-the-command-line).
* `user` the user to log into the hosts as, such as `integration`. This is the
  user the deployment is performed as, not the user the deployed service runs
  as, which is a [variable](#template-variables) in the `[deploy.systemd]`
  section. Also available as `--user <user>`.
* `port` the port to connect over. Passed to `ssh` with `-p`.
* `identity_file` the identity file used to authenticate. Passed with `-i`.
* `options` a list of extra options passed to `ssh` with `-o`, such as the
  ones which [share a connection](#connections) between the access check and
  the deployment.
* `sudo` whether privileged remote commands are prefixed with `sudo`. Defaults
  to `true` over ssh and `false` for a local deployment, set it to `false` when
  deploying as `root`. See [sudo and interactivity](#sudo-and-interactivity),
  since sudo must not require a password over ssh.
* `bin_dir` the directory the binary is installed into. Defaults to
  `/usr/local/bin` over ssh, and to `$CARGO_HOME/bin` or `~/.cargo/bin` for a
  [local deployment](#local-deployments). A leading `~`
  [expands](#home-directories) to the home directory of the user deploying.
  Also available as `--bin-dir <dir>`.
* `unit_dir` the directory the systemd unit is installed into. Defaults to
  `/etc/systemd/system`, or `~/.config/systemd/user` for a
  [user unit](#user-units).
* `staging_dir` the remote directory files are uploaded to before they are
  installed. Defaults to `.kick-deploy`, which is relative to the home directory
  of the user being logged in as. A leading `~` [expands](#home-directories)
  like it does for `bin_dir`. Not used by a local deployment.
* `build` a table which is [layered](./build.md#layering) over the `[build]`
  section, for anything about the build which is particular to deploying.
* `files` a list of [extra files](#deployfiles) to install.
* `post_install` and `post_start` [commands](#target-commands) run on the
  target once everything is installed, and once the service has been started.
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
`[deploy]` section: what the profile sets wins, lists like `files` and
`post_install` are added to, `[deploy.profiles.<name>.build]` is layered over
the [build](./build.md#layering), and `[deploy.profiles.<name>.systemd]`
overrides individual options and variables of `[deploy.systemd]`. So `[deploy]`
holds what the profiles share, and each profile what is particular to it:

```toml
[build]
binary = "track"
pre_build = ["trunk build --release"]
features = ["bundle"]

[deploy.profiles.production]
host = ["moore", "dahl"]
user = "integration"

[deploy.profiles.staging]
host = "hilbert"
user = "integration"

[deploy.profiles.local]
kind = "local"
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
`--profile` and the `profile` option in `[build]` are the cargo build profile
the binary is picked up from.

`[install]` takes profiles the same way, which are selected with
`kick install --to <name>`.

#### Leaving the host to the command line

A profile doesn't have to say where it deploys to. One which describes *how* the
project is deployed to a server, but not *which* server, leaves `host` out:

```toml
[build]
binary = "track"

[deploy.profiles.remote]
user = "integration"

[deploy.profiles.local]
kind = "local"
```

The host is then given when deploying, so the same project can be put on any
machine without editing its `Kick.toml`:

```sh
kick deploy --to remote --host moore
kick deploy --to remote --host dahl --host hilbert
```

Deploying such a profile without `--host` is an error which names the profile
and the command to run instead. A `host` set in `[deploy]` is shared with every
profile like any other option, so a profile only goes without one when neither
it nor `[deploy]` sets it.

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
* `bin_dir` defaults to `$CARGO_HOME/bin`, or `~/.cargo/bin`, the same as for
  [`kick install`](./install.md).
* The unit is a [user unit](#user-units) unless `scope` says otherwise, since
  it needs neither root nor sudo.

So a local deployment installs into your own home directory without any
further configuration, and the only difference between `kick install` and a
local deployment is that the deployment installs a systemd unit by default.

<br>

#### Example: the kanban board

The kanban board runs as a user service on the machine it is developed on. Its
frontend is built by `trunk` and bundled into the binary, which is installed
into `~/.cargo/bin` and started with `kanban serve` from the checkout:

```toml
[build]
binary = "kanban"
pre_build = ["trunk build --release"]
features = ["bundle"]

[deploy.profiles.local]
kind = "local"

[deploy.profiles.local.systemd]
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

A leading `~`, `$HOME` or `${HOME}` in `bin_dir`, `unit_dir`, `staging_dir`
and the `dest` of [`[[deploy.files]]`](#deployfiles) expands to the home
directory of the user deploying. For a local deployment that is the user running `kick`. Over ssh it
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

How the project is built is configured in the [`[build]` section](./build.md),
which is shared with `kick install`. `[deploy.build]` and
`[deploy.profiles.<name>.build]` adjust it for deployments alone, see
[layering](./build.md#layering).

Building is skipped entirely with `--no-build`, which is what you want when the
binary has already been built by something else.

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

### Target commands

`post_install` and `post_start` are lists of commands which are run on the
machine being deployed to, as part of the script which installs the deployment.
Use them for anything which has to happen next to the installed binary rather
than before the build, like migrating a database with the new binary or
creating a system user.

* `post_install` runs after the binary, the files and the units have been
  installed and systemd has been reloaded, but before the socket or the service
  is enabled and started.
* `post_start` runs after the service has been started (or the socket enabled
  and the service restarted).

Without a systemd unit (`systemd = false`, `--no-systemd`, or an install which
has none) there is nothing to start, so both run once everything has been
installed, `post_install` first. Since they are part of starting the service,
`--no-restart` skips them.

An entry is [configured](./build.md#commands) as a string, a list of arguments
or a table:

* A string is a command line which is run by the shell executing the script,
  exactly as written. Quoting, `~` and `$HOME` work like they do in the shell of
  the user deploying.
* A list of arguments is quoted for that shell, so nothing in it is expanded.
* A table takes either of the above as `command`, and `sudo = true` to run it
  under the same `sudo` prefix as the rest of the deployment (see
  [sudo and interactivity](#sudo-and-interactivity)). The prefix is put in front
  of the command line, so the shell of the deploying user still expands `~`
  before sudo runs, and it only applies to the first command in it. Use
  `sh -c '...'` to run something like a pipeline under sudo. With `sudo = false`
  in the `[deploy]` section, there is no prefix.

The commands of a [profile](#profiles) are added after the ones in `[deploy]`.
The script runs with `set -eu`, so a command which fails fails the deployment,
and they show up in the script printed by `--dry-run` and `-V`. They run over
the same connection as the rest of the deployment.

<br>

#### Example

A system deployment of the kanban board creates the `kanban` group its socket
is shared with before the socket starts, and refreshes the skills bundled with
the new binary before the service restarts:

```toml
[deploy.profiles.server]
post_install = [
    { command = "systemd-sysusers", sudo = true },
    { command = "/usr/local/bin/kanban --db /var/lib/kanban/kanban.db install", sudo = true },
]

[[deploy.profiles.server.files]]
source = "dist/sysusers.conf"
dest = "/etc/sysusers.d/kanban.conf"
mode = "644"
```

Which adds the following to the script, right before the socket is enabled:

```sh
sudo -n systemd-sysusers
sudo -n /usr/local/bin/kanban --db /var/lib/kanban/kanban.db install
sudo -n systemctl enable --now kanban.socket
sudo -n systemctl restart kanban
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
* `scope` which systemd instance the unit is installed into, either `system` or
  `user`. Defaults to `system` over ssh, and to `user` for a
  [local deployment](#local-deployments) or an install. See
  [user units](#user-units).
* `socket` a socket unit which activates the service, see [socket
  units](#socket-units).

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

#### Socket units

A service which is started by systemd on demand, or which is handed a socket
that systemd owns, is installed together with a socket unit through
`[deploy.systemd.socket]`. It takes the same forms as the `systemd` option:

```toml
[deploy.systemd]
# The built-in socket template, configured through variables.
socket = true
# Or your own template.
socket = "systemd/kanban.socket"
```

Or as a table:

```toml
[deploy.systemd.socket]
template = "systemd/kanban.socket"
name = "kanban"
listen_stream = "/run/kanban/kanban.sock"
```

* `template` the path to a socket unit template, relative to the repo. Defaults
  to the [built-in socket template](#the-built-in-socket-template).
* `name` the name of the socket unit. Defaults to the name of the service, which
  is what lets systemd pair them up without a `Service=` directive.

Every other key in the section is a variable the socket unit is rendered with.
These are the socket's own, a variable defined for the service is not visible to
the socket and the other way around, but the [built-in
variables](#template-variables) are available to both, with `name` being the name
of the socket unit. A socket template also has `service`, the file name of the
service it activates such as `kanban.service`. A service template with a socket
has `socket`, the file name of the socket unit such as `kanban.socket`, which the
built-in template uses to add `Requires=` and `After=` on it.

Like the rest of the `systemd` section, a profile can override individual
socket variables, or turn the socket off with `socket = false`.

The socket unit is installed as `<unit_dir>/<name>.socket` next to the service,
and is compared and written the same way, only when it changed. With a socket
unit, the deployment:

* Compares the socket unit with the installed one up front. If it changed, or
  isn't installed yet, the service and then the socket are stopped.
* Installs the binary, the service unit and, if it changed, the socket unit, and
  runs `systemctl daemon-reload` once if either unit was written.
* Runs any [`post_install`](#target-commands) commands.
* Runs `systemctl enable --now <name>.socket`. It is the socket which is
  enabled, not the service. With `enable = false` the socket is only started.
* Runs `systemctl restart <service>`, followed by any
  [`post_start`](#target-commands) commands.

So a deployment which only changes the binary leaves an active socket alone and
only restarts the service, and anything connecting in the meantime is queued by
systemd rather than refused. With `--no-restart`, the units are installed and
the socket enabled, but nothing is stopped or started.

<br>

#### The built-in socket template

```jinja
[Unit]
Description={{ description | default(name ~ " socket") }}

[Socket]
{%- for listen in ([listen_stream] if listen_stream is string else listen_stream) %}
ListenStream={{ listen }}
{%- endfor %}
{%- if socket_user is defined %}
SocketUser={{ socket_user }}
{%- endif %}
{%- if socket_group is defined %}
SocketGroup={{ socket_group }}
{%- endif %}
{%- if socket_mode is defined %}
SocketMode={{ socket_mode }}
{%- endif %}
{%- if directory_mode is defined %}
DirectoryMode={{ directory_mode }}
{%- endif %}
{%- if remove_on_stop is defined %}
RemoveOnStop={{ remove_on_stop if remove_on_stop is string else ("yes" if remove_on_stop else "no") }}
{%- endif %}
{%- if service != name ~ ".service" %}
Service={{ service }}
{%- endif %}

[Install]
WantedBy={{ wanted_by | default("sockets.target") }}
```

* `listen_stream` is required, either a single address or a list of them, each
  of which becomes a `ListenStream=`.
* `description`, defaults to `<name> socket`.
* `socket_user`, `socket_group` and `socket_mode`.
* `directory_mode`.
* `remove_on_stop`, either a boolean or a string such as `"yes"`.
* `wanted_by`, defaults to `sockets.target`.

`Service=` is only written when the socket is named differently from the
service.

<br>

#### Example: a socket-activated service

The kanban board listens on a unix socket which systemd owns. As a user service
the socket lives in the runtime directory of the user, and on a server it is
shared with the members of the `kanban` group:

```toml
[build]
binary = "kanban"

[deploy.systemd]
args = ["serve"]

[deploy.systemd.socket]
remove_on_stop = true

[deploy.profiles.local]
kind = "local"

[deploy.profiles.local.systemd.socket]
listen_stream = "%t/kanban/kanban.sock"
socket_mode = "0600"
directory_mode = "0700"

[deploy.profiles.server]
host = "moore"

[deploy.profiles.server.systemd]
user = "kanban"

[deploy.profiles.server.systemd.socket]
listen_stream = "/run/kanban/kanban.sock"
socket_mode = "0660"
socket_group = "kanban"
```

`kick deploy --to local` installs the following into
`~/.config/systemd/user/kanban.socket`:

```text
[Unit]
Description=kanban socket

[Socket]
ListenStream=%t/kanban/kanban.sock
SocketMode=0600
DirectoryMode=0700
RemoveOnStop=yes

[Install]
WantedBy=sockets.target
```

Along with a `kanban.service` which has `Requires=kanban.socket` and
`After=kanban.socket`, and runs:

```sh
set -eu
socket_changed=no
if ! cmp -s <rendered socket> ~/.config/systemd/user/kanban.socket; then
  socket_changed=yes
fi
if [ "$socket_changed" = yes ]; then
  systemctl --user stop kanban 2>/dev/null || true
  systemctl --user stop kanban.socket 2>/dev/null || true
fi
mkdir -p ~/.cargo/bin
install -m 0755 <repo>/target/release/kanban ~/.cargo/bin/kanban
mkdir -p ~/.config/systemd/user
reload=no
if ! cmp -s <rendered unit> ~/.config/systemd/user/kanban.service; then
  install -m 0644 <rendered unit> ~/.config/systemd/user/kanban.service
  reload=yes
fi
if [ "$socket_changed" = yes ]; then
  install -m 0644 <rendered socket> ~/.config/systemd/user/kanban.socket
  reload=yes
fi
if [ "$reload" = yes ]; then
  systemctl --user daemon-reload
fi
systemctl --user enable --now kanban.socket
systemctl --user restart kanban
```

`kick deploy --to server` does the same with `sudo -n systemctl` over ssh, and
installs into `/etc/systemd/system`. Use `--dry-run` with either to see both
rendered units and the script without changing anything.

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
{%- if socket is defined %}
Requires={{ socket }}
After={{ socket }}
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
* `socket` is defined when a [socket unit](#socket-units) is installed with the
  service, which adds `Requires=` and `After=` on it.
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
[build]
binary = "track"

[deploy]
host = "moore"
user = "integration"

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
* `install` and `tar` are available, along with `systemctl` and `cmp` if a unit
  is being installed.
* `sudo` can be used without being prompted for a password, when `sudo` is
  enabled. This is an error, see
  [sudo and interactivity](#sudo-and-interactivity) below.

The check also finds out the home directory of the user being logged in as,
which is what a leading `~` [expands](#home-directories) to.

The check only reads state, it doesn't modify the remote host, so it is
performed for `--dry-run` as well. It can be skipped with `--no-check`. A
[local deployment](#local-deployments) has no access check.

<br>

### Connections

Each host is logged into twice: once by the [access check](#access-check)
before the build, and once to deploy, which sends the files and installs them
over the same connection. With `--no-check` that is once per host.

The access check is a separate connection on purpose, since it runs before what
can be a lengthy build so that an unreachable host is found out about up front,
and since the home directory it reports is needed to [expand](#home-directories)
the paths being deployed to.

If logging in is expensive, such as when it prompts for a password or a second
factor, OpenSSH can share a single authenticated connection between the two
through connection multiplexing. `kick` doesn't do this for you, since it isn't
available everywhere (notably not in the OpenSSH which ships with Windows) and
since it would override any multiplexing you have configured yourself, but it
only takes a few `options`:

```toml
[deploy]
host = "integration@moore"
options = [
    "ControlMaster=auto",
    "ControlPath=~/.ssh/kick-%C",
    "ControlPersist=10m",
]
```

`ControlPersist` keeps the connection open in the background after the access
check exits, and needs to outlive the build for the deployment to reuse it.
The same thing can be done for every connection to a host with a `Host` block
in `~/.ssh/config`. Run `ssh -O exit -o ControlPath=~/.ssh/kick-%C <host>` to
close a shared connection early.

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
which overrides the `binary` option in `[build]`, along with the following
options. All of them except `--host`, `--user` and `--no-check` are also taken
by `kick install`:

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
* `--bin-dir <dir>` overrides the `bin_dir` option.
* The [build options](./build.md#options) `--profile`, `--package`,
  `--features`, `--pre-build` and `--no-build`.
* `--no-check` skips the [access check](#access-check).
* `--no-systemd` skips installing the systemd unit, and by extension restarting
  the service.
* `--no-restart` installs everything without stopping or starting the service,
  and skips the [target commands](#target-commands).
* `--dry-run` prints the profile being deployed, the unit which would be
  installed and every command which would be run without changing anything.
* `--verbose` / `-V` prints the deployment plan, the unit being installed and
  the script which is run remotely, and traces the remote script as it
  executes. Passing it twice (`-VV`) also prints the [access
  check](#access-check) and makes `ssh` verbose.

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
* Have `tar` available, which the files being deployed are sent as. Any of GNU
  tar, bsdtar and busybox tar will do.
* Have `systemd` and `cmp` available if the `systemd` option is in use. The
  latter is part of diffutils.

<br>

### Migrating

How the project is built moved out of `[deploy]` into the
[`[build]` section](./build.md) which `kick install` shares, and `pre_start`
was renamed to `post_install`, which says when it runs for an install too:

| Before                              | After                                 |
|-------------------------------------|---------------------------------------|
| `[deploy] binary`                   | `[build] binary`                      |
| `[deploy] profile`                  | `[build] profile`                     |
| `[deploy] pre_build`                | `[build] pre_build`                   |
| `[deploy] build`                    | `[build] commands`                    |
| `[deploy] build_features`           | `[build] features`                    |
| `pre_start`                         | `post_install`                        |
| `--build-features`                  | `--features`                          |

Options which were set in a profile move to `[deploy.profiles.<name>.build]`.
kick reports each of the old options with where it went. A local deployment
now defaults to `bin_dir = "~/.cargo/bin"` and a user unit, so those can be
removed from a `kind = "local"` profile:

```toml
# Before.
[deploy]
binary = "kanban"
pre_build = ["trunk build --release"]
build_features = ["bundle"]

[deploy.profiles.local]
kind = "local"
bin_dir = "~/.cargo/bin"
pre_start = ["~/.cargo/bin/kanban install"]

[deploy.profiles.local.systemd]
scope = "user"

# After.
[build]
binary = "kanban"
pre_build = ["trunk build --release"]
features = ["bundle"]

[deploy.profiles.local]
kind = "local"
post_install = ["~/.cargo/bin/kanban install"]
```
