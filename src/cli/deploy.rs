use std::collections::{HashMap, HashSet};
use std::env::consts::EXE_EXTENSION;
use std::fmt::Write as _;
use std::io::{IsTerminal as _, Write as _};
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, anyhow, bail};
use clap::{Args, Parser};
use termcolor::{ColorChoice, StandardStream};

use crate::cli::WithRepos;
use crate::config::{Build, ConfigCommand, Deploy, DeployKind, Section, SystemdScope};
use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::model::Repo;
use crate::packaging::{self, Mode};
use crate::process::Command;
use crate::shell::Shell;
use crate::systemd;

/// The remote directory binaries are installed into by default.
const DEFAULT_BIN_DIR: &str = "/usr/local/bin";
/// The directory a local install puts binaries into by default, unless
/// `CARGO_HOME` says otherwise.
const DEFAULT_CARGO_BIN_DIR: &str = "~/.cargo/bin";
/// The directory system units are installed into by default.
const DEFAULT_UNIT_DIR: &str = "/etc/systemd/system";
/// The directory user units are installed into by default.
const DEFAULT_USER_UNIT_DIR: &str = "~/.config/systemd/user";
/// The host a local deployment is reported as deploying to.
const LOCAL_HOST: &str = "localhost";
/// The remote directory files are uploaded to by default, relative to the home
/// directory of the user being logged in as.
const DEFAULT_STAGING_DIR: &str = ".kick-deploy";
/// The build profile binaries are picked up from by default.
const DEFAULT_PROFILE: &str = "release";
/// Remote commands which are always needed.
///
/// `tar` unpacks the files being deployed, which are streamed to the remote
/// host over the same ssh connection which installs them.
const REQUIRED_COMMANDS: &[&str] = &["install", "tar"];
/// Remote commands which are needed to install a systemd unit.
const SYSTEMD_COMMANDS: &[&str] = &["systemctl", "cmp"];

/// The options shared by `kick install` and `kick deploy`.
#[derive(Default, Debug, Clone, Args)]
pub(crate) struct Common {
    /// The name of the binary to install.
    ///
    /// This overrides the `binary` option in the `[build]` section, and
    /// defaults to the `package` being built or the name of the primary crate
    /// in the project.
    pub(crate) binary: Option<String>,
    /// The profile to use, as defined in a `profiles.<name>` section.
    ///
    /// Defaults to the `default_profile` option of the section, or the only
    /// profile if there is just one. With several profiles and no default, you
    /// are asked which one to use when running in a terminal.
    ///
    /// Note that this is not the same as `--profile`, which is the cargo build
    /// profile.
    #[arg(long = "to", value_name = "PROFILE")]
    pub(crate) to: Option<String>,
    /// The cargo build profile, overrides the `profile` option in the
    /// `[build]` section.
    #[arg(long)]
    pub(crate) profile: Option<String>,
    /// The cargo package to build, overrides the `package` option in the
    /// `[build]` section.
    #[arg(long, value_name = "PACKAGE")]
    pub(crate) package: Option<String>,
    /// A feature to enable when building, can be used more than once and takes
    /// comma-separated lists.
    ///
    /// This is added to the `features` option in the `[build]` section, and
    /// has no effect if the build command is replaced through `commands`.
    #[arg(long = "features", value_name = "FEATURES")]
    pub(crate) features: Vec<String>,
    /// A command to run before the project is built, can be used more than
    /// once.
    ///
    /// This is added to the `pre_build` option in the `[build]` section.
    #[arg(long = "pre-build", value_name = "COMMAND")]
    pub(crate) pre_build: Vec<String>,
    /// Do not build the project, which is what you want when the binary has
    /// already been built.
    #[arg(long)]
    pub(crate) no_build: bool,
    /// The directory the binary is installed into, overrides the `bin_dir`
    /// option.
    #[arg(long = "bin-dir", value_name = "DIR")]
    pub(crate) bin_dir: Option<String>,
    /// The arguments the installed service is started with.
    ///
    /// Can be used more than once, and each use is split on whitespace. This
    /// defines the `args` variable, which the built-in unit template appends
    /// to `ExecStart`, and overrides `args` in the `systemd` section. An
    /// argument which itself contains whitespace has to be specified through
    /// the variable instead.
    ///
    /// Since service arguments tend to start with `-`, values are taken as
    /// they are given, which means that `--args --user x` passes `--user x` to
    /// the service rather than being read as an option to `kick`.
    #[arg(long = "args", value_name = "ARGS", allow_hyphen_values = true)]
    pub(crate) args: Vec<String>,
    /// The user the installed service runs as.
    ///
    /// This defines the `user` variable, which the built-in unit template
    /// installs as a `User=` directive, and overrides `user` in the `systemd`
    /// section. Without it a system service runs as `root`, which is what
    /// systemd does in the absence of a `User=` directive.
    ///
    /// This is the user the service runs as, not the user a deployment logs in
    /// as, which is `--user`.
    #[arg(long = "service-user", value_name = "USER")]
    pub(crate) service_user: Option<String>,
    /// The group the installed service runs as.
    ///
    /// This defines the `group` variable, which the built-in unit template
    /// installs as a `Group=` directive. It defaults to `--service-user`,
    /// since a service which runs as a dedicated user conventionally has a
    /// group of the same name, unless `group` is set in the `systemd` section.
    #[arg(long)]
    pub(crate) group: Option<String>,
    /// Do not install the systemd unit.
    #[arg(long)]
    pub(crate) no_systemd: bool,
    /// Do not stop or start the service, which also skips the `post_install`
    /// and `post_start` commands.
    #[arg(long)]
    pub(crate) no_restart: bool,
    /// Print the commands which would be run instead of running them.
    ///
    /// Note that the access check of a deployment is still performed, since
    /// it does not modify the remote host.
    #[arg(long)]
    pub(crate) dry_run: bool,
    /// Print verbose information about what is being done.
    ///
    /// One level `-V` prints the plan, the systemd unit and the install
    /// script, and traces the script as it executes. Two levels `-VV`
    /// additionally prints the access check of a deployment and passes `-v`
    /// to `ssh`.
    #[arg(long, short = 'V', action = clap::ArgAction::Count)]
    pub(crate) verbose: u8,
}

impl Common {
    /// Whether details about what is being done should be printed.
    ///
    /// A dry run is verbose by definition, since printing what would be done
    /// is the only thing it does.
    fn details(&self) -> bool {
        self.verbose >= 1 || self.dry_run
    }
}

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[command(flatten)]
    pub(crate) common: Common,
    /// A host to deploy to, can be used more than once.
    ///
    /// This replaces the `host` option in the `[deploy]` section rather than
    /// adding to it, and each host is deployed to in turn. A login user can be
    /// spelled out as part of the host, in which case it wins over `--user`.
    ///
    /// It is required when deploying over ssh with a configuration which sets no
    /// host, such as `kick deploy --to remote --host moore`.
    #[arg(long = "host", value_name = "HOST")]
    host: Vec<String>,
    /// The user to log into the hosts as, overrides the `user` option in the
    /// `[deploy]` section.
    ///
    /// This is the user the deployment is performed as, not the user the
    /// deployed service runs as, which is `--service-user`. It is ignored for
    /// a host which spells out a user of its own.
    #[arg(long)]
    user: Option<String>,
    /// Do not check that the remote host can be accessed before deploying.
    #[arg(long)]
    no_check: bool,
}

impl Opts {
    /// Options for an install, which has no remote options.
    pub(crate) fn local(common: Common) -> Self {
        Self {
            common,
            ..Self::default()
        }
    }
}

impl Deref for Opts {
    type Target = Common;

    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.common
    }
}

/// Print a block of `#` prefixed lines describing what is being done.
fn details<'a>(
    o: &mut StandardStream,
    title: &str,
    lines: impl IntoIterator<Item = &'a str>,
) -> Result<()> {
    writeln!(o, "# {title}:")?;

    for line in lines {
        if line.is_empty() {
            writeln!(o, "#")?;
        } else {
            writeln!(o, "#   {line}")?;
        }
    }

    Ok(())
}

pub(crate) fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    let mut o = StandardStream::stdout(ColorChoice::Auto);

    with_repos.run("deploy", format_args!("deploy: {opts:?}"), |cx, repo| {
        deploy(&mut o, cx, opts, repo, Section::Deploy)
    })?;

    Ok(())
}

/// Which profile a deployment uses.
#[derive(Debug, PartialEq, Eq)]
enum Choice<'a> {
    /// No profiles are defined, so the `[deploy]` section is used as it is.
    Base,
    /// The named profile.
    Profile(&'a str),
    /// Several profiles are defined and none is selected, so the user has to
    /// be asked which one to use.
    Ask(Vec<&'a str>),
}

/// The names of the defined profiles as a comma-separated list.
fn profile_names(config: &Deploy) -> String {
    config
        .profiles
        .keys()
        .map(|name| format!("`{name}`"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Pick the profile to deploy.
///
/// `--to` wins, followed by `default_profile`, followed by the only profile
/// there is. With several profiles and nothing to pick between them, the user
/// is asked if `interactive` is set, and it is an error otherwise.
fn choose_profile<'a>(
    config: &'a Deploy,
    section: Section,
    to: Option<&'a str>,
    interactive: bool,
) -> Result<Choice<'a>> {
    let section = section.as_str();

    if let Some(to) = to {
        if config.profiles.is_empty() {
            bail!(
                "Cannot use profile `{to}` since no profiles are defined, add a `[{section}.profiles.{to}]` section"
            );
        }

        if !config.profiles.contains_key(to) {
            bail!(
                "No profile named `{to}` in `[{section}]`, the defined profiles are: {}",
                profile_names(config)
            );
        }

        return Ok(Choice::Profile(to));
    }

    if config.profiles.is_empty() {
        if let Some(default) = &config.default_profile {
            bail!(
                "The `default_profile` in `[{section}]` is `{default}`, but no profiles are defined, add a `[{section}.profiles.{default}]` section"
            );
        }

        return Ok(Choice::Base);
    }

    if let Some(default) = &config.default_profile {
        if !config.profiles.contains_key(default) {
            bail!(
                "The `default_profile` in `[{section}]` is `{default}`, which is not a defined profile, the defined profiles are: {}",
                profile_names(config)
            );
        }

        return Ok(Choice::Profile(default));
    }

    let mut names = config.profiles.keys().map(String::as_str);

    if let (Some(name), None) = (names.next(), names.next()) {
        return Ok(Choice::Profile(name));
    }

    if interactive {
        return Ok(Choice::Ask(
            config.profiles.keys().map(String::as_str).collect(),
        ));
    }

    bail!(
        "Multiple profiles are defined in `[{section}]` and none is selected, pass `--to <profile>` or set `default_profile` in `[{section}]`. The defined profiles are: {}",
        profile_names(config)
    )
}

/// Ask the user which of the given profiles to deploy.
fn ask_profile<'a>(names: &[&'a str]) -> Result<&'a str> {
    let stdin = std::io::stdin();
    let mut stderr = std::io::stderr();

    writeln!(stderr, "Multiple profiles are defined:")?;

    for (index, name) in names.iter().enumerate() {
        writeln!(stderr, "  {}) {name}", index + 1)?;
    }

    loop {
        write!(stderr, "Profile to use [1-{}]: ", names.len())?;
        stderr.flush()?;

        let mut line = String::new();

        if stdin.read_line(&mut line)? == 0 {
            bail!("No profile selected, pass `--to <profile>` to select one");
        }

        let line = line.trim();

        if let Ok(index) = line.parse::<usize>()
            && let Some(name) = index.checked_sub(1).and_then(|index| names.get(index))
        {
            return Ok(name);
        }

        if let Some(name) = names.iter().find(|name| **name == line) {
            return Ok(name);
        }

        writeln!(stderr, "No such profile `{line}`")?;
    }
}

