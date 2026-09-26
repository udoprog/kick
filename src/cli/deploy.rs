use std::collections::{HashMap, HashSet};
use std::env::consts::EXE_EXTENSION;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use termcolor::{ColorChoice, StandardStream};

use crate::cli::WithRepos;
use crate::config::{ConfigCommand, Deploy};
use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::model::Repo;
use crate::packaging::{self, Mode};
use crate::process::Command;
use crate::shell::Shell;
use crate::systemd;

/// The remote directory binaries are installed into by default.
const DEFAULT_BIN_DIR: &str = "/usr/local/bin";
/// The remote directory systemd units are installed into by default.
const DEFAULT_UNIT_DIR: &str = "/etc/systemd/system";
/// The remote directory files are uploaded to by default, relative to the home
/// directory of the user being logged in as.
const DEFAULT_STAGING_DIR: &str = ".kick-deploy";
/// The build profile binaries are picked up from by default.
const DEFAULT_PROFILE: &str = "release";
/// Remote commands which are always needed.
const REQUIRED_COMMANDS: &[&str] = &["install"];
/// Remote commands which are needed to install a systemd unit.
const SYSTEMD_COMMANDS: &[&str] = &["systemctl", "cmp"];

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// The name of the binary to deploy.
    ///
    /// This overrides the `binary` option in the `[deploy]` section, and
    /// defaults to the name of the primary crate in the project.
    binary: Option<String>,
    /// A host to deploy to, can be used more than once.
    ///
    /// This replaces the `host` option in the `[deploy]` section rather than
    /// adding to it, and each host is deployed to in turn. A login user can be
    /// spelled out as part of the host, in which case it wins over `--user`.
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
    /// The arguments the deployed service is started with.
    ///
    /// Can be used more than once, and each use is split on whitespace. This
    /// defines the `args` variable, which the built-in unit template appends
    /// to `ExecStart`, and overrides `args` in the `[deploy.systemd]` section.
    /// An argument which itself contains whitespace has to be specified
    /// through the variable instead.
    ///
    /// Since service arguments tend to start with `-`, values are taken as
    /// they are given, which means that `--args --user x` passes `--user x` to
    /// the service rather than being read as an option to `kick`.
    #[arg(long = "args", value_name = "ARGS", allow_hyphen_values = true)]
    args: Vec<String>,
    /// The user the deployed service runs as.
    ///
    /// This defines the `user` variable, which the built-in unit template
    /// installs as a `User=` directive, and overrides `user` in the
    /// `[deploy.systemd]` section. Without it the service runs as `root`,
    /// which is what systemd does in the absence of a `User=` directive.
    ///
    /// This is the user the service runs as, not the user the deployment is
    /// performed as, which is `--user`.
    #[arg(long = "service-user", value_name = "USER")]
    service_user: Option<String>,
    /// The group the deployed service runs as.
    ///
    /// This defines the `group` variable, which the built-in unit template
    /// installs as a `Group=` directive. It defaults to `--service-user`,
    /// since a service which runs as a dedicated user conventionally has a
    /// group of the same name, unless `group` is set in the `[deploy.systemd]`
    /// section.
    #[arg(long)]
    group: Option<String>,
    /// A command to run before the project is built, can be used more than
    /// once.
    ///
    /// This is added to whatever the `pre_build` option in the `[deploy]`
    /// section specifies.
    #[arg(long = "pre-build", value_name = "COMMAND")]
    pre_build: Vec<String>,
    /// A feature to enable when building, can be used more than once.
    ///
    /// This is added to whatever the `build_features` option in the `[deploy]`
    /// section specifies, and has no effect if the build command is specified
    /// in full through the `build` option.
    #[arg(long = "build-features", value_name = "FEATURES")]
    build_features: Vec<String>,
    /// The build profile the binary being deployed is found in, overrides the
    /// `profile` option in the `[deploy]` section.
    #[arg(long)]
    profile: Option<String>,
    /// Do not run the commands specified in the `build` option of the
    /// `[deploy]` section.
    #[arg(long)]
    no_build: bool,
    /// Do not install the systemd unit associated with the deployment.
    #[arg(long)]
    no_systemd: bool,
    /// Do not stop or start the service being deployed.
    #[arg(long)]
    no_restart: bool,
    /// Do not check that the remote host can be accessed before deploying.
    #[arg(long)]
    no_check: bool,
    /// Print the commands which would be run instead of running them.
    ///
    /// Note that the access check is still performed, since it does not modify
    /// the remote host.
    #[arg(long)]
    dry_run: bool,
    /// Print verbose information about what is being done.
    ///
    /// One level `-V` prints the deployment plan, the systemd unit and the
    /// script which is run remotely, and traces the remote script as it
    /// executes. Two levels `-VV` additionally prints the access check and
    /// passes `-v` to `ssh` and `scp`.
    #[arg(long, short = 'V', action = clap::ArgAction::Count)]
    verbose: u8,
}

