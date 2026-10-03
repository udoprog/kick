//! The resolved plan of an install or a deployment.
//!
//! Everything here is derived from configuration and options alone, without
//! building anything, reading the project or contacting a host. That is what
//! lets `kick inspect` report what `kick install` and `kick deploy` will do
//! from the same code which decides it.

use crate::config::{Build, Deploy, DeployKind, Section, Systemd, SystemdScope, SystemdSocket};
use crate::systemd::UnitKind;

use super::Common;

/// The remote directory binaries are installed into by default.
const DEFAULT_BIN_DIR: &str = "/usr/local/bin";
/// The directory a local install puts binaries into by default, unless
/// `CARGO_HOME` says otherwise.
const DEFAULT_CARGO_BIN_DIR: &str = "~/.cargo/bin";
/// The directory system units are installed into by default.
const DEFAULT_UNIT_DIR: &str = "/etc/systemd/system";
/// The directory user units are installed into by default.
const DEFAULT_USER_UNIT_DIR: &str = "~/.config/systemd/user";
/// The remote directory files are uploaded to by default, relative to the home
/// directory of the user being logged in as.
const DEFAULT_STAGING_DIR: &str = ".kick-deploy";
/// The build profile binaries are picked up from by default.
const DEFAULT_PROFILE: &str = "release";

/// An install or a deployment, resolved from its configuration and options.
#[derive(Debug)]
pub(crate) struct Plan {
    /// The section the deployment is configured in.
    pub(crate) section: Section,
    /// The names of the profiles defined in the section.
    pub(crate) profiles: Vec<String>,
    /// The selected profile, if any.
    pub(crate) selected: Option<String>,
    /// The section with the selected profile layered over it.
    pub(crate) config: Deploy,
    /// How the machine being deployed to is reached.
    pub(crate) kind: DeployKind,
    /// Which systemd instance units are installed into.
    pub(crate) scope: SystemdScope,
    /// Whether privileged commands are prefixed with `sudo`.
    pub(crate) sudo: bool,
    /// The `[build]` section with the build of the deployment layered over it.
    pub(crate) build: Build,
    /// The systemd configuration, if a unit is installed.
    pub(crate) systemd: Option<Systemd>,
    /// Whether `commands` replace building and installing the binary.
    pub(crate) custom: bool,
    /// The cargo profile the binary is built with.
    pub(crate) cargo_profile: String,
    /// The cargo package being built, if any.
    pub(crate) package: Option<String>,
    /// The binary being installed, if it is configured rather than determined
    /// from the project.
    pub(crate) binary: Option<String>,
    /// The directories involved, before `~` is expanded.
    pub(crate) dirs: Dirs,
}

impl Plan {
    /// Resolve a deployment.
    ///
    /// `base` is the section as configured and `config` is the same section
    /// with the `selected` profile layered over it.
    pub(crate) fn new(
        section: Section,
        base: &Deploy,
        config: Deploy,
        selected: Option<String>,
        base_build: &Build,
        opts: &Common,
    ) -> Self {
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

        let mut build = base_build.clone();
        build.merge_with(config.build.clone());

        // NB: Deploying a service without a unit to run it is rarely what
        // anyone wants, so the built-in template applies unless it is turned
        // off. Most things which are installed are not services, so an install
        // only has a unit when it is configured with one.
        let systemd_default = match section {
            Section::Install => config.systemd.is_some(),
            Section::Deploy => true,
        };

        let systemd = (systemd_config.enabled.unwrap_or(systemd_default) && !opts.no_systemd)
            .then_some(systemd_config);

        // NB: Installing locally happens as the user running kick, which is
        // rarely someone who wants to elevate.
        let sudo = config.sudo.unwrap_or(kind == DeployKind::Ssh);

        let cargo_profile = opts
            .profile
            .as_deref()
            .or(build.profile.as_deref())
            .unwrap_or(DEFAULT_PROFILE)
            .to_owned();

        let package = opts.package.clone().or_else(|| build.package.clone());

        let binary = opts
            .binary
            .clone()
            .or_else(|| build.binary.clone())
            .or_else(|| package.clone());

        let dirs = Dirs {
            bin_dir: trim_dir(
                opts.bin_dir
                    .as_deref()
                    .or(config.bin_dir.as_deref())
                    .unwrap_or(&default_bin_dir(kind)),
            )
            .to_owned(),
            unit_dir: trim_dir(
                config
                    .unit_dir
                    .as_deref()
                    .unwrap_or(default_unit_dir(scope)),
            )
            .to_owned(),
            staging_dir: trim_dir(config.staging_dir.as_deref().unwrap_or(DEFAULT_STAGING_DIR))
                .to_owned(),
        };

        Self {
            section,
            profiles: base.profiles.keys().cloned().collect(),
            selected,
            // NB: Commands replace building and installing the binary.
            custom: !config.commands.is_empty(),
            config,
            kind,
            scope,
            sudo,
            build,
            systemd,
            cargo_profile,
            package,
            binary,
            dirs,
        }
    }