/// Warn about options which have no effect on a local deployment.
fn warn_ignored_for_local(layer: &Deploy, opts: &Opts, what: &str) {
    let mut ignored = Vec::new();

    if !layer.host.is_empty() {
        ignored.push("host");
    }

    if layer.user.is_some() {
        ignored.push("user");
    }

    if layer.port.is_some() {
        ignored.push("port");
    }

    if layer.identity_file.is_some() {
        ignored.push("identity_file");
    }

    if !layer.options.is_empty() {
        ignored.push("options");
    }

    if layer.staging_dir.is_some() {
        ignored.push("staging_dir");
    }

    if !ignored.is_empty() {
        tracing::warn!(
            "Ignoring {} in {what} since the deployment is local",
            ignored
                .iter()
                .map(|key| format!("`{key}`"))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    if !opts.host.is_empty() {
        tracing::warn!("Ignoring `--host` since the deployment is local");
    }

    if opts.user.is_some() {
        tracing::warn!("Ignoring `--user` since the deployment is local");
    }

    if opts.no_check {
        tracing::warn!(
            "Ignoring `--no-check` since there is no access check for a local deployment"
        );
    }
}

/// Expand a leading `~`, `$HOME` or `${HOME}` in a path to the given home
/// directory.
///
/// Returns `Ok(None)` if there is nothing to expand, and `Err(())` if there is
/// but the home directory is not known.
fn expand_home(value: &str, home: Option<&str>) -> Result<Option<String>, ()> {
    let rest = ["~", "${HOME}", "$HOME"].into_iter().find_map(|prefix| {
        let rest = value.strip_prefix(prefix)?;
        (rest.is_empty() || rest.starts_with('/')).then_some(rest)
    });

    let Some(rest) = rest else {
        return Ok(None);
    };

    let Some(home) = home else {
        return Err(());
    };

    Ok(Some(format!("{}{rest}", home.trim_end_matches('/'))))
}

/// Expand the home directory in every string in the given value.
fn expand_value(value: &mut toml::Value, home: Option<&str>) -> Result<(), String> {
    match value {
        toml::Value::String(string) => match expand_home(string, home) {
            Ok(Some(expanded)) => *string = expanded,
            Ok(None) => {}
            Err(()) => return Err(string.clone()),
        },
        toml::Value::Array(values) => {
            for value in values {
                expand_value(value, home)?;
            }
        }
        toml::Value::Table(table) => {
            for (_, value) in table.iter_mut() {
                expand_value(value, home)?;
            }
        }
        _ => {}
    }

    Ok(())
}

/// The home directory of the user running `kick`.
fn local_home() -> Option<String> {
    if let Some(home) = std::env::var_os("HOME")
        && !home.is_empty()
    {
        return Some(home.to_string_lossy().into_owned());
    }

    let dirs = directories::BaseDirs::new()?;
    Some(dirs.home_dir().to_string_lossy().into_owned())
}

/// Run an install or a deployment, depending on the section which configures
/// it.
#[tracing::instrument(skip_all)]
pub(crate) fn deploy(
    o: &mut StandardStream,
    cx: &Ctxt<'_>,
    opts: &Opts,
    repo: &Repo,
    section: Section,
) -> Result<()> {
    let base = match section {
        Section::Install => cx.config.install(repo),
        Section::Deploy => cx.config.deploy(repo),
    };

    let interactive = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();

    let selected = match choose_profile(&base, section, opts.to.as_deref(), interactive)? {
        Choice::Base => None,
        Choice::Profile(name) => Some(name.to_owned()),
        Choice::Ask(names) => Some(ask_profile(&names)?.to_owned()),
    };

    let config = match &selected {
        Some(name) => base
            .with_profile(name)
            .with_context(|| anyhow!("Missing profile `{name}`"))?,
        None => base.clone(),
    };

    // NB: An install always happens on the machine kick is running on.
    let kind = match section {
        Section::Install => DeployKind::Local,
        Section::Deploy => config.kind.unwrap_or_default(),
    };

    let systemd_config = config.systemd.clone().unwrap_or_default();

    // NB: Installing locally happens as the user running kick, whose own
    // systemd instance is the one which needs neither root nor sudo.
    let scope = systemd_config.scope.unwrap_or(match kind {
        DeployKind::Ssh => SystemdScope::System,
        DeployKind::Local => SystemdScope::User,
    });

    let mut build = cx.config.build(repo);
    build.merge_with(config.build.clone());
    let build = build;

    let targets = match kind {
        DeployKind::Ssh => ssh_targets(&config, selected.as_deref(), opts)?,
        DeployKind::Local => {
            match &selected {
                Some(name) => {
                    if let Some(profile) = base.profiles.get(name) {
                        warn_ignored_for_local(
                            profile,
                            opts,
                            &format!("`[{}.profiles.{name}]`", section.as_str()),
                        );
                    }
                }
                None => warn_ignored_for_local(&config, opts, &format!("`[{}]`", section.as_str())),
            }

            vec![Target::local()]
        }
    };

    let root = cx.to_path(repo.path());

    // NB: Deploying a service without a unit to run it is rarely what anyone
    // wants, so the built-in template applies unless it is turned off. Most
    // things which are installed are not services, so an install only has a
    // unit when it is configured with one.
    let systemd_default = match section {
        Section::Install => config.systemd.is_some(),
        Section::Deploy => true,
    };

    let systemd = (systemd_config.enabled.unwrap_or(systemd_default) && !opts.no_systemd)
        .then_some(systemd_config);

    if systemd.is_none() {
        if opts.service_user.is_some() {
            tracing::warn!("Ignoring `--service-user` since no systemd unit is being installed");
        }

        if opts.group.is_some() {
            tracing::warn!("Ignoring `--group` since no systemd unit is being installed");
        }

        if !opts.args.is_empty() {
            tracing::warn!("Ignoring `--args` since no systemd unit is being installed");
        }
    }

    // NB: Installing locally happens as the user running kick, which is
    // rarely someone who wants to elevate.
    let use_sudo = config.sudo.unwrap_or(kind == DeployKind::Ssh);

    // NB: Access is checked before anything is built, since discovering that we
    // cannot log in after a lengthy build is not very helpful. Every host is
    // checked up front for the same reason, so a fleet which cannot be fully
    // deployed to says so before the first host is touched.
    //
    // The check also reports the home directory of the user being logged in
    // as, which is what a leading `~` in a path expands to.
    let mut homes = Vec::with_capacity(targets.len());

    match kind {
        DeployKind::Ssh => {
            for target in &targets {
                if opts.no_check {
                    homes.push(None);
                } else {
                    homes.push(check(
                        o,
                        opts,
                        &config,
                        target,
                        use_sudo,
                        systemd.is_some(),
                    )?);
                }
            }
        }
        DeployKind::Local => {
            homes.push(local_home());
        }
    }

    let profile = opts
        .profile
        .as_deref()
        .or(build.profile.as_deref())
        .unwrap_or(DEFAULT_PROFILE);

    let package = opts.package.as_deref().or(build.package.as_deref());

    // NB: Commands replace building and installing the binary, so a binary is
    // only needed when it is installed or when a unit runs it.
    let custom = !config.commands.is_empty();

    let binary = match opts
        .binary
        .as_deref()
        .or(build.binary.as_deref())
        .or(package)
    {
        Some(binary) => Some(binary.to_owned()),
        None if !custom || systemd.is_some() => Some(default_binary(cx, repo)?),
        None => None,
    };

    let manifest_dir = manifest_dir(cx, repo);

    if !opts.no_build {
        pre_build(o, opts, &build, &root)?;

        if custom {
            for command in &config.commands {
                run(o, opts, &mut command.to_command(&root))?;
            }
        } else {
            cargo_build(o, opts, &build, &root, &manifest_dir, profile, package)?;
        }
    }

    // NB: The binary which is installed, if kick installs it rather than the
    // configured commands.
    let binary_path = match (&binary, custom) {
        (Some(binary), false) => {
            let mut path = target_dir(&manifest_dir);
            path.push(profile_dir(profile));
            path.push(binary);
            path.set_extension(EXE_EXTENSION);

            // NB: The build is only printed during a dry run, so the binary is
            // only required to exist when it will actually be installed.
            if !path.is_file() {
                if opts.dry_run && !opts.no_build {
                    tracing::warn!(
                        "Missing binary to install: {} (it would be built first)",
                        path.display()
                    );
                } else {
                    bail!("Missing binary to install: {}", path.display());
                }
            }

            Some(path)
        }
        _ => None,
    };

    let installed_binary = binary_path.as_ref().and(binary.as_deref());

    let default_unit_dir = match scope {
        SystemdScope::System => DEFAULT_UNIT_DIR,
        SystemdScope::User => DEFAULT_USER_UNIT_DIR,
    };

    let default_bin_dir = match kind {
        DeployKind::Ssh => DEFAULT_BIN_DIR.to_owned(),
        DeployKind::Local => cargo_bin_dir(),
    };

    let bin_dir = trim_dir(
        opts.bin_dir
            .as_deref()
            .or(config.bin_dir.as_deref())
            .unwrap_or(&default_bin_dir),
    );
    let unit_dir = trim_dir(config.unit_dir.as_deref().unwrap_or(default_unit_dir));
    let staging_dir = trim_dir(config.staging_dir.as_deref().unwrap_or(DEFAULT_STAGING_DIR));

    // Files to upload, as `(local path, staged file name)`. The unit is added
    // per host, since it is rendered for the host it is being installed on.
    let mut uploads = Vec::new();
    // Files to install, as `(staged file name, destination, mode)`.
    let mut installs = Vec::new();

    if let (Some(path), Some(binary)) = (&binary_path, installed_binary) {
        uploads.push((path.clone(), binary.to_owned()));
    }

    for file in &config.files {
        let glob = Glob::new(&root, &file.source);
        let mut matched = false;

        for source in glob.matcher() {
            let relative = source?;

            let Some(file_name) = relative.file_name() else {
                bail!("Missing file name: {relative}");
            };

            let path = cx.to_path(repo.path().join(&relative));

            let dest = if file.dest.ends_with('/') {
                format!("{}{file_name}", file.dest)
            } else {
                file.dest.clone()
            };

            let mode = match file.mode {
                Some(mode) => mode,
                None => packaging::infer(&path)?.mode,
            };

            installs.push((file_name.to_owned(), dest, mode));
            uploads.push((path, file_name.to_owned()));
            matched = true;
        }

        if !matched {
            bail!("No files matched: {}", file.source);
        }
    }

    // NB: A local deployment installs straight from where the files are, which
    // is only unambiguous with absolute paths since the script is not
    // necessarily run from the repo.
    if kind == DeployKind::Local {
        for (path, _) in &mut uploads {
            *path = std::path::absolute(&*path)
                .with_context(|| anyhow!("Making {} absolute", path.display()))?;
        }
    }

    let unit_name = match (&systemd, &binary) {
        (Some(systemd), Some(binary)) => Some(systemd.name.as_deref().unwrap_or(binary).to_owned()),
        _ => None,
    };

    // NB: A socket unit is only installed alongside the service it activates,
    // and is named after it unless told otherwise, which is what lets systemd
    // pair them up without a `Service=` directive.
    let socket = systemd
        .as_ref()
        .and_then(|systemd| systemd.socket.as_ref())
        .filter(|socket| socket.enabled.unwrap_or(true));

    let socket_name = match (socket, &unit_name) {
        (Some(socket), Some(name)) => Some(socket.name.as_deref().unwrap_or(name).to_owned()),
        _ => None,
    };

    let mut staged = HashMap::new();

    for (path, name) in &uploads {
        if let Some(existing) = staged.insert(name.clone(), path.clone()) {
            bail!(
                "Multiple files would be staged as `{name}`: {} and {}",
                existing.display(),
                path.display()
            );
        }
    }

    // NB: The unit is generated as part of the deployment instead of being one
    // of the files being uploaded, so it is checked on its own.
    if let Some(name) = &unit_name {
        let file_name = format!("{name}.service");

        if let Some(existing) = staged.get(&file_name) {
            bail!(
                "The systemd unit and {} would both be staged as `{file_name}`",
                existing.display()
            );
        }
    }

    if let Some(name) = &socket_name {
        let file_name = format!("{name}.socket");

        if let Some(existing) = staged.get(&file_name) {
            bail!(
                "The systemd socket unit and {} would both be staged as `{file_name}`",
                existing.display()
            );
        }
    }

    // The unit file is generated locally into a temporary directory so that it
    // can be uploaded under its expected name. Each host gets its own
    // directory, since the unit is rendered for the host it belongs to.
    let temp = tempfile::TempDir::new().context("Creating temporary directory")?;

    for (index, (target, home)) in targets.iter().zip(&homes).enumerate() {
        let home = home.as_deref();

        let expand = |value: &str| -> Result<String> {
            match expand_home(value, home) {
                Ok(Some(expanded)) => Ok(expanded),
                Ok(None) => Ok(value.to_owned()),
                Err(()) => Err(missing_home(target, value)),
            }
        };

        let bin_dir = expand(bin_dir)?;
        let unit_dir = expand(unit_dir)?;
        let staging_dir = expand(staging_dir)?;

        let installs = installs
            .iter()
            .map(|(name, dest, mode)| Ok((name.clone(), expand(dest)?, *mode)))
            .collect::<Result<Vec<_>>>()?;

        let mut uploads = uploads.clone();

        let mut socket_unit = None;

        let unit = match (&systemd, &unit_name, &binary) {
            (Some(systemd), Some(name), Some(binary)) => {
                let file_name = format!("{name}.service");
                let dir = temp.path().join(index.to_string());

                std::fs::create_dir_all(&dir)
                    .with_context(|| anyhow!("Creating {}", dir.display()))?;

                // NB: Both units are rendered with the same set of built-in
                // variables, but each with its own name.
                let builtins = |name: &str| -> toml::Table {
                    let mut variables = toml::Table::new();
                    variables.insert(String::from("name"), toml::Value::String(name.to_owned()));
                    variables.insert(String::from("binary"), toml::Value::String(binary.clone()));
                    variables.insert(
                        String::from("exec"),
                        toml::Value::String(format!("{bin_dir}/{binary}")),
                    );
                    variables.insert(
                        String::from("bin_dir"),
                        toml::Value::String(bin_dir.clone()),
                    );
                    variables.insert(
                        String::from("unit_dir"),
                        toml::Value::String(unit_dir.clone()),
                    );
                    variables.insert(
                        String::from("host"),
                        toml::Value::String(target.host.clone()),
                    );
                    variables.insert(
                        String::from("scope"),
                        toml::Value::String(scope.as_str().to_owned()),
                    );
                    variables
                };

                // NB: A user unit runs as the user being deployed as, so a `~`
                // means the same thing to it as it does to us. A system unit
                // runs as whatever `User=` says, and systemd resolves a `~` in
                // `WorkingDirectory=` against that user, so it is left alone.
                let expand_variables = |variables: &mut toml::Table| -> Result<()> {
                    if scope == SystemdScope::User {
                        for (_, value) in variables.iter_mut() {
                            if let Err(value) = expand_value(value, home) {
                                return Err(missing_home(target, &value));
                            }
                        }
                    }

                    Ok(())
                };

                let socket_file_name = socket_name.as_ref().map(|name| format!("{name}.socket"));

                if let (Some(socket), Some(socket_name), Some(socket_file_name)) =
                    (socket, &socket_name, &socket_file_name)
                {
                    // NB: The socket is rendered with its own variables rather
                    // than those of the service, since a `Description=` or
                    // `WantedBy=` meant for one is wrong for the other.
                    let mut variables = socket.variables.clone();
                    expand_variables(&mut variables)?;
                    variables.extend(builtins(socket_name));
                    variables.insert(
                        String::from("service"),
                        toml::Value::String(file_name.clone()),
                    );

                    let template = socket
                        .template
                        .as_deref()
                        .unwrap_or(systemd::DEFAULT_SOCKET_TEMPLATE);

                    let contents = systemd::render(template, &variables).with_context(|| {
                        anyhow!("Rendering unit `{socket_file_name}` for `{}`", target.ssh)
                    })?;

                    let path = dir.join(socket_file_name);

                    std::fs::write(&path, &contents)
                        .with_context(|| anyhow!("Writing {}", path.display()))?;

                    uploads.push((path, socket_file_name.clone()));
                    socket_unit = Some((socket_file_name.clone(), contents));
                }

                // NB: The unit is rendered with the variables which are scoped
                // to it in the `[deploy.systemd]` section, since a unit
                // directive is not something anything else in the
                // configuration has any use for.
                let mut variables = systemd.variables.clone();
                expand_variables(&mut variables)?;
                variables.extend(builtins(name));

                if let Some(socket_file_name) = &socket_file_name {
                    variables.insert(
                        String::from("socket"),
                        toml::Value::String(socket_file_name.clone()),
                    );
                }

                // NB: Which user a service runs as and what it is started with
                // are things a deployment which has no configuration at all
                // still needs to be able to say.
                if let Some(user) = &opts.service_user {
                    variables.insert(String::from("user"), toml::Value::String(user.clone()));
                }

                // NB: A service which runs as a dedicated user conventionally
                // has a group of the same name, but a configured group is not
                // something to be overridden by that convention.
                let group = match &opts.group {
                    Some(group) => Some(group),
                    None if !variables.contains_key("group") => opts.service_user.as_ref(),
                    None => None,
                };

                if let Some(group) = group {
                    variables.insert(String::from("group"), toml::Value::String(group.clone()));
                }

                let args = opts
                    .args
                    .iter()
                    .flat_map(|args| args.split_whitespace())
                    .map(|arg| toml::Value::String(arg.to_owned()))
                    .collect::<Vec<_>>();

                if !args.is_empty() {
                    variables.insert(String::from("args"), toml::Value::Array(args));
                }

                let template = systemd
                    .template
                    .as_deref()
                    .unwrap_or(systemd::DEFAULT_TEMPLATE);

                let contents = systemd::render(template, &variables).with_context(|| {
                    anyhow!("Rendering unit `{file_name}` for `{}`", target.ssh)
                })?;

                let path = dir.join(&file_name);

                std::fs::write(&path, &contents)
                    .with_context(|| anyhow!("Writing {}", path.display()))?;

                uploads.push((path, file_name.clone()));
                Some((name.clone(), file_name, contents))
            }
            _ => None,
        };

        // NB: With `commands` installing the binary there might be nothing
        // left for the script to do.
        if installed_binary.is_none()
            && installs.is_empty()
            && unit.is_none()
            && config.post_install.is_empty()
            && config.post_start.is_empty()
        {
            continue;
        }

        let sources = match kind {
            DeployKind::Ssh => Sources::Staged(&staging_dir),
            DeployKind::Local => Sources::Local(&uploads),
        };

        let script = script(
            &config,
            opts,
            &installs,
            ScriptOpts {
                sudo: match (use_sudo, kind) {
                    (false, _) => "",
                    // NB: The script is run non-interactively over ssh, so
                    // `-n` is used to make sudo fail immediately with a
                    // diagnostic instead of trying to prompt for a password
                    // on a terminal which isn't there.
                    (true, DeployKind::Ssh) => "sudo -n ",
                    // NB: A local script runs on our terminal, where sudo can
                    // prompt like it usually does.
                    (true, DeployKind::Local) => "sudo ",
                },
                scope,
                binary: installed_binary,
                bin_dir: &bin_dir,
                unit_dir: &unit_dir,
                sources,
                unit: unit
                    .as_ref()
                    .map(|(name, file_name, _)| (&**name, &**file_name)),
                socket: socket_unit.as_ref().map(|(file_name, _)| &**file_name),
            },
        )?;

        if opts.details() {
            let mut plan = Vec::new();

            if let Some(name) = &selected {
                plan.push(format!("profile: {name}"));
            }

            if !base.profiles.is_empty() {
                plan.push(format!(
                    "profiles: {}",
                    base.profiles.keys().cloned().collect::<Vec<_>>().join(", ")
                ));
            }

            if section == Section::Deploy {
                plan.push(format!("kind: {kind}"));
            }

            if kind == DeployKind::Ssh {
                plan.push(format!("host: {}", target.host));

                if let Some(user) = &target.user {
                    plan.push(format!("user: {user}"));
                }

                if let Some(port) = config.port {
                    plan.push(format!("port: {port}"));
                }
            }

            if let Some(binary) = &binary {
                plan.push(format!("binary: {binary}"));
            }

            if custom {
                plan.push(String::from("build: replaced by `commands`"));
            } else {
                if let Some(package) = package {
                    plan.push(format!("package: {package}"));
                }

                plan.push(format!("cargo profile: {profile}"));
            }

            plan.push(format!("sudo: {}", if use_sudo { "yes" } else { "no" }));
            plan.push(format!("bin_dir: {bin_dir}"));

            if let Some((_, file_name, _)) = &unit {
                plan.push(format!("unit_dir: {unit_dir}"));
                plan.push(format!("unit: {file_name}"));

                if let Some((socket_file_name, _)) = &socket_unit {
                    plan.push(format!("socket: {socket_file_name}"));
                }

                plan.push(format!("scope: {scope}"));
            }

            if kind == DeployKind::Ssh {
                plan.push(format!("staging_dir: {staging_dir}"));
            }

            let title = match section {
                Section::Install => "install",
                Section::Deploy => "deployment",
            };

            details(o, title, plan.iter().map(String::as_str))?;

            if kind == DeployKind::Ssh {
                let uploaded = uploads
                    .iter()
                    .map(|(path, name)| format!("{} -> {staging_dir}/{name}", path.display()))
                    .collect::<Vec<_>>();

                details(
                    o,
                    "uploads (streamed as a tar archive to the remote script)",
                    uploaded.iter().map(String::as_str),
                )?;
            }

            let from = |name: &str| sources.display(name);

            let mut installed = Vec::new();

            if let Some(binary) = installed_binary {
                installed.push(format!("{} -> {bin_dir}/{binary} (0755)", from(binary)));
            }

            for (name, dest, mode) in &installs {
                installed.push(format!(
                    "{} -> {dest} ({:04o})",
                    from(name),
                    mode.permissions()
                ));
            }

            if let Some((_, file_name, _)) = &unit {
                installed.push(format!(
                    "{} -> {unit_dir}/{file_name} (0644)",
                    from(file_name)
                ));
            }

            if let Some((file_name, _)) = &socket_unit {
                installed.push(format!(
                    "{} -> {unit_dir}/{file_name} (0644)",
                    from(file_name)
                ));
            }

            details(o, "installs", installed.iter().map(String::as_str))?;

            if let Some((_, file_name, contents)) = &unit {
                details(o, &format!("{unit_dir}/{file_name}"), contents.lines())?;
            }

            if let Some((file_name, contents)) = &socket_unit {
                details(o, &format!("{unit_dir}/{file_name}"), contents.lines())?;
            }

            let title = match kind {
                DeployKind::Ssh => "remote script",
                DeployKind::Local => "local script",
            };

            details(o, title, script.lines())?;
        }

        match kind {
            DeployKind::Ssh => {
                // NB: Everything happens over a single connection. The files
                // are streamed as a tar archive over the stdin of the remote
                // script, which unpacks them into the staging directory before
                // it installs anything.
                let mut command = ssh(opts, &config, target);
                command.arg(&script);
                run_with_payload(o, opts, &mut command, &uploads)?;
            }
            DeployKind::Local => {
                let mut command = Command::new("sh");
                command.arg("-c");
                command.arg(&script);
                command.current_dir(&root);
                run(o, opts, &mut command)?;
            }
        }
    }

    Ok(())
}

/// The error raised when a path starts with `~` but the home directory it
/// refers to is not known.
fn missing_home(target: &Target, value: &str) -> anyhow::Error {
    if target.local {
        anyhow!("Cannot expand `{value}` since the home directory is not known, set `HOME`")
    } else {
        anyhow!(
            "Cannot expand `{value}` since the home directory on `{}` is not known. It is determined by the access check, so either don't pass `--no-check` or use an absolute path",
            target.ssh
        )
    }
}

/// The hosts an ssh deployment goes to, from `--host` or else the `host`
/// option of the deployment being performed.
///
/// A configuration can leave the host out entirely, in which case it has to be
/// given with `--host`, so the same profile can be deployed to any machine.
fn ssh_targets(config: &Deploy, selected: Option<&str>, opts: &Opts) -> Result<Vec<Target>> {
    // NB: Hosts given on the command line replace the configured ones rather
    // than adding to them, since `--host` is how you deploy somewhere other
    // than where the project usually goes.
    let hosts = if opts.host.is_empty() {
        &config.host[..]
    } else {
        &opts.host[..]
    };

    if hosts.is_empty() {
        match selected {
            Some(name) => bail!(
                "Missing host to deploy profile `{name}` to, deploy with `kick deploy --to {name} --host <host>` or set `host` in the `[deploy.profiles.{name}]` section"
            ),
            None => bail!(
                "Missing host to deploy to, deploy with `kick deploy --host <host>` or set `host` in the `[deploy]` section"
            ),
        }
    }

    let login = opts.user.as_deref().or(config.user.as_deref());

    Ok(hosts.iter().map(|host| Target::new(login, host)).collect())
}

/// A host being deployed to, along with the user we log into it as.
#[derive(Debug)]
struct Target {
    /// The argument handed to `ssh` and `scp`, which is `<user>@<host>` when
    /// there is a user to log in as.
    ssh: String,
    /// The host on its own, without any login user.
    host: String,
    /// The user we expect to end up as after logging in, if we know it.
    user: Option<String>,
    /// Whether this is the local machine.
    local: bool,
}

impl Target {
    /// Combine a configured login user with a host.
    ///
    /// A user which is spelled out as part of the host wins, since it is the
    /// more specific of the two.
    fn new(login: Option<&str>, host: &str) -> Self {
        // NB: ssh separates the user from the host at the last `@`.
        if let Some((user, bare)) = host.rsplit_once('@') {
            return Self {
                ssh: host.to_owned(),
                host: bare.to_owned(),
                user: Some(user.to_owned()),
                local: false,
            };
        }

        let Some(login) = login else {
            return Self {
                ssh: host.to_owned(),
                host: host.to_owned(),
                user: None,
                local: false,
            };
        };

        Self {
            ssh: format!("{login}@{host}"),
            host: host.to_owned(),
            user: Some(login.to_owned()),
            local: false,
        }
    }

    /// The machine kick is running on.
    fn local() -> Self {
        Self {
            ssh: String::from(LOCAL_HOST),
            host: String::from(LOCAL_HOST),
            user: None,
            local: true,
        }
    }
}

/// Where the script installs files from.
#[derive(Clone, Copy)]
enum Sources<'a> {
    /// Files have been uploaded into the given staging directory.
    Staged(&'a str),
    /// Files are installed from where they are, as `(path, staged name)`.
    Local(&'a [(PathBuf, String)]),
}

impl Sources<'_> {
    /// The path a file is installed from.
    fn display(&self, name: &str) -> String {
        match self {
            Sources::Staged(dir) => format!("{dir}/{name}"),
            Sources::Local(files) => files
                .iter()
                .find(|(_, n)| n == name)
                .map(|(path, _)| path.display().to_string())
                .unwrap_or_else(|| name.to_owned()),
        }
    }
}

struct ScriptOpts<'a> {
    sudo: &'a str,
    scope: SystemdScope,
    /// The binary being installed, unless it is installed by `commands`.
    binary: Option<&'a str>,
    bin_dir: &'a str,
    unit_dir: &'a str,
    sources: Sources<'a>,
    unit: Option<(&'a str, &'a str)>,
    /// The file name of the socket unit which activates the service, if any.
    socket: Option<&'a str>,
}

/// Build the script which installs the deployed files.
fn script(
    config: &Deploy,
    opts: &Opts,
    installs: &[(String, String, Mode)],
    s: ScriptOpts<'_>,
) -> Result<String> {
    let shell = Shell::Bash;

    let ScriptOpts {
        sudo,
        scope,
        binary,
        bin_dir,
        unit_dir,
        sources,
        unit,
        socket,
    } = s;

    let escape = move |value: &str| shell.escape(value).into_owned();
    let source = move |name: &str| escape(&sources.display(name));

    // NB: A user unit belongs to the user being deployed as, so neither it nor
    // its manager has any use for sudo, and `sudo systemctl --user` would talk
    // to the wrong manager.
    let (unit_sudo, systemctl) = match scope {
        SystemdScope::System => (sudo, format!("{sudo}systemctl")),
        SystemdScope::User => ("", String::from("systemctl --user")),
    };

    // NB: The commands are part of starting the service, so a deployment
    // which leaves the service alone skips them too. They are command lines
    // for the shell running the script, so they are used as written and the
    // sudo prefix goes in front of them. That way the shell expands `~` and
    // `$HOME` for the user being deployed as before sudo runs.
    let commands = |script: &mut String, commands: &[ConfigCommand]| -> Result<()> {
        if opts.no_restart {
            return Ok(());
        }

        for c in commands {
            let sudo = if c.sudo { sudo } else { "" };
            writeln!(script, "{sudo}{}", c.to_shell(shell))?;
        }

        Ok(())
    };

    let mut script = String::new();

    // NB: Tracing the script is the only insight into the remote half of the
    // deployment, since it is run non-interactively over ssh.
    if opts.verbose >= 1 {
        writeln!(script, "set -eux")?;
    } else {
        writeln!(script, "set -eu")?;
    }

    // NB: The staged files arrive as a tar archive on stdin, and are unpacked
    // before anything else so that a failed transfer leaves the running
    // service alone.
    if let Sources::Staged(dir) = sources {
        writeln!(script, "mkdir -p {}", shell.escape(dir))?;
        writeln!(script, "tar -x -f - -C {}", shell.escape(dir))?;
    }

    let socket_dest = socket.map(|file_name| escape(&format!("{unit_dir}/{file_name}")));

    match (unit, socket.zip(socket_dest.as_deref())) {
        // NB: A socket unit is compared up front, since whether it changed
        // decides whether the socket has to be stopped. A socket which is
        // already active and unchanged is left alone, so that whatever is
        // listening on it keeps working while the binary is replaced, and the
        // service is restarted once it is in place.
        (Some((name, _)), Some((file_name, dest))) => {
            writeln!(script, "socket_changed=no")?;
            writeln!(
                script,
                "if ! {unit_sudo}cmp -s {} {dest}; then",
                source(file_name)
            )?;
            writeln!(script, "  socket_changed=yes")?;
            writeln!(script, "fi")?;

            if !opts.no_restart {
                writeln!(script, "if [ \"$socket_changed\" = yes ]; then")?;
                writeln!(
                    script,
                    "  {systemctl} stop {} 2>/dev/null || true",
                    shell.escape(name)
                )?;
                writeln!(
                    script,
                    "  {systemctl} stop {} 2>/dev/null || true",
                    shell.escape(file_name)
                )?;
                writeln!(script, "fi")?;
            }
        }
        // Stop the service before its binary is replaced, the unit might not
        // exist yet in which case this is a no-op.
        (Some((name, _)), None) if !opts.no_restart => {
            writeln!(
                script,
                "{systemctl} stop {} 2>/dev/null || true",
                shell.escape(name)
            )?;
        }
        _ => {}
    }

    if let Some(binary) = binary {
        writeln!(script, "{sudo}mkdir -p {}", shell.escape(bin_dir))?;

        writeln!(
            script,
            "{sudo}install -m 0755 {} {}",
            source(binary),
            escape(&format!("{bin_dir}/{binary}"))
        )?;
    }

    for (name, dest, mode) in installs {
        writeln!(
            script,
            "{sudo}install -D -m {:04o} {} {}",
            mode.permissions(),
            source(name),
            shell.escape(dest)
        )?;
    }

    if let Some((name, file_name)) = unit {
        let dest = escape(&format!("{unit_dir}/{file_name}"));

        writeln!(script, "{unit_sudo}mkdir -p {}", shell.escape(unit_dir))?;

        let enable = config
            .systemd
            .as_ref()
            .and_then(|s| s.enable)
            .unwrap_or(true);

        if let (Some(socket), Some(socket_dest)) = (socket, &socket_dest) {
            // NB: With two units which might change, systemd is reloaded once
            // after both have been installed.
            writeln!(script, "reload=no")?;

            writeln!(
                script,
                "if ! {unit_sudo}cmp -s {} {dest}; then",
                source(file_name)
            )?;
            writeln!(
                script,
                "  {unit_sudo}install -m 0644 {} {dest}",
                source(file_name)
            )?;
            writeln!(script, "  reload=yes")?;
            writeln!(script, "fi")?;

            writeln!(script, "if [ \"$socket_changed\" = yes ]; then")?;
            writeln!(
                script,
                "  {unit_sudo}install -m 0644 {} {socket_dest}",
                source(socket)
            )?;
            writeln!(script, "  reload=yes")?;
            writeln!(script, "fi")?;

            writeln!(script, "if [ \"$reload\" = yes ]; then")?;
            writeln!(script, "  {systemctl} daemon-reload")?;
            writeln!(script, "fi")?;

            commands(&mut script, &config.post_install)?;

            // NB: It is the socket which is enabled rather than the service,
            // since the service is started by connections to the socket.
            match (enable, opts.no_restart) {
                (true, false) => {
                    writeln!(script, "{systemctl} enable --now {}", shell.escape(socket))?;
                }
                (true, true) => {
                    writeln!(script, "{systemctl} enable {}", shell.escape(socket))?;
                }
                (false, false) => {
                    writeln!(script, "{systemctl} start {}", shell.escape(socket))?;
                }
                (false, true) => {}
            }

            // NB: The service is only stopped up front if the socket changed,
            // so it is restarted to pick up the new binary.
            if !opts.no_restart {
                writeln!(script, "{systemctl} restart {}", shell.escape(name))?;
            }

            commands(&mut script, &config.post_start)?;
        } else {
            // NB: Installing the unit unconditionally would touch it on every
            // deployment, so only do it when it actually changed. This also
            // keeps us from reloading systemd for no reason.
            writeln!(
                script,
                "if ! {unit_sudo}cmp -s {} {dest}; then",
                source(file_name)
            )?;

            writeln!(
                script,
                "  {unit_sudo}install -m 0644 {} {dest}",
                source(file_name)
            )?;

            writeln!(script, "  {systemctl} daemon-reload")?;
            writeln!(script, "fi")?;

            commands(&mut script, &config.post_install)?;

            if enable {
                writeln!(script, "{systemctl} enable {}", shell.escape(name))?;
            }

            if !opts.no_restart {
                writeln!(script, "{systemctl} start {}", shell.escape(name))?;
            }

            commands(&mut script, &config.post_start)?;
        }
    } else {
        // NB: Without a unit there is nothing to start, so the commands run
        // back to back once everything has been installed.
        commands(&mut script, &config.post_install)?;
        commands(&mut script, &config.post_start)?;
    }

    // NB: Only staged copies are removed, a local deployment installs from the
    // originals.
    if let Sources::Staged(dir) = sources {
        let mut names = Vec::from_iter(binary);
        names.extend(installs.iter().map(|(name, _, _)| name.as_str()));
        names.extend(unit.map(|(_, file_name)| file_name));
        names.extend(socket);

        for name in names {
            writeln!(script, "rm -f {}", escape(&format!("{dir}/{name}")))?;
        }
    }

    Ok(script)
}

/// Run the `pre_build` commands, followed by any passed with `--pre-build`.
fn pre_build(o: &mut StandardStream, opts: &Opts, build: &Build, root: &Path) -> Result<()> {
    let extra = opts
        .pre_build
        .iter()
        .filter_map(|c| ConfigCommand::split(c));

    for pre_build in build.pre_build.iter().cloned().chain(extra) {
        run(o, opts, &mut pre_build.to_command(root))?;
    }

    Ok(())
}

/// Build the project.
///
/// The build command is generated from the profile, the package and the
/// features being enabled unless it has been replaced through `commands`.
fn cargo_build(
    o: &mut StandardStream,
    opts: &Opts,
    build: &Build,
    root: &Path,
    manifest_dir: &Path,
    profile: &str,
    package: Option<&str>,
) -> Result<()> {
    let features = build
        .features
        .iter()
        .chain(&opts.features)
        .flat_map(|f| f.split([',', ' ']))
        .filter(|f| !f.is_empty())
        .collect::<Vec<_>>()
        .join(",");

    if !build.commands.is_empty() {
        if !features.is_empty() {
            tracing::warn!(
                "Ignoring features `{features}` since the build command is replaced by `commands`"
            );
        }

        for command in &build.commands {
            run(o, opts, &mut command.to_command(root))?;
        }

        return Ok(());
    }

    let mut command = Command::new("cargo");
    command.arg("build");

    match profile {
        "dev" => {}
        "release" => {
            command.arg("--release");
        }
        profile => {
            command.arg("--profile");
            command.arg(profile);
        }
    }

    if let Some(package) = package {
        command.arg("--package");
        command.arg(package);
    }

    if !features.is_empty() {
        command.arg("--features");
        command.arg(&features);
    }

    command.current_dir(manifest_dir);
    run(o, opts, &mut command)
}

/// The binary installed when none is configured, which is the name of the
/// primary crate of the project, or else the package at its root.
fn default_binary(cx: &Ctxt<'_>, repo: &Repo) -> Result<String> {
    let workspace = repo.workspace(cx)?;

    let manifest = match workspace.primary_package() {
        Ok(manifest) => manifest,
        Err(error) => match workspace.manifests().next() {
            Some(manifest) if manifest.is_package() => manifest,
            _ => {
                return Err(error.context(
                    "Cannot determine the binary to install, set `binary` in the `[build]` section",
                ));
            }
        },
    };

    Ok(manifest.ensure_package()?.name()?.to_owned())
}

/// The directory cargo is run in, which is where `cargo_toml` points to if it
/// is set.
fn manifest_dir(cx: &Ctxt<'_>, repo: &Repo) -> PathBuf {
    let path = match cx
        .config
        .cargo_toml(repo)
        .and_then(|manifest| manifest.parent())
    {
        Some(dir) => repo.path().join(dir),
        None => repo.path().to_owned(),
    };

    cx.to_path(path)
}

/// The target directory cargo builds into.
///
/// This asks cargo, since a workspace, `CARGO_TARGET_DIR` and the
/// `build.target-dir` setting all change it, and falls back to `target` in
/// the manifest directory.
fn target_dir(manifest_dir: &Path) -> PathBuf {
    let output = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(manifest_dir)
        .stderr(Stdio::null())
        .output();

    #[derive(serde::Deserialize)]
    struct Metadata {
        target_directory: PathBuf,
    }

    if let Ok(output) = output
        && output.status.success()
        && let Ok(metadata) = serde_json::from_slice::<Metadata>(&output.stdout)
    {
        return metadata.target_directory;
    }

    manifest_dir.join("target")
}

/// The directory cargo installs binaries into, which is where something
/// installed on the machine kick runs on goes by default.
fn cargo_bin_dir() -> String {
    match std::env::var("CARGO_HOME") {
        Ok(home) if !home.is_empty() => format!("{}/bin", home.trim_end_matches('/')),
        _ => String::from(DEFAULT_CARGO_BIN_DIR),
    }
}

/// The directory under `target` which cargo puts the given profile in.
fn profile_dir(profile: &str) -> &str {
    match profile {
        "dev" | "test" => "debug",
        "bench" => "release",
        profile => profile,
    }
}

/// Construct an `ssh` command towards the given host.
fn ssh(opts: &Opts, config: &Deploy, target: &Target) -> Command {
    let mut command = Command::new("ssh");

    if let Some(port) = config.port {
        command.arg("-p");
        command.arg(port.to_string());
    }

    options(&mut command, opts, config);
    command.arg(&target.ssh);
    command
}

fn options(command: &mut Command, opts: &Opts, config: &Deploy) {
    // NB: `ssh` is only made verbose at the second level, since
    // what they print is about the connection rather than the deployment.
    if opts.verbose >= 2 {
        command.arg("-v");
    }

    if let Some(identity_file) = &config.identity_file {
        command.arg("-i");
        command.arg(identity_file);
    }

    for option in &config.options {
        command.arg("-o");
        command.arg(option);
    }
}

fn run(o: &mut StandardStream, opts: &Opts, command: &mut Command) -> Result<()> {
    let repr = command.display().to_string();

    if opts.dry_run {
        writeln!(o, "{repr}")?;
        return Ok(());
    }

    tracing::info!("{repr}");

    let status = command.status()?;

    if !status.success() {
        bail!("Command failed with {status}: {repr}");
    }

    Ok(())
}

/// Run a command with the given files streamed to its stdin as a tar archive.
///
/// A dry run only prints the command, since the files being sent are listed
/// with the rest of the deployment.
fn run_with_payload(
    o: &mut StandardStream,
    opts: &Opts,
    command: &mut Command,
    files: &[(PathBuf, String)],
) -> Result<()> {
    let repr = command.display().to_string();

    if opts.dry_run {
        writeln!(o, "{repr} < <tar archive of the uploads>")?;
        return Ok(());
    }

    tracing::info!("{repr}");

    let mut child = command.stdin(Stdio::piped()).spawn()?;
    let stdin = child.stdin()?;

    // NB: The remote end hanging up while the archive is being written is
    // reported through its exit status, which is the more useful of the two
    // errors since it comes with whatever the remote script printed.
    let written = write_payload(stdin, files).map(drop);
    let status = child.wait_with_output()?.status;

    if !status.success() {
        bail!("Command failed with {status}: {repr}");
    }

    written.with_context(|| anyhow!("Sending files to: {repr}"))
}

/// Write the given files as a tar archive, each named by its staged name.
///
/// Entries are owned by uid and gid 0 and only readable by their owner. The
/// owner only takes effect when the archive is unpacked as root, which is then
/// who they should belong to, and the staged copies are only read by `install`
/// which applies the mode they are actually installed with.
fn write_payload<W>(out: W, files: &[(PathBuf, String)]) -> Result<W>
where
    W: std::io::Write,
{
    let mut builder = tar::Builder::new(out);

    for (path, name) in files {
        let file =
            std::fs::File::open(path).with_context(|| anyhow!("Opening {}", path.display()))?;

        let len = file
            .metadata()
            .with_context(|| anyhow!("Reading metadata of {}", path.display()))?
            .len();

        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(len);
        header.set_mode(0o600);
        header.set_uid(0);
        header.set_gid(0);
        header.set_mtime(0);

        builder
            .append_data(&mut header, name, std::io::Read::take(file, len))
            .with_context(|| anyhow!("Sending {}", path.display()))?;
    }

    Ok(builder.into_inner()?)
}

/// Check that the host being deployed to can actually be accessed before doing
/// any work.
///
/// This logs in over ssh and makes sure that we end up as the expected user,
/// that the commands we depend on are available, and that we can elevate
/// privileges without being prompted for a password. Nothing is modified on the
/// remote host.
///
/// Returns the home directory of the user being logged in as, if it could be
/// determined.
fn check(
    o: &mut StandardStream,
    opts: &Opts,
    config: &Deploy,
    target: &Target,
    use_sudo: bool,
    systemd: bool,
) -> Result<Option<String>> {
    let host = &target.ssh;

    let mut commands = REQUIRED_COMMANDS.to_vec();

    if systemd {
        commands.extend(SYSTEMD_COMMANDS);
    }

    let mut script = String::new();

    writeln!(
        script,
        r#"printf 'user=%s
' "$(id -un 2>/dev/null || true)""#
    )?;
    writeln!(
        script,
        r#"printf 'home=%s
' "$HOME""#
    )?;
    writeln!(script, "for cmd in {}; do", commands.join(" "))?;
    writeln!(
        script,
        r#"if command -v "$cmd" >/dev/null 2>&1; then printf 'command=%s
' "$cmd"; fi"#
    )?;
    writeln!(script, "done")?;

    if use_sudo {
        writeln!(
            script,
            r#"if sudo -n true >/dev/null 2>&1; then printf 'sudo=yes
'; else printf 'sudo=no
'; fi"#
        )?;
    }

    if opts.verbose >= 2 {
        details(o, "access check", script.lines())?;
    }

    let mut command = ssh(opts, config, target);
    command.arg(&script);

    if opts.dry_run {
        writeln!(o, "{}", command.display())?;
    } else {
        tracing::debug!("{}", command.display());
        tracing::info!("Checking access to `{host}`");
    }

    // NB: Only stdout is captured, since ssh reports failures to authenticate
    // and any prompts it needs to make over stderr.
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()?;

    if !output.status.success() {
        bail!(
            "Failed to access `{host}` over ssh with {}, see the error above (pass `--no-check` to skip this check)",
            output.status
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);

    let mut user = None;
    let mut home = None;
    let mut available = HashSet::new();
    let mut sudo = None;

    for line in stdout.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };

        match key {
            "user" if !value.is_empty() => user = Some(value.to_owned()),
            "home" if !value.is_empty() => home = Some(value.to_owned()),
            "command" => {
                available.insert(value.to_owned());
            }
            "sudo" => sudo = Some(value == "yes"),
            _ => {}
        }
    }

    match (target.user.as_deref(), user.as_deref()) {
        (Some(expected), Some(user)) if expected != user => {
            tracing::warn!("Logged into `{host}` as `{user}`, but expected `{expected}`");
        }
        (_, Some(user)) => {
            tracing::info!("Logged into `{host}` as `{user}`");
        }
        (_, None) => {
            tracing::warn!("Logged into `{host}`, but could not determine which user as");
        }
    }

    for command in commands {
        if available.contains(command) {
            continue;
        }

        match command {
            "systemctl" => bail!(
                "Missing `systemctl` on `{host}`, which is needed to install the systemd unit (pass `--no-systemd` to skip it)"
            ),
            "tar" => bail!(
                "Missing `tar` on `{host}`, which is needed to receive the files being deployed"
            ),
            command => bail!(
                "Missing `{command}` on `{host}`, which is needed to install the files being deployed"
            ),
        }
    }

    // NB: The script which installs the deployment is run non-interactively
    // over ssh, so there is nowhere for a sudo password prompt to go.
    if sudo == Some(false) {
        bail!(
            "Cannot use sudo on `{host}` without a password, and the deployment is run \
             non-interactively over ssh so there is nowhere to prompt for one. Give the user a \
             NOPASSWD entry in sudoers, or set `sudo = false` in the `[deploy]` section if you \
             are deploying as root"
        );
    }

    Ok(home)
}