impl Opts {
    /// Whether details about what is being done should be printed.
    ///
    /// A dry run is verbose by definition, since printing what would be done
    /// is the only thing it does.
    fn details(&self) -> bool {
        self.verbose >= 1 || self.dry_run
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
        deploy(&mut o, cx, opts, repo)
    })?;

    Ok(())
}

#[tracing::instrument(skip_all)]
fn deploy(o: &mut StandardStream, cx: &Ctxt<'_>, opts: &Opts, repo: &Repo) -> Result<()> {
    let config = cx.config.deploy(repo);

    // NB: Hosts given on the command line replace the configured ones rather
    // than adding to them, since `--host` is how you deploy somewhere other
    // than where the project usually goes.
    let hosts = if opts.host.is_empty() {
        &config.host[..]
    } else {
        &opts.host[..]
    };

    if hosts.is_empty() {
        bail!(
            "Missing host to deploy to, specify `host` in the `[deploy]` section or pass `--host <host>`"
        );
    }

    let login = opts.user.as_deref().or(config.user.as_deref());

    let targets = hosts
        .iter()
        .map(|host| Target::new(login, host))
        .collect::<Vec<_>>();

    let root = cx.to_path(repo.path());

    // NB: Deploying a service without a unit to run it is rarely what anyone
    // wants, so the built-in template applies unless it is turned off.
    let systemd = config.systemd.clone().unwrap_or_default();
    let systemd = (systemd.enabled.unwrap_or(true) && !opts.no_systemd).then_some(systemd);

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

    let use_sudo = config.sudo.unwrap_or(true);

    // NB: Access is checked before anything is built, since discovering that we
    // cannot log in after a lengthy build is not very helpful. Every host is
    // checked up front for the same reason, so a fleet which cannot be fully
    // deployed to says so before the first host is touched.
    if !opts.no_check {
        for target in &targets {
            check(o, opts, &config, target, use_sudo, systemd.is_some())?;
        }
    }

    let profile = opts
        .profile
        .as_deref()
        .or(config.profile.as_deref())
        .unwrap_or(DEFAULT_PROFILE);

    if !opts.no_build {
        build(o, opts, &config, &root, profile)?;
    }

    let binary = match opts.binary.as_deref().or(config.binary.as_deref()) {
        Some(binary) => binary.to_owned(),
        None => {
            let workspace = repo.workspace(cx)?;
            let package = workspace.primary_package()?.ensure_package()?;
            package.name()?.to_owned()
        }
    };

    let mut source = repo.path().to_owned();
    source.push("target");
    source.push(profile_dir(profile));
    source.push(&binary);
    source.set_extension(EXE_EXTENSION);

    let binary_path = cx.to_path(&source);

    if !binary_path.is_file() {
        bail!("Missing binary to deploy: {}", binary_path.display());
    }

    let bin_dir = trim_dir(config.bin_dir.as_deref().unwrap_or(DEFAULT_BIN_DIR));
    let unit_dir = trim_dir(config.unit_dir.as_deref().unwrap_or(DEFAULT_UNIT_DIR));
    let staging_dir = trim_dir(config.staging_dir.as_deref().unwrap_or(DEFAULT_STAGING_DIR));

    // NB: The script is run non-interactively over ssh, so `-n` is used to make
    // sudo fail immediately with a diagnostic instead of trying to prompt for a
    // password on a terminal which isn't there.
    let sudo = if use_sudo { "sudo -n " } else { "" };

    // Files to upload, as `(local path, remote file name)`. The unit is added
    // per host, since it is rendered for the host it is being installed on.
    let mut uploads = Vec::new();
    // Files to install remotely, as `(remote file name, destination, mode)`.
    let mut installs = Vec::new();

    uploads.push((binary_path, binary.clone()));

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

    let unit_name = systemd
        .as_ref()
        .map(|systemd| systemd.name.as_deref().unwrap_or(&binary).to_owned());

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

    // The unit file is generated locally into a temporary directory so that it
    // can be uploaded under its expected name. Each host gets its own
    // directory, since the unit is rendered for the host it belongs to.
    let temp = tempfile::TempDir::new().context("Creating temporary directory")?;

    for (index, target) in targets.iter().enumerate() {
        let mut uploads = uploads.clone();

        let unit = match (&systemd, &unit_name) {
            (Some(systemd), Some(name)) => {
                let file_name = format!("{name}.service");

                // NB: The unit is rendered with the variables which are scoped
                // to it in the `[deploy.systemd]` section, since a unit
                // directive is not something anything else in the
                // configuration has any use for.
                let mut variables = systemd.variables.clone();
                variables.insert(String::from("name"), toml::Value::String(name.clone()));
                variables.insert(String::from("binary"), toml::Value::String(binary.clone()));
                variables.insert(
                    String::from("exec"),
                    toml::Value::String(format!("{bin_dir}/{binary}")),
                );
                variables.insert(
                    String::from("bin_dir"),
                    toml::Value::String(bin_dir.to_owned()),
                );
                variables.insert(
                    String::from("unit_dir"),
                    toml::Value::String(unit_dir.to_owned()),
                );
                variables.insert(
                    String::from("host"),
                    toml::Value::String(target.host.clone()),
                );

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

                let dir = temp.path().join(index.to_string());

                std::fs::create_dir_all(&dir)
                    .with_context(|| anyhow!("Creating {}", dir.display()))?;

                let path = dir.join(&file_name);

                std::fs::write(&path, &contents)
                    .with_context(|| anyhow!("Writing {}", path.display()))?;

                uploads.push((path, file_name.clone()));
                Some((name.clone(), file_name, contents))
            }
            _ => None,
        };

        let script = script(
            &config,
            opts,
            &uploads,
            &installs,
            ScriptOpts {
                sudo,
                binary: &binary,
                bin_dir,
                unit_dir,
                staging_dir,
                unit: unit
                    .as_ref()
                    .map(|(name, file_name, _)| (&**name, &**file_name)),
            },
        )?;

        if opts.details() {
            let mut plan = Vec::new();

            plan.push(format!("host: {}", target.host));

            if let Some(user) = &target.user {
                plan.push(format!("user: {user}"));
            }

            if let Some(port) = config.port {
                plan.push(format!("port: {port}"));
            }

            plan.push(format!("binary: {binary}"));
            plan.push(format!("profile: {profile}"));
            plan.push(format!("sudo: {}", if use_sudo { "yes" } else { "no" }));
            plan.push(format!("bin_dir: {bin_dir}"));

            if let Some((_, file_name, _)) = &unit {
                plan.push(format!("unit_dir: {unit_dir}"));
                plan.push(format!("unit: {file_name}"));
            }

            plan.push(format!("staging_dir: {staging_dir}"));

            details(o, "deployment", plan.iter().map(String::as_str))?;

            let uploaded = uploads
                .iter()
                .map(|(path, name)| format!("{} -> {staging_dir}/{name}", path.display()))
                .collect::<Vec<_>>();

            details(o, "uploads", uploaded.iter().map(String::as_str))?;

            let mut installed = vec![format!(
                "{staging_dir}/{binary} -> {bin_dir}/{binary} (0755)"
            )];

            for (name, dest, mode) in &installs {
                installed.push(format!(
                    "{staging_dir}/{name} -> {dest} ({:04o})",
                    mode.permissions()
                ));
            }

            if let Some((_, file_name, _)) = &unit {
                installed.push(format!(
                    "{staging_dir}/{file_name} -> {unit_dir}/{file_name} (0644)"
                ));
            }

            details(o, "installs", installed.iter().map(String::as_str))?;

            if let Some((_, file_name, contents)) = &unit {
                details(o, &format!("{unit_dir}/{file_name}"), contents.lines())?;
            }

            details(o, "remote script", script.lines())?;
        }

        let shell = Shell::Bash;

        let mut command = ssh(opts, &config, target);
        command.arg(format!("mkdir -p {}", shell.escape(staging_dir)));
        run(o, opts, &mut command)?;

        let mut command = scp(opts, &config);

        for (path, _) in &uploads {
            command.arg(path);
        }

        command.arg(format!("{}:{staging_dir}/", target.ssh));
        run(o, opts, &mut command)?;

        let mut command = ssh(opts, &config, target);
        command.arg(&script);
        run(o, opts, &mut command)?;
    }

    Ok(())
}