    /// Whether a binary is needed, which it is when it is installed or when a
    /// unit runs it.
    pub(crate) fn needs_binary(&self) -> bool {
        !self.custom || self.systemd.is_some()
    }

    /// The socket unit which activates the service, if one is installed.
    pub(crate) fn socket(&self) -> Option<&SystemdSocket> {
        self.systemd
            .as_ref()
            .and_then(|systemd| systemd.socket.as_ref())
            .filter(|socket| socket.enabled.unwrap_or(true))
    }

    /// The name of the service unit, given the binary it runs.
    pub(crate) fn unit_name(&self, binary: Option<&str>) -> Option<String> {
        let systemd = self.systemd.as_ref()?;
        Some(systemd.name.as_deref().or(binary)?.to_owned())
    }

    /// The name of the socket unit, given the name of the service it
    /// activates.
    ///
    /// A socket unit is only installed alongside the service it activates, and
    /// is named after it unless told otherwise, which is what lets systemd pair
    /// them up without a `Service=` directive.
    pub(crate) fn socket_name(&self, unit_name: Option<&str>) -> Option<String> {
        let socket = self.socket()?;
        Some(socket.name.as_deref().or(unit_name)?.to_owned())
    }

    /// The prefix privileged commands in the install script are run with.
    pub(crate) fn sudo_prefix(&self) -> &'static str {
        match (self.sudo, self.kind) {
            (false, _) => "",
            // NB: The script is run non-interactively over ssh, so `-n` is used
            // to make sudo fail immediately with a diagnostic instead of trying
            // to prompt for a password on a terminal which isn't there.
            (true, DeployKind::Ssh) => "sudo -n ",
            // NB: A local script runs on our terminal, where sudo can prompt
            // like it usually does.
            (true, DeployKind::Local) => "sudo ",
        }
    }

    /// Expand `~` in the configured variables of a unit.
    ///
    /// A user unit runs as the user being deployed as, so a `~` means the same
    /// thing to it as it does to us. A system unit runs as whatever `User=`
    /// says, and systemd resolves a `~` in `WorkingDirectory=` against that
    /// user, so it is left alone.
    ///
    /// Returns the value which could not be expanded since `home` is not
    /// known.
    pub(crate) fn expand_variables(
        &self,
        variables: &mut toml::Table,
        home: Option<&str>,
    ) -> Result<(), String> {
        if self.scope == SystemdScope::User {
            for (_, value) in variables.iter_mut() {
                expand_value(value, home)?;
            }
        }

        Ok(())
    }
}

/// The directories a deployment involves.
#[derive(Debug, Clone)]
pub(crate) struct Dirs {
    /// The directory the binary is installed into.
    pub(crate) bin_dir: String,
    /// The directory units are installed into.
    pub(crate) unit_dir: String,
    /// The remote directory files are uploaded to.
    pub(crate) staging_dir: String,
}

impl Dirs {
    /// Expand a leading `~` in every directory to the given home directory.
    ///
    /// Returns the value which could not be expanded since `home` is not
    /// known.
    pub(crate) fn expand(&self, home: Option<&str>) -> Result<Self, String> {
        Ok(Self {
            bin_dir: expand_or_keep(&self.bin_dir, home)?,
            unit_dir: expand_or_keep(&self.unit_dir, home)?,
            staging_dir: expand_or_keep(&self.staging_dir, home)?,
        })
    }
}