/// Trim any trailing slashes from a directory so that it can be consistently
/// joined with a file name.
fn trim_dir(dir: &str) -> &str {
    let trimmed = dir.trim_end_matches('/');

    if trimmed.is_empty() { dir } else { trimmed }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::config::{
        Build, CommandLine, ConfigCommand, Deploy, DeployKind, Section, Systemd, SystemdScope,
    };
    use crate::packaging::Mode;

    use super::{
        Choice, Common, Opts, ScriptOpts, Sources, Target, choose_profile, expand_home, script,
        ssh_targets, write_payload,
    };

    fn profiles(names: &[&str]) -> Deploy {
        let mut deploy = Deploy::default();

        for name in names {
            deploy
                .profiles
                .insert((*name).to_owned(), Deploy::default());
        }

        deploy
    }

    #[test]
    fn profile_without_profiles_is_the_base() {
        let deploy = Deploy::default();
        assert_eq!(
            choose_profile(&deploy, Section::Deploy, None, false).unwrap(),
            Choice::Base
        );
        assert_eq!(
            choose_profile(&deploy, Section::Deploy, None, true).unwrap(),
            Choice::Base
        );

        let error = choose_profile(&deploy, Section::Deploy, Some("local"), false).unwrap_err();
        assert!(error.to_string().contains("no profiles are defined"));
    }

    #[test]
    fn profile_from_flag() {
        let mut deploy = profiles(&["local", "remote"]);
        deploy.default_profile = Some(String::from("remote"));

        assert_eq!(
            choose_profile(&deploy, Section::Deploy, Some("local"), false).unwrap(),
            Choice::Profile("local")
        );
    }

    #[test]
    fn profile_from_default() {
        let mut deploy = profiles(&["local", "remote"]);
        deploy.default_profile = Some(String::from("remote"));

        assert_eq!(
            choose_profile(&deploy, Section::Deploy, None, false).unwrap(),
            Choice::Profile("remote")
        );

        deploy.default_profile = Some(String::from("missing"));
        let error = choose_profile(&deploy, Section::Deploy, None, false).unwrap_err();
        assert!(error.to_string().contains("`local`, `remote`"));
    }

    #[test]
    fn profile_single() {
        let deploy = profiles(&["local"]);

        assert_eq!(
            choose_profile(&deploy, Section::Deploy, None, false).unwrap(),
            Choice::Profile("local")
        );
    }

    #[test]
    fn profile_ambiguous() {
        let deploy = profiles(&["local", "remote"]);

        let error = choose_profile(&deploy, Section::Deploy, None, false).unwrap_err();
        let error = error.to_string();
        assert!(error.contains("--to <profile>"), "{error}");
        assert!(error.contains("default_profile"), "{error}");
        assert!(error.contains("`local`, `remote`"), "{error}");

        assert_eq!(
            choose_profile(&deploy, Section::Deploy, None, true).unwrap(),
            Choice::Ask(vec!["local", "remote"])
        );
    }

    #[test]
    fn profile_unknown() {
        let deploy = profiles(&["local", "remote"]);

        let error = choose_profile(&deploy, Section::Deploy, Some("nope"), true).unwrap_err();
        let error = error.to_string();
        assert!(error.contains("`nope`"), "{error}");
        assert!(error.contains("`local`, `remote`"), "{error}");
    }

    #[test]
    fn profile_layers_over_base() {
        let mut deploy = Deploy {
            build: Build {
                binary: Some(String::from("kanban")),
                features: vec![String::from("bundle")],
                ..Build::default()
            },
            bin_dir: Some(String::from("/usr/local/bin")),
            default_profile: Some(String::from("local")),
            ..Deploy::default()
        };

        deploy.profiles.insert(
            String::from("local"),
            Deploy {
                kind: Some(DeployKind::Local),
                bin_dir: Some(String::from("~/.cargo/bin")),
                ..Deploy::default()
            },
        );

        let local = deploy.with_profile("local").unwrap();
        assert_eq!(local.kind, Some(DeployKind::Local));
        assert_eq!(local.build.binary.as_deref(), Some("kanban"));
        assert_eq!(local.build.features, ["bundle"]);
        assert_eq!(local.bin_dir.as_deref(), Some("~/.cargo/bin"));
        assert!(local.profiles.is_empty());
        assert!(local.default_profile.is_none());

        assert!(deploy.with_profile("missing").is_none());
    }

    /// A configuration with a `remote` ssh profile which names no host.
    fn hostless_remote() -> Deploy {
        let mut deploy = Deploy::default();

        deploy.profiles.insert(
            String::from("remote"),
            Deploy {
                kind: Some(DeployKind::Ssh),
                user: Some(String::from("integration")),
                ..Deploy::default()
            },
        );

        deploy
    }

    fn hosts(targets: &[Target]) -> Vec<&str> {
        targets.iter().map(|t| t.ssh.as_str()).collect()
    }

    #[test]
    fn host_from_flag_for_hostless_profile() {
        let config = hostless_remote().with_profile("remote").unwrap();

        let opts = Opts {
            host: vec![String::from("moore")],
            ..Opts::default()
        };

        let targets = ssh_targets(&config, Some("remote"), &opts).unwrap();
        assert_eq!(hosts(&targets), ["integration@moore"]);
        assert_eq!(targets[0].host, "moore");
    }

    #[test]
    fn host_missing_for_profile() {
        let config = hostless_remote().with_profile("remote").unwrap();

        let error = ssh_targets(&config, Some("remote"), &Opts::default()).unwrap_err();
        let error = error.to_string();
        assert!(error.contains("profile `remote`"), "{error}");
        assert!(
            error.contains("`kick deploy --to remote --host <host>`"),
            "{error}"
        );
        assert!(error.contains("`[deploy.profiles.remote]`"), "{error}");
    }

    #[test]
    fn host_missing_without_profile() {
        let error = ssh_targets(&Deploy::default(), None, &Opts::default()).unwrap_err();
        let error = error.to_string();
        assert!(error.contains("`kick deploy --host <host>`"), "{error}");
        assert!(error.contains("`[deploy]`"), "{error}");
    }

    #[test]
    fn host_layers_from_base_into_profile() {
        let mut deploy = hostless_remote();
        deploy.host = vec![String::from("dahl")];

        let config = deploy.with_profile("remote").unwrap();
        let targets = ssh_targets(&config, Some("remote"), &Opts::default()).unwrap();
        assert_eq!(hosts(&targets), ["integration@dahl"]);

        // A host named on the command line replaces the layered one.
        let opts = Opts {
            host: vec![String::from("moore"), String::from("root@hilbert")],
            ..Opts::default()
        };

        let targets = ssh_targets(&config, Some("remote"), &opts).unwrap();
        assert_eq!(hosts(&targets), ["integration@moore", "root@hilbert"]);

        // A host set by the profile wins over the one in `[deploy]`.
        deploy.profiles.get_mut("remote").unwrap().host = vec![String::from("moore")];
        let config = deploy.with_profile("remote").unwrap();
        let targets = ssh_targets(&config, Some("remote"), &Opts::default()).unwrap();
        assert_eq!(hosts(&targets), ["integration@moore"]);
    }

    #[test]
    fn expands_home() {
        let home = Some("/home/me");

        assert_eq!(expand_home("~", home), Ok(Some(String::from("/home/me"))));
        assert_eq!(
            expand_home("~/.cargo/bin", home),
            Ok(Some(String::from("/home/me/.cargo/bin")))
        );
        assert_eq!(
            expand_home("$HOME/x", home),
            Ok(Some(String::from("/home/me/x")))
        );
        assert_eq!(
            expand_home("${HOME}/x", Some("/home/me/")),
            Ok(Some(String::from("/home/me/x")))
        );
        assert_eq!(expand_home("~other/x", home), Ok(None));
        assert_eq!(expand_home("$HOMEDIR", home), Ok(None));
        assert_eq!(expand_home("/usr/local/bin", home), Ok(None));
        assert_eq!(expand_home("/usr/local/bin", None), Ok(None));
        assert_eq!(expand_home("~/x", None), Err(()));
    }

    fn user_unit() -> Deploy {
        Deploy {
            systemd: Some(Systemd {
                scope: Some(SystemdScope::User),
                ..Systemd::default()
            }),
            ..Deploy::default()
        }
    }

    #[test]
    fn local_script() {
        let config = user_unit();
        let opts = Opts::default();

        let uploads = vec![
            (
                PathBuf::from("/src/kanban/target/release/kanban"),
                String::from("kanban"),
            ),
            (
                PathBuf::from("/src/kanban/config.toml"),
                String::from("config.toml"),
            ),
            (
                PathBuf::from("/tmp/unit/kanban.service"),
                String::from("kanban.service"),
            ),
        ];

        let installs = vec![(
            String::from("config.toml"),
            String::from("/home/me/.config/kanban/config.toml"),
            "644".parse::<Mode>().unwrap(),
        )];

        let script = script(
            &config,
            &opts,
            &installs,
            ScriptOpts {
                sudo: "",
                scope: SystemdScope::User,
                binary: Some("kanban"),
                bin_dir: "/home/me/.cargo/bin",
                unit_dir: "/home/me/.config/systemd/user",
                sources: Sources::Local(&uploads),
                unit: Some(("kanban", "kanban.service")),
                socket: None,
            },
        )
        .unwrap();

        let expected = "\
set -eu
systemctl --user stop kanban 2>/dev/null || true
mkdir -p /home/me/.cargo/bin
install -m 0755 /src/kanban/target/release/kanban /home/me/.cargo/bin/kanban
install -D -m 0644 /src/kanban/config.toml /home/me/.config/kanban/config.toml
mkdir -p /home/me/.config/systemd/user
if ! cmp -s /tmp/unit/kanban.service /home/me/.config/systemd/user/kanban.service; then
  install -m 0644 /tmp/unit/kanban.service /home/me/.config/systemd/user/kanban.service
  systemctl --user daemon-reload
fi
systemctl --user enable kanban
systemctl --user start kanban
";

        assert_eq!(script, expected);
    }

    #[test]
    fn ssh_script_with_user_unit() {
        let config = user_unit();
        let opts = Opts::default();

        let script = script(
            &config,
            &opts,
            &[],
            ScriptOpts {
                sudo: "sudo -n ",
                scope: SystemdScope::User,
                binary: Some("track"),
                bin_dir: "/usr/local/bin",
                unit_dir: "/home/integration/.config/systemd/user",
                sources: Sources::Staged(".kick-deploy"),
                unit: Some(("track", "track.service")),
                socket: None,
            },
        )
        .unwrap();

        // NB: The binary still needs sudo, but nothing belonging to the user
        // unit does.
        let expected = "\
set -eu
mkdir -p .kick-deploy
tar -x -f - -C .kick-deploy
systemctl --user stop track 2>/dev/null || true
sudo -n mkdir -p /usr/local/bin
sudo -n install -m 0755 .kick-deploy/track /usr/local/bin/track
mkdir -p /home/integration/.config/systemd/user
if ! cmp -s .kick-deploy/track.service /home/integration/.config/systemd/user/track.service; then
  install -m 0644 .kick-deploy/track.service /home/integration/.config/systemd/user/track.service
  systemctl --user daemon-reload
fi
systemctl --user enable track
systemctl --user start track
rm -f .kick-deploy/track
rm -f .kick-deploy/track.service
";

        assert_eq!(script, expected);
    }

    #[test]
    fn ssh_script_with_system_unit() {
        let config = Deploy::default();
        let opts = Opts::default();

        let script = script(
            &config,
            &opts,
            &[],
            ScriptOpts {
                sudo: "sudo -n ",
                scope: SystemdScope::System,
                binary: Some("track"),
                bin_dir: "/usr/local/bin",
                unit_dir: "/etc/systemd/system",
                sources: Sources::Staged(".kick-deploy"),
                unit: Some(("track", "track.service")),
                socket: None,
            },
        )
        .unwrap();

        let expected = "\
set -eu
mkdir -p .kick-deploy
tar -x -f - -C .kick-deploy
sudo -n systemctl stop track 2>/dev/null || true
sudo -n mkdir -p /usr/local/bin
sudo -n install -m 0755 .kick-deploy/track /usr/local/bin/track
sudo -n mkdir -p /etc/systemd/system
if ! sudo -n cmp -s .kick-deploy/track.service /etc/systemd/system/track.service; then
  sudo -n install -m 0644 .kick-deploy/track.service /etc/systemd/system/track.service
  sudo -n systemctl daemon-reload
fi
sudo -n systemctl enable track
sudo -n systemctl start track
rm -f .kick-deploy/track
rm -f .kick-deploy/track.service
";

        assert_eq!(script, expected);
    }

    #[test]
    fn local_script_with_socket() {
        let config = user_unit();
        let opts = Opts::default();

        let uploads = vec![
            (
                PathBuf::from("/src/kanban/target/release/kanban"),
                String::from("kanban"),
            ),
            (
                PathBuf::from("/tmp/unit/kanban.service"),
                String::from("kanban.service"),
            ),
            (
                PathBuf::from("/tmp/unit/kanban.socket"),
                String::from("kanban.socket"),
            ),
        ];

        let script = script(
            &config,
            &opts,
            &[],
            ScriptOpts {
                sudo: "",
                scope: SystemdScope::User,
                binary: Some("kanban"),
                bin_dir: "/home/me/.cargo/bin",
                unit_dir: "/home/me/.config/systemd/user",
                sources: Sources::Local(&uploads),
                unit: Some(("kanban", "kanban.service")),
                socket: Some("kanban.socket"),
            },
        )
        .unwrap();

        let expected = "\
set -eu
socket_changed=no
if ! cmp -s /tmp/unit/kanban.socket /home/me/.config/systemd/user/kanban.socket; then
  socket_changed=yes
fi
if [ \"$socket_changed\" = yes ]; then
  systemctl --user stop kanban 2>/dev/null || true
  systemctl --user stop kanban.socket 2>/dev/null || true
fi
mkdir -p /home/me/.cargo/bin
install -m 0755 /src/kanban/target/release/kanban /home/me/.cargo/bin/kanban
mkdir -p /home/me/.config/systemd/user
reload=no
if ! cmp -s /tmp/unit/kanban.service /home/me/.config/systemd/user/kanban.service; then
  install -m 0644 /tmp/unit/kanban.service /home/me/.config/systemd/user/kanban.service
  reload=yes
fi
if [ \"$socket_changed\" = yes ]; then
  install -m 0644 /tmp/unit/kanban.socket /home/me/.config/systemd/user/kanban.socket
  reload=yes
fi
if [ \"$reload\" = yes ]; then
  systemctl --user daemon-reload
fi
systemctl --user enable --now kanban.socket
systemctl --user restart kanban
";

        assert_eq!(script, expected);
    }

    #[test]
    fn ssh_script_with_system_socket() {
        let config = Deploy::default();
        let opts = Opts::default();

        let script = script(
            &config,
            &opts,
            &[],
            ScriptOpts {
                sudo: "sudo -n ",
                scope: SystemdScope::System,
                binary: Some("kanban"),
                bin_dir: "/usr/local/bin",
                unit_dir: "/etc/systemd/system",
                sources: Sources::Staged(".kick-deploy"),
                unit: Some(("kanban", "kanban.service")),
                socket: Some("kanban.socket"),
            },
        )
        .unwrap();

        let expected = "\
set -eu
mkdir -p .kick-deploy
tar -x -f - -C .kick-deploy
socket_changed=no
if ! sudo -n cmp -s .kick-deploy/kanban.socket /etc/systemd/system/kanban.socket; then
  socket_changed=yes
fi
if [ \"$socket_changed\" = yes ]; then
  sudo -n systemctl stop kanban 2>/dev/null || true
  sudo -n systemctl stop kanban.socket 2>/dev/null || true
fi
sudo -n mkdir -p /usr/local/bin
sudo -n install -m 0755 .kick-deploy/kanban /usr/local/bin/kanban
sudo -n mkdir -p /etc/systemd/system
reload=no
if ! sudo -n cmp -s .kick-deploy/kanban.service /etc/systemd/system/kanban.service; then
  sudo -n install -m 0644 .kick-deploy/kanban.service /etc/systemd/system/kanban.service
  reload=yes
fi
if [ \"$socket_changed\" = yes ]; then
  sudo -n install -m 0644 .kick-deploy/kanban.socket /etc/systemd/system/kanban.socket
  reload=yes
fi
if [ \"$reload\" = yes ]; then
  sudo -n systemctl daemon-reload
fi
sudo -n systemctl enable --now kanban.socket
sudo -n systemctl restart kanban
rm -f .kick-deploy/kanban
rm -f .kick-deploy/kanban.service
rm -f .kick-deploy/kanban.socket
";

        assert_eq!(script, expected);
    }

    /// Without restarting, the units are still installed and the socket is
    /// still enabled, but nothing is stopped or started.
    #[test]
    fn socket_script_without_restart() {
        let config = user_unit();

        let opts = Opts::local(Common {
            no_restart: true,
            ..Common::default()
        });

        let script = script(
            &config,
            &opts,
            &[],
            ScriptOpts {
                sudo: "",
                scope: SystemdScope::User,
                binary: Some("kanban"),
                bin_dir: "/bin",
                unit_dir: "/units",
                sources: Sources::Staged("s"),
                unit: Some(("kanban", "kanban.service")),
                socket: Some("kanban.socket"),
            },
        )
        .unwrap();

        assert!(!script.contains(" stop "), "{script}");
        assert!(!script.contains(" start "), "{script}");
        assert!(!script.contains(" restart "), "{script}");
        assert!(!script.contains("--now"), "{script}");
        assert!(
            script.contains("systemctl --user enable kanban.socket\n"),
            "{script}"
        );
        assert!(!script.contains("enable kanban\n"), "{script}");
    }

    /// When `commands` install the binary, the script only installs what
    /// is left, such as files and hooks.
    #[test]
    fn local_script_without_binary() {
        let config = start_commands();
        let opts = Opts::default();

        let script = script(
            &config,
            &opts,
            &[(
                String::from("kanban.conf"),
                String::from("/etc/kanban.conf"),
                Mode::READ_WRITE,
            )],
            ScriptOpts {
                sudo: "",
                scope: SystemdScope::User,
                binary: None,
                bin_dir: "/home/me/.cargo/bin",
                unit_dir: "/units",
                sources: Sources::Staged("s"),
                unit: None,
                socket: None,
            },
        )
        .unwrap();

        assert!(!script.contains(".cargo/bin"), "{script}");
        assert!(
            script.contains("install -D -m 0644 s/kanban.conf /etc/kanban.conf\n"),
            "{script}"
        );
        assert!(script.contains("\necho started\n"), "{script}");
        assert!(!script.contains("rm -f s/kanban\n"), "{script}");
    }

    fn start_commands() -> Deploy {
        Deploy {
            post_install: vec![
                ConfigCommand {
                    line: CommandLine::Line(String::from("systemd-sysusers")),
                    sudo: true,
                },
                ConfigCommand {
                    line: CommandLine::Line(String::from(
                        "/usr/local/bin/kanban --db ~/kanban.db install",
                    )),
                    sudo: false,
                },
            ],
            post_start: vec![ConfigCommand {
                line: CommandLine::Args(vec![String::from("echo"), String::from("started")]),
                sudo: false,
            }],
            ..Deploy::default()
        }
    }

    /// Commands run after the units are installed, around the start of the
    /// socket and the service.
    #[test]
    fn ssh_script_with_start_commands() {
        let config = start_commands();
        let opts = Opts::default();

        let script = script(
            &config,
            &opts,
            &[],
            ScriptOpts {
                sudo: "sudo -n ",
                scope: SystemdScope::System,
                binary: Some("kanban"),
                bin_dir: "/usr/local/bin",
                unit_dir: "/etc/systemd/system",
                sources: Sources::Staged(".kick-deploy"),
                unit: Some(("kanban", "kanban.service")),
                socket: Some("kanban.socket"),
            },
        )
        .unwrap();

        let expected = "\
if [ \"$reload\" = yes ]; then
  sudo -n systemctl daemon-reload
fi
sudo -n systemd-sysusers
/usr/local/bin/kanban --db ~/kanban.db install
sudo -n systemctl enable --now kanban.socket
sudo -n systemctl restart kanban
echo started
rm -f .kick-deploy/kanban
";

        assert!(script.contains(expected), "{script}");

        let script = super::script(
            &config,
            &opts,
            &[],
            ScriptOpts {
                sudo: "sudo -n ",
                scope: SystemdScope::System,
                binary: Some("kanban"),
                bin_dir: "/usr/local/bin",
                unit_dir: "/etc/systemd/system",
                sources: Sources::Staged(".kick-deploy"),
                unit: Some(("kanban", "kanban.service")),
                socket: None,
            },
        )
        .unwrap();

        let expected = "\
  sudo -n systemctl daemon-reload
fi
sudo -n systemd-sysusers
/usr/local/bin/kanban --db ~/kanban.db install
sudo -n systemctl enable kanban
sudo -n systemctl start kanban
echo started
rm -f .kick-deploy/kanban
";

        assert!(script.contains(expected), "{script}");
    }

    /// Without a unit the commands run once everything is installed, and a
    /// deployment which doesn't restart anything skips them.
    #[test]
    fn local_script_with_start_commands() {
        let config = start_commands();
        let uploads = vec![(PathBuf::from("/src/kanban"), String::from("kanban"))];

        let s = || ScriptOpts {
            sudo: "sudo ",
            scope: SystemdScope::System,
            binary: Some("kanban"),
            bin_dir: "/usr/local/bin",
            unit_dir: "/etc/systemd/system",
            sources: Sources::Local(&uploads),
            unit: None,
            socket: None,
        };

        let script = script(&config, &Opts::default(), &[], s()).unwrap();

        let expected = "\
set -eu
sudo mkdir -p /usr/local/bin
sudo install -m 0755 /src/kanban /usr/local/bin/kanban
sudo systemd-sysusers
/usr/local/bin/kanban --db ~/kanban.db install
echo started
";

        assert_eq!(script, expected);

        let opts = Opts::local(Common {
            no_restart: true,
            ..Common::default()
        });

        let script = super::script(&config, &opts, &[], s()).unwrap();
        assert!(!script.contains("sysusers"), "{script}");
        assert!(!script.contains("echo started"), "{script}");
    }

    #[test]
    fn payload_round_trips() {
        let dir = tempfile::TempDir::new().unwrap();

        let binary = dir.path().join("track");
        std::fs::write(&binary, b"\x7fELF binary").unwrap();

        let unit = dir.path().join("track.service");
        std::fs::write(&unit, b"[Service]\n").unwrap();

        let long = "x".repeat(150);
        let config = dir.path().join("config.toml");
        std::fs::write(&config, b"").unwrap();

        let files = vec![
            (binary, String::from("track")),
            (unit, String::from("track.service")),
            (config, long.clone()),
        ];

        let archive = write_payload(Vec::new(), &files).unwrap();
        assert_eq!(archive.len() % 512, 0);

        let mut archive = tar::Archive::new(&archive[..]);
        let mut entries = Vec::new();

        for entry in archive.entries().unwrap() {
            let mut entry = entry.unwrap();
            let header = entry.header();
            assert_eq!(header.entry_type(), tar::EntryType::Regular);
            assert_eq!(header.mode().unwrap(), 0o600);
            assert_eq!(header.uid().unwrap(), 0);
            assert_eq!(header.gid().unwrap(), 0);

            let path = entry.path().unwrap().to_string_lossy().into_owned();
            let mut contents = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut contents).unwrap();
            entries.push((path, contents));
        }

        assert_eq!(
            entries,
            [
                (String::from("track"), b"\x7fELF binary".to_vec()),
                (String::from("track.service"), b"[Service]\n".to_vec()),
                (long, Vec::new()),
            ]
        );
    }
}