/// A host being deployed to, along with the user we log into it as.
struct Target {
    /// The argument handed to `ssh` and `scp`, which is `<user>@<host>` when
    /// there is a user to log in as.
    ssh: String,
    /// The host on its own, without any login user.
    host: String,
    /// The user we expect to end up as after logging in, if we know it.
    user: Option<String>,
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
            };
        }

        let Some(login) = login else {
            return Self {
                ssh: host.to_owned(),
                host: host.to_owned(),
                user: None,
            };
        };

        Self {
            ssh: format!("{login}@{host}"),
            host: host.to_owned(),
            user: Some(login.to_owned()),
        }
    }
}

struct ScriptOpts<'a> {
    sudo: &'a str,
    binary: &'a str,
    bin_dir: &'a str,
    unit_dir: &'a str,
    staging_dir: &'a str,
    unit: Option<(&'a str, &'a str)>,
}

/// Build the script which installs the uploaded files remotely.
fn script(
    config: &Deploy,
    opts: &Opts,
    uploads: &[(PathBuf, String)],
    installs: &[(String, String, Mode)],
    s: ScriptOpts<'_>,
) -> Result<String> {
    let shell = Shell::Bash;

    let ScriptOpts {
        sudo,
        binary,
        bin_dir,
        unit_dir,
        staging_dir,
        unit,
    } = s;

    let escape = move |value: &str| shell.escape(value).into_owned();
    let staged = move |name: &str| escape(&format!("{staging_dir}/{name}"));

    let mut script = String::new();

    // NB: Tracing the script is the only insight into the remote half of the
    // deployment, since it is run non-interactively over ssh.
    if opts.verbose >= 1 {
        writeln!(script, "set -eux")?;
    } else {
        writeln!(script, "set -eu")?;
    }

    // Stop the service before its binary is replaced, the unit might not exist
    // yet in which case this is a no-op.
    if let Some((name, _)) = unit
        && !opts.no_restart
    {
        writeln!(
            script,
            "{sudo}systemctl stop {} 2>/dev/null || true",
            shell.escape(name)
        )?;
    }

    writeln!(script, "{sudo}mkdir -p {}", shell.escape(bin_dir))?;

    writeln!(
        script,
        "{sudo}install -m 0755 {} {}",
        staged(binary),
        escape(&format!("{bin_dir}/{binary}"))
    )?;

    for (name, dest, mode) in installs {
        writeln!(
            script,
            "{sudo}install -D -m {:04o} {} {}",
            mode.permissions(),
            staged(name),
            shell.escape(dest)
        )?;
    }

    if let Some((name, file_name)) = unit {
        let dest = escape(&format!("{unit_dir}/{file_name}"));

        writeln!(script, "{sudo}mkdir -p {}", shell.escape(unit_dir))?;

        // NB: Installing the unit unconditionally would touch it on every
        // deployment, so only do it when it actually changed. This also keeps
        // us from reloading systemd for no reason.
        writeln!(
            script,
            "if ! {sudo}cmp -s {} {dest}; then",
            staged(file_name)
        )?;

        writeln!(
            script,
            "  {sudo}install -m 0644 {} {dest}",
            staged(file_name)
        )?;

        writeln!(script, "  {sudo}systemctl daemon-reload")?;
        writeln!(script, "fi")?;

        let enable = config
            .systemd
            .as_ref()
            .and_then(|s| s.enable)
            .unwrap_or(true);

        if enable {
            writeln!(script, "{sudo}systemctl enable {}", shell.escape(name))?;
        }

        if !opts.no_restart {
            writeln!(script, "{sudo}systemctl start {}", shell.escape(name))?;
        }
    }

    for (_, name) in uploads {
        writeln!(script, "rm -f {}", staged(name))?;
    }

    Ok(script)
}