/// The variables kick provides to a unit template.
///
/// Both `kick deploy` and `kick inspect` build their variables through this,
/// so that inspect reports exactly the variables a unit is rendered with. `V`
/// is the value of a variable, which inspect might only be able to describe.
pub(crate) struct Builtins<V> {
    /// The name of the unit.
    pub(crate) name: V,
    /// The name of the binary.
    pub(crate) binary: V,
    /// The path of the installed binary, see [`exec`].
    pub(crate) exec: V,
    pub(crate) bin_dir: V,
    pub(crate) unit_dir: V,
    /// The host being deployed to, without any login user.
    pub(crate) host: V,
    pub(crate) scope: V,
    /// The file name of the unit the unit is paired with: the socket unit
    /// which activates a service, if any, or the service a socket activates.
    pub(crate) peer: Option<V>,
}

impl<V> Builtins<V> {
    /// The variables as `(name, value)`, in the order they are provided.
    pub(crate) fn into_vec(self, kind: UnitKind) -> Vec<(&'static str, V)> {
        let peer = match kind {
            UnitKind::Service => "socket",
            UnitKind::Socket => "service",
        };

        let mut out = vec![
            ("name", self.name),
            ("binary", self.binary),
            ("exec", self.exec),
            ("bin_dir", self.bin_dir),
            ("unit_dir", self.unit_dir),
            ("host", self.host),
            ("scope", self.scope),
        ];

        if let Some(value) = self.peer {
            out.push((peer, value));
        }

        out
    }
}

/// The path the binary is installed as, which is the `exec` variable.
pub(crate) fn exec(bin_dir: &str, binary: &str) -> String {
    format!("{bin_dir}/{binary}")
}

/// The file name of a unit.
pub(crate) fn unit_file(name: &str, kind: UnitKind) -> String {
    match kind {
        UnitKind::Service => format!("{name}.service"),
        UnitKind::Socket => format!("{name}.socket"),
    }
}

/// The directory a binary is installed into unless told otherwise.
pub(crate) fn default_bin_dir(kind: DeployKind) -> String {
    match kind {
        DeployKind::Ssh => DEFAULT_BIN_DIR.to_owned(),
        DeployKind::Local => cargo_bin_dir(),
    }
}

/// The directory a unit is installed into unless told otherwise.
pub(crate) fn default_unit_dir(scope: SystemdScope) -> &'static str {
    match scope {
        SystemdScope::System => DEFAULT_UNIT_DIR,
        SystemdScope::User => DEFAULT_USER_UNIT_DIR,
    }
}

/// The directory cargo installs binaries into, which is where something
/// installed on the machine kick runs on goes by default.
fn cargo_bin_dir() -> String {
    match std::env::var("CARGO_HOME") {
        Ok(home) if !home.is_empty() => format!("{}/bin", home.trim_end_matches('/')),
        _ => String::from(DEFAULT_CARGO_BIN_DIR),
    }
}

/// Trim any trailing slashes from a directory so that it can be consistently
/// joined with a file name.
pub(crate) fn trim_dir(dir: &str) -> &str {
    let trimmed = dir.trim_end_matches('/');

    if trimmed.is_empty() { dir } else { trimmed }
}

/// Expand a leading `~`, `$HOME` or `${HOME}` in a path to the given home
/// directory.
///
/// Returns `Ok(None)` if there is nothing to expand, and `Err(())` if there is
/// but the home directory is not known.
pub(crate) fn expand_home(value: &str, home: Option<&str>) -> Result<Option<String>, ()> {
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

/// Expand the home directory in a path, keeping it as it is if there is
/// nothing to expand.
///
/// Returns the value if it could not be expanded since `home` is not known.
pub(crate) fn expand_or_keep(value: &str, home: Option<&str>) -> Result<String, String> {
    match expand_home(value, home) {
        Ok(Some(expanded)) => Ok(expanded),
        Ok(None) => Ok(value.to_owned()),
        Err(()) => Err(value.to_owned()),
    }
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
pub(crate) fn local_home() -> Option<String> {
    if let Some(home) = std::env::var_os("HOME")
        && !home.is_empty()
    {
        return Some(home.to_string_lossy().into_owned());
    }

    let dirs = directories::BaseDirs::new()?;
    Some(dirs.home_dir().to_string_lossy().into_owned())
}