/// Build the project locally.
///
/// Anything in `pre_build` is run first, followed by the build command. The
/// build command is generated from the profile and the features being enabled
/// unless it has been specified in full.
fn build(
    o: &mut StandardStream,
    opts: &Opts,
    config: &Deploy,
    root: &Path,
    profile: &str,
) -> Result<()> {
    let extra = opts
        .pre_build
        .iter()
        .filter_map(|c| ConfigCommand::split(c));

    for pre_build in config.pre_build.iter().cloned().chain(extra) {
        run(o, opts, &mut pre_build.to_command(root))?;
    }

    let features = config
        .build_features
        .iter()
        .chain(&opts.build_features)
        .flat_map(|f| f.split([',', ' ']))
        .filter(|f| !f.is_empty())
        .collect::<Vec<_>>()
        .join(",");

    if !config.build.is_empty() {
        if !features.is_empty() {
            tracing::warn!(
                "Ignoring features `{features}` since the build command is specified in full"
            );
        }

        for build in &config.build {
            run(o, opts, &mut build.to_command(root))?;
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

    if !features.is_empty() {
        command.arg("--features");
        command.arg(&features);
    }

    command.current_dir(root);
    run(o, opts, &mut command)
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

/// Construct an `scp` command.
fn scp(opts: &Opts, config: &Deploy) -> Command {
    let mut command = Command::new("scp");

    if let Some(port) = config.port {
        // NB: Unlike ssh, scp spells the port option with a capital `P`.
        command.arg("-P");
        command.arg(port.to_string());
    }

    options(&mut command, opts, config);
    command
}

fn options(command: &mut Command, opts: &Opts, config: &Deploy) {
    // NB: `ssh` and `scp` are only made verbose at the second level, since
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

/// Check that the host being deployed to can actually be accessed before doing
/// any work.
///
/// This logs in over ssh and makes sure that we end up as the expected user,
/// that the commands we depend on are available, and that we can elevate
/// privileges without being prompted for a password. Nothing is modified on the
/// remote host.
fn check(
    o: &mut StandardStream,
    opts: &Opts,
    config: &Deploy,
    target: &Target,
    use_sudo: bool,
    systemd: bool,
) -> Result<()> {
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
    let mut available = HashSet::new();
    let mut sudo = None;

    for line in stdout.lines() {
        let Some((key, value)) = line.trim().split_once('=') else {
            continue;
        };

        match key {
            "user" if !value.is_empty() => user = Some(value.to_owned()),
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

    Ok(())
}

/// Trim any trailing slashes from a directory so that it can be consistently
/// joined with a file name.
fn trim_dir(dir: &str) -> &str {
    let trimmed = dir.trim_end_matches('/');

    if trimmed.is_empty() { dir } else { trimmed }
}
