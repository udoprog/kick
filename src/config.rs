use core::cell::RefCell;
use core::mem;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, btree_map};
use std::fmt;
use std::fs::{self, File};
use std::hash::Hash;
use std::io::{self, BufRead, BufReader, Write};
use std::iter;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::str::{self, FromStr};

use anyhow::{Context, Error, Result, anyhow, bail};
use musli::{Decode, Encode};
use relative_path::{RelativePath, RelativePathBuf};
use semver::Version;
use tempfile::NamedTempFile;
use url::Url;

use crate::KICK_TOML;
use crate::ctxt::Paths;
use crate::glob::Glob;
use crate::keys::Keys;
use crate::model::{Repo, RepoParams, RepoRef, RepoSource};
use crate::packaging::Mode;
use crate::process;
use crate::shell::Shell;
use crate::systemd;
use crate::templates::{Template, Templating};

/// Default job name.
const DEFAULT_CI_NAME: &str = "CI";
/// Default weekly name.
const DEFAULT_WEEKLY_NAME: &str = "Weekly";
/// Default license to use in configuration.
const DEFAULT_LICENSE: &str = "MIT/Apache-2.0";

struct ErrorMarker;

/// Set up variable defaults.
pub(crate) fn defaults() -> toml::Table {
    let mut defaults = toml::Table::new();
    defaults.insert(
        String::from("ci_name"),
        toml::Value::String(String::from(DEFAULT_CI_NAME)),
    );
    defaults.insert(
        String::from("weekly_name"),
        toml::Value::String(String::from(DEFAULT_WEEKLY_NAME)),
    );
    defaults
}

#[derive(Clone, Encode, Decode)]
pub(crate) struct Replaced {
    path: PathBuf,
    content: Vec<u8>,
    replacement: Box<str>,
    ranges: Vec<Range<usize>>,
}

impl Replaced {
    /// Get the path to replace.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Get replacement string.
    pub(crate) fn replacement(&self) -> &str {
        &self.replacement
    }

    fn write_ranges<O>(&self, mut out: O) -> io::Result<()>
    where
        O: Write,
    {
        let mut last = 0;

        for range in &self.ranges {
            out.write_all(&self.content[last..range.start])?;
            out.write_all(self.replacement.as_bytes())?;
            last = range.end;
        }

        out.write_all(&self.content[last..])?;
        Ok(())
    }

    /// Perform the given write.
    pub(crate) fn save(&self) -> Result<()> {
        let Some(parent) = self.path.parent() else {
            bail!("{}: missing parent directory", self.path.display());
        };

        let mut file = NamedTempFile::new_in(parent)?;
        self.write_ranges(&mut file)
            .with_context(|| self.path.display().to_string())?;
        file.persist(&self.path)?;
        Ok(())
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct Upgrade {
    /// Packages to exclude during an upgrade.
    pub(crate) exclude: BTreeSet<String>,
}

impl Upgrade {
    fn merge_with(&mut self, other: Self) {
        self.exclude.extend(other.exclude);
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct RpmPackage {
    /// Requirements to add to an rpm package.
    pub(crate) requires: Vec<RpmRequire>,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct DebPackage {
    /// Dependencies to add to a debian package.
    pub(crate) depends: Vec<DebDependency>,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct Binaries {
    /// Whether to automatically include binaries.
    pub(crate) enabled: Option<bool>,
    /// Excluded binaries.
    pub(crate) exclude: BTreeSet<String>,
}

impl Binaries {
    fn merge_with(&mut self, other: Self) {
        self.enabled = other.enabled.or(self.enabled);
        self.exclude.extend(other.exclude);
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct Package {
    /// Automatically include binaries.
    pub(crate) binaries: Binaries,
    /// Packages to include in an rpm package.
    pub(crate) files: Vec<PackageFile>,
    /// Options specific to rpm packages.
    pub(crate) rpm: RpmPackage,
    /// Options specific to deb packages.
    pub(crate) deb: DebPackage,
}

impl Package {
    fn merge_with(&mut self, other: Self) {
        self.binaries.merge_with(other.binaries);
        self.files.extend(other.files);
        self.rpm.requires.extend(other.rpm.requires);
        self.deb.depends.extend(other.deb.depends);
    }
}

/// The command line of a configured command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CommandLine {
    /// A command line as written, which is split on whitespace when it is run
    /// by `kick` itself and handed to the shell as it is when it is part of an
    /// install script.
    Line(String),
    /// A list of arguments, the first of which is the program. This is how an
    /// argument which contains whitespace is specified.
    Args(Vec<String>),
}

/// A configured command.
///
/// In configuration this is either a string, a list of arguments, or a table
/// of the form `{ command = <string or list>, sudo = <bool> }`. The same forms
/// are accepted everywhere a command is, but where a command runs decides how
/// a string is interpreted:
///
/// * Commands run by `kick` itself, like `pre_build`, are not run through a
///   shell, so a string is split on whitespace.
/// * Commands which are part of the script which installs the project, like
///   `post_install`, are command lines for the shell running that script.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigCommand {
    /// The command line.
    pub(crate) line: CommandLine,
    /// Whether the command is run under the sudo prefix of the install script.
    pub(crate) sudo: bool,
}

impl ConfigCommand {
    /// Construct a command from a command line.
    ///
    /// Returns `None` if the string is empty or only contains whitespace.
    pub(crate) fn split(command: &str) -> Option<Self> {
        if command.trim().is_empty() {
            return None;
        }

        Some(Self {
            line: CommandLine::Line(command.to_owned()),
            sudo: false,
        })
    }

    /// The program and its arguments, as `kick` runs the command itself.
    pub(crate) fn argv(&self) -> Vec<&str> {
        match &self.line {
            CommandLine::Line(line) => line.split_whitespace().collect(),
            CommandLine::Args(args) => args.iter().map(String::as_str).collect(),
        }
    }

    /// Build a process command which runs this command in the given directory.
    pub(crate) fn to_command(&self, dir: &Path) -> process::Command {
        let argv = self.argv();
        let (program, args) = argv.split_first().map_or(("", &[][..]), |(p, a)| (*p, a));
        let mut command = process::Command::new(program);
        command.args(args.iter().copied());
        command.current_dir(dir);
        command
    }

    /// The command as a line for the given shell.
    ///
    /// A line is used as it is, while a list of arguments is escaped.
    pub(crate) fn to_shell(&self, shell: Shell) -> String {
        match &self.line {
            CommandLine::Line(line) => line.clone(),
            CommandLine::Args(args) => args
                .iter()
                .map(|arg| shell.escape(arg).into_owned())
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

/// How the project is built, which is shared by `kick install` and `kick
/// deploy`.
#[derive(Default, Debug, Clone)]
pub(crate) struct Build {
    /// The name of the binary which is built and installed.
    pub(crate) binary: Option<String>,
    /// The cargo package which is built, passed as `--package`.
    pub(crate) package: Option<String>,
    /// The cargo profile the binary is built with.
    pub(crate) profile: Option<String>,
    /// Features to enable in the generated build command.
    pub(crate) features: Vec<String>,
    /// Commands which are run before the project is built.
    pub(crate) pre_build: Vec<ConfigCommand>,
    /// Commands which replace the build command which would otherwise be
    /// generated.
    pub(crate) commands: Vec<ConfigCommand>,
}

impl Build {
    pub(crate) fn merge_with(&mut self, mut other: Self) {
        self.binary = other.binary.or(self.binary.take());
        self.package = other.package.or(self.package.take());
        self.profile = other.profile.or(self.profile.take());
        self.features.append(&mut other.features);
        self.pre_build.append(&mut other.pre_build);
        self.commands.append(&mut other.commands);
    }
}

/// An extra file to install as part of a deployment.
#[derive(Debug, Clone)]
pub(crate) struct DeployFile {
    /// The local source of the file, this can be a wildcard.
    pub(crate) source: RelativePathBuf,
    /// The remote destination of the file. If this ends with a `/` the file
    /// name of the source is appended to it.
    pub(crate) dest: String,
    /// The mode the file is installed with.
    pub(crate) mode: Option<Mode>,
}

/// How a deployment reaches the machine it is installed on.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DeployKind {
    /// Upload with `scp` and install over `ssh`, the default.
    #[default]
    Ssh,
    /// Install on the machine `kick` is running on through the local shell.
    Local,
}

impl fmt::Display for DeployKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeployKind::Ssh => f.write_str("ssh"),
            DeployKind::Local => f.write_str("local"),
        }
    }
}

impl FromStr for DeployKind {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ssh" => Ok(DeployKind::Ssh),
            "local" => Ok(DeployKind::Local),
            other => bail!("unsupported deploy kind `{other}`, expected `ssh` or `local`"),
        }
    }
}

/// Which systemd instance a unit is installed into.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SystemdScope {
    /// The system instance, managed with `systemctl`.
    #[default]
    System,
    /// The user instance of the user being deployed as, managed with
    /// `systemctl --user`.
    User,
}

impl SystemdScope {
    /// The name of the scope as used in configuration.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            SystemdScope::System => "system",
            SystemdScope::User => "user",
        }
    }
}

impl fmt::Display for SystemdScope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for SystemdScope {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "system" => Ok(SystemdScope::System),
            "user" => Ok(SystemdScope::User),
            other => bail!("unsupported systemd scope `{other}`, expected `system` or `user`"),
        }
    }
}

/// Options which used to be part of the `[install]` and `[deploy]` sections,
/// and where they are configured now.
const MOVED_TARGET_KEYS: &[(&str, &str)] = &[
    ("binary", "`binary` in the `[build]` section"),
    ("profile", "`profile` in the `[build]` section"),
    ("pre_build", "`pre_build` in the `[build]` section"),
    ("build_features", "`features` in the `[build]` section"),
    ("pre_start", "`post_install`"),
];

/// Which section a target is configured in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Section {
    /// The `[install]` section, which installs on the machine `kick` runs on.
    Install,
    /// The `[deploy]` section.
    Deploy,
}

impl Section {
    /// The name of the section, as used in configuration.
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Section::Install => "install",
            Section::Deploy => "deploy",
        }
    }
}

/// Where and how a built project is installed, which is what the `[install]`
/// and `[deploy]` sections both configure.
///
/// The `[install]` section takes everything except for the options which only
/// make sense over ssh, and `commands` is only available there.
#[derive(Default, Debug, Clone)]
pub(crate) struct Deploy {
    /// How the deployment reaches the machine being deployed to.
    pub(crate) kind: Option<DeployKind>,
    /// The profile deployed when none is selected with `--to`.
    pub(crate) default_profile: Option<String>,
    /// Named profiles, each of which is layered over the rest of this section.
    pub(crate) profiles: BTreeMap<String, Deploy>,
    /// The hosts to deploy to, such as `example.com`.
    pub(crate) host: Vec<String>,
    /// The user to log into the hosts as.
    pub(crate) user: Option<String>,
    /// The port to connect to.
    pub(crate) port: Option<u16>,
    /// The identity file to authenticate with.
    pub(crate) identity_file: Option<String>,
    /// Extra options passed to `ssh` through `-o`.
    pub(crate) options: Vec<String>,
    /// Whether privileged commands should be prefixed with `sudo`.
    pub(crate) sudo: Option<bool>,
    /// The directory binaries are installed into.
    pub(crate) bin_dir: Option<String>,
    /// The directory systemd units are installed into.
    pub(crate) unit_dir: Option<String>,
    /// The remote directory files are uploaded to before being installed.
    pub(crate) staging_dir: Option<String>,
    /// How the project is built, layered over the `[build]` section.
    pub(crate) build: Build,
    /// Commands which replace building and installing the binary, only
    /// available in the `[install]` section.
    pub(crate) commands: Vec<ConfigCommand>,
    /// Extra files to install.
    pub(crate) files: Vec<DeployFile>,
    /// Commands which are run on the target after everything has been
    /// installed and before the service is started.
    pub(crate) post_install: Vec<ConfigCommand>,
    /// Commands which are run on the target after the service has been
    /// started.
    pub(crate) post_start: Vec<ConfigCommand>,
    /// The systemd unit to install.
    pub(crate) systemd: Option<Systemd>,
}

impl Deploy {
    /// The deployment described by the named profile, which is the profile
    /// layered over the settings it shares with every other profile.
    ///
    /// Returns `None` if there is no such profile.
    pub(crate) fn with_profile(&self, name: &str) -> Option<Deploy> {
        let profile = self.profiles.get(name)?;

        let mut deploy = Deploy {
            default_profile: None,
            profiles: BTreeMap::new(),
            ..self.clone()
        };

        deploy.merge_with(profile.clone());
        Some(deploy)
    }

    fn merge_with(&mut self, mut other: Self) {
        self.kind = other.kind.or(self.kind.take());
        self.default_profile = other.default_profile.or(self.default_profile.take());

        for (name, profile) in other.profiles {
            match self.profiles.entry(name) {
                btree_map::Entry::Vacant(e) => {
                    e.insert(profile);
                }
                btree_map::Entry::Occupied(e) => {
                    e.into_mut().merge_with(profile);
                }
            }
        }

        // NB: Hosts replace rather than extend, since a more specific layer
        // which names its own hosts is redirecting the deployment rather than
        // adding to it.
        if !other.host.is_empty() {
            self.host = other.host;
        }

        self.user = other.user.or(self.user.take());
        self.port = other.port.or(self.port.take());
        self.identity_file = other.identity_file.or(self.identity_file.take());
        self.options.append(&mut other.options);
        self.sudo = other.sudo.or(self.sudo.take());
        self.bin_dir = other.bin_dir.or(self.bin_dir.take());
        self.unit_dir = other.unit_dir.or(self.unit_dir.take());
        self.staging_dir = other.staging_dir.or(self.staging_dir.take());
        self.build.merge_with(other.build);
        self.commands.append(&mut other.commands);
        self.files.append(&mut other.files);
        self.post_install.append(&mut other.post_install);
        self.post_start.append(&mut other.post_start);

        match (&mut self.systemd, other.systemd) {
            (Some(systemd), Some(other)) => systemd.merge_with(other),
            (systemd @ None, other) => *systemd = other,
            _ => {}
        }
    }
}

/// The systemd unit associated with a deployment.
#[derive(Debug, Clone)]
pub(crate) struct Systemd {
    /// Whether a unit should be installed at all, which is how `systemd =
    /// false` is represented. Defaults to `true`.
    pub(crate) enabled: Option<bool>,
    /// The source of the template which is rendered into the unit being
    /// installed, or `None` for the built-in template.
    pub(crate) template: Option<Box<str>>,
    /// The name of the unit, defaults to the name of the binary.
    pub(crate) name: Option<String>,
    /// Whether the unit should be enabled so that it starts on boot.
    pub(crate) enable: Option<bool>,
    /// Which systemd instance the unit is installed into.
    pub(crate) scope: Option<SystemdScope>,
    /// The socket unit which activates the service, if any.
    pub(crate) socket: Option<SystemdSocket>,
    /// The variables the unit template is rendered with.
    ///
    /// These are every key in the section which isn't one of the options
    /// above, which scopes them to the unit rather than putting them in the
    /// global `[variables]` section where nothing else has any use for them.
    pub(crate) variables: toml::Table,
}

impl Default for Systemd {
    #[inline]
    fn default() -> Self {
        Self {
            enabled: None,
            template: None,
            name: None,
            enable: None,
            scope: None,
            socket: None,
            variables: toml::Table::new(),
        }
    }
}

impl Systemd {
    fn merge_with(&mut self, mut other: Self) {
        // NB: Only what the more specific layer actually said overrides what
        // came before it, since a section which just sets a variable has no
        // opinion on which template is being rendered or whether a unit is
        // being installed at all.
        self.enabled = other.enabled.or(self.enabled.take());
        self.template = other.template.take().or(self.template.take());
        self.name = other.name.or(self.name.take());
        self.enable = other.enable.or(self.enable.take());
        self.scope = other.scope.or(self.scope.take());

        match (&mut self.socket, other.socket) {
            (Some(socket), Some(other)) => socket.merge_with(other),
            (socket @ None, other) => *socket = other,
            _ => {}
        }

        merge_map(&mut self.variables, other.variables);
    }
}

/// The socket unit which activates the service of a deployment.
#[derive(Default, Debug, Clone)]
pub(crate) struct SystemdSocket {
    /// Whether a socket unit should be installed at all, which is how `socket
    /// = false` is represented. Defaults to `true` once the section is present.
    pub(crate) enabled: Option<bool>,
    /// The source of the template which is rendered into the socket unit, or
    /// `None` for the built-in template.
    pub(crate) template: Option<Box<str>>,
    /// The name of the socket unit, defaults to the name of the service.
    pub(crate) name: Option<String>,
    /// The variables the socket template is rendered with.
    pub(crate) variables: toml::Table,
}

impl SystemdSocket {
    fn merge_with(&mut self, mut other: Self) {
        self.enabled = other.enabled.or(self.enabled.take());
        self.template = other.template.take().or(self.template.take());
        self.name = other.name.or(self.name.take());
        merge_map(&mut self.variables, other.variables);
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct DenyAction {
    /// The name of a denied action.
    pub(crate) name: String,
    /// The reason an action is denied.
    pub(crate) reason: Option<String>,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct LatestAction {
    /// The name of an action.
    pub(crate) name: String,
    /// The latest version available of the given action.
    pub(crate) version: String,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct Actions {
    /// Packages to include in an rpm package.
    pub(crate) deny: Vec<DenyAction>,
    /// Latest versions of available actions.
    pub(crate) latest: Vec<LatestAction>,
}

impl Actions {
    fn merge_with(&mut self, other: Self) {
        self.deny.extend(other.deny);
        self.latest.extend(other.latest);
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct PackageFile {
    /// The source of an rpm file.
    pub(crate) source: RelativePathBuf,
    /// Destination of an rpm file.
    pub(crate) dest: RelativePathBuf,
    /// The mode of a file.
    pub(crate) mode: Option<Mode>,
}

#[derive(Clone, Debug, Copy)]
pub(crate) enum VersionConstraint {
    /// Greater than.
    Gt,
    /// Greater than or equal to.
    Ge,
    /// Less than.
    Lt,
    /// Less than or equal to.
    Le,
    /// Equal to.
    Eq,
}

impl fmt::Display for VersionConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionConstraint::Gt => write!(f, ">"),
            VersionConstraint::Ge => write!(f, ">="),
            VersionConstraint::Lt => write!(f, "<"),
            VersionConstraint::Le => write!(f, "<="),
            VersionConstraint::Eq => write!(f, "="),
        }
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) enum VersionRequirement {
    #[default]
    Any,
    Constraint(VersionConstraint, Version),
}

impl FromStr for VersionRequirement {
    type Err = Error;

    #[inline]
    fn from_str(string: &str) -> Result<Self, Self::Err> {
        if string == "*" {
            return Ok(VersionRequirement::Any);
        }

        let Some((op, version)) = string.split_once(' ') else {
            return Err(anyhow!("illegal version specification: {string}"));
        };

        let op = match op {
            "<" => VersionConstraint::Lt,
            "<=" => VersionConstraint::Le,
            "=" => VersionConstraint::Eq,
            ">=" => VersionConstraint::Ge,
            ">" => VersionConstraint::Gt,
            version => return Err(anyhow!("illegal version constraint: {version}")),
        };

        Ok(VersionRequirement::Constraint(op, Version::parse(version)?))
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct RpmRequire {
    /// The package being required.
    pub(crate) package: String,
    /// The version being required.
    pub(crate) version: VersionRequirement,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct DebDependency {
    /// The package being required.
    pub(crate) package: String,
    /// The version being required.
    pub(crate) version: VersionRequirement,
}

#[derive(Debug, Clone)]
pub(crate) struct Replacement {
    /// Replacements to perform in a given package.
    pub(crate) package_name: Option<String>,
    /// Replacement path.
    pub(crate) paths: Vec<RelativePathBuf>,
    /// A regular expression pattern to replace.
    pub(crate) pattern: regex::bytes::Regex,
}

impl Replacement {
    /// Find and perform replacements in the given root path.
    pub(crate) fn replace_in(
        &self,
        root: &Path,
        group: &str,
        replacement: &str,
    ) -> Result<Vec<Replaced>> {
        let mut output = Vec::new();

        for path in &self.paths {
            let glob = Glob::new(root, path);

            for path in glob.matcher() {
                let path = path?;
                let output_path = path.to_path(root);

                let content = match fs::read(&output_path) {
                    Ok(content) => content,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        tracing::warn!("{path}: failed to read");
                        continue;
                    }
                    Err(e) => return Err(Error::from(e)).context(path),
                };

                let mut ranges = Vec::new();

                for cap in self.pattern.captures_iter(&content) {
                    if let Some(m) = cap.name(group)
                        && m.as_bytes() != replacement.as_bytes()
                    {
                        ranges.push(m.range());
                    }
                }

                if !ranges.is_empty() {
                    output.push(Replaced {
                        path: output_path,
                        content,
                        replacement: replacement.into(),
                        ranges,
                    });
                }
            }
        }

        Ok(output)
    }
}

/// Which operating system we are on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Os {
    Windows,
    Linux,
    Mac,
    Other(String),
}

impl Os {
    /// Get the shell for the given operating system.
    pub(crate) fn shell(&self) -> Shell {
        match self {
            Os::Windows => Shell::Powershell,
            _ => Shell::Bash,
        }
    }

    /// Get the corresponding tree value.
    pub(crate) fn as_tree_value(&self) -> &str {
        match self {
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::Mac => "macOS",
            Self::Other(other) => other,
        }
    }
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Os::Windows => write!(f, "Windows"),
            Os::Linux => write!(f, "Linux"),
            Os::Mac => write!(f, "Mac"),
            Os::Other(other) => other.fmt(f),
        }
    }
}

/// Which distribution we are on.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Distribution {
    #[default]
    Other,
    Ubuntu,
    Debian,
    Fedora,
}

impl Distribution {
    /// Get the distribution from a string.
    pub(crate) fn from_string_ignore_case(string: impl AsRef<str>) -> Self {
        let string = string.as_ref();

        if string.eq_ignore_ascii_case("ubuntu") {
            return Distribution::Ubuntu;
        }

        if string.eq_ignore_ascii_case("debian") {
            return Distribution::Debian;
        }

        if string.eq_ignore_ascii_case("fedora") {
            return Distribution::Fedora;
        }

        Distribution::Other
    }

    /// Get the linux distribution we are currently on.
    pub(crate) fn linux_distribution() -> Option<Self> {
        let Ok(f) = File::open("/etc/os-release") else {
            tracing::trace!("No such file: /etc/os-release");
            return None;
        };

        let mut f = BufReader::new(f);
        let mut line = String::new();

        loop {
            line.clear();

            if f.read_line(&mut line).ok()? == 0 {
                break;
            }

            let Some((key, value)) = line.trim().split_once('=') else {
                continue;
            };

            if key.trim().eq_ignore_ascii_case("id") {
                return Some(Distribution::from_string_ignore_case(value.trim()));
            }
        }

        None
    }
}

impl fmt::Display for Distribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Distribution::Other => write!(f, "Other"),
            Distribution::Ubuntu => write!(f, "Ubuntu"),
            Distribution::Debian => write!(f, "Debian"),
            Distribution::Fedora => write!(f, "Fedora"),
        }
    }
}

#[derive(Default, Debug)]
pub(crate) struct RepoConfig {
    /// Sources for this repo.
    pub(crate) sources: BTreeSet<RepoSource>,
    /// Override crate to use.
    pub(crate) name: Option<String>,
    /// URLs to use.
    pub(crate) urls: BTreeSet<Url>,
    /// Supported operating system.
    pub(crate) os: BTreeSet<Os>,
    /// Name of the repo branch.
    pub(crate) branch: Option<String>,
    /// Filesystem workflows that have been found.
    pub(crate) filesystem_workflows: HashSet<String>,
    /// Workflows to incorporate.
    pub(crate) workflows: HashMap<String, PartialWorkflowConfig>,
    /// License of the project.
    pub(crate) license: Option<String>,
    /// Authors of the project.
    pub(crate) authors: Vec<String>,
    /// Documentation link of the project.
    pub(crate) documentation: Option<Template>,
    /// Custom lib template.
    pub(crate) lib: Option<Template>,
    /// Custom readme template.
    pub(crate) readme: Option<Template>,
    /// Custom badges for a specific project.
    pub(crate) badges: Vec<ConfigBadge>,
    /// Path to Cargo.toml to build.
    pub(crate) cargo_toml: Option<RelativePathBuf>,
    /// Disabled modules.
    pub(crate) disabled: BTreeSet<String>,
    /// Badges used in lib file.
    pub(crate) lib_badges: IdSet,
    /// Badges used in readmes.
    pub(crate) readme_badges: IdSet,
    /// Variables that can be used verbatim in templates.
    pub(crate) variables: toml::Table,
    /// Files to look for in replacements.
    pub(crate) version: Vec<Replacement>,
    /// Upgrade configuration.
    pub(crate) upgrade: Upgrade,
    /// RPM configuration.
    pub(crate) package: Package,
    /// Actions configuration.
    pub(crate) actions: Actions,
    /// How the project is built.
    pub(crate) build: Build,
    /// Deployment configuration.
    pub(crate) deploy: Deploy,
    /// Install configuration.
    pub(crate) install: Deploy,
}

impl RepoConfig {
    /// Merge this config with another.
    pub(crate) fn merge_with(&mut self, mut other: Self) {
        self.name = other.name.or(self.name.take());
        self.os.extend(other.os);
        self.branch = other.branch.or(self.branch.take());

        for id in other.filesystem_workflows {
            self.filesystem_workflows.insert(id);
        }

        for (id, workflow) in other.workflows {
            self.workflows.entry(id).or_default().merge_with(workflow);
        }

        self.license = other.license.or(self.license.take());
        self.authors.append(&mut other.authors);
        self.documentation = other.documentation.or(self.documentation.take());
        self.lib = other.lib.or(self.lib.take());
        self.badges.append(&mut other.badges);
        self.cargo_toml = other.cargo_toml.or(self.cargo_toml.take());
        self.disabled.extend(other.disabled);
        self.lib_badges.merge_with(other.lib_badges);
        self.readme_badges.merge_with(other.readme_badges);
        self.version.extend(other.version);
        self.upgrade.merge_with(other.upgrade);
        self.package.merge_with(other.package);
        self.actions.merge_with(other.actions);
        self.build.merge_with(other.build);
        self.deploy.merge_with(other.deploy);
        self.install.merge_with(other.install);

        merge_map(&mut self.variables, other.variables);
    }

    /// Test if this repo wants the specified readme badge.
    pub(crate) fn wants_lib_badge(&self, b: &ConfigBadge, enabled: bool) -> bool {
        let Some(id) = &b.id else {
            return enabled;
        };

        self.lib_badges.is_enabled(id, enabled)
    }

    /// Test if this repo wants the specified lib badge.
    pub(crate) fn wants_readme_badge(&self, b: &ConfigBadge, enabled: bool) -> bool {
        let Some(id) = &b.id else {
            return enabled;
        };

        self.readme_badges.is_enabled(id, enabled)
    }
}

/// A workflow configuration.
#[derive(Default, Debug, Clone)]
pub struct PartialWorkflowConfig {
    /// Workflow template.
    pub(crate) template: Option<Template>,
    /// The expected name of the workflow.
    pub(crate) name: Option<String>,
    /// Eanbled workflow features.
    pub(crate) features: HashSet<WorkflowFeature>,
    /// Branch that the workflow should trigger on.
    pub(crate) branch: Option<String>,
    /// If the workflow config is disabled.
    pub(crate) disable: Option<bool>,
}

impl PartialWorkflowConfig {
    fn merge_with(&mut self, other: Self) {
        self.template = other.template.or(self.template.take());
        self.name = other.name.or(self.name.take());
        self.features.extend(other.features);
        self.branch = other.branch.or(self.branch.take());
        self.disable = other.disable.or(self.disable.take());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkflowFeature {
    ScheduleRandomWeekly,
}

impl FromStr for WorkflowFeature {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "schedule-random-weekly" => Ok(WorkflowFeature::ScheduleRandomWeekly),
            other => bail!("Unknown workflow feature: {other}"),
        }
    }
}

/// A workflow configuration.
#[derive(Default, Debug, Clone)]
pub struct WorkflowConfig {
    /// The expected name of the workflow.
    pub(crate) name: Option<String>,
    /// Features enabled in workflow configuration.
    pub(crate) features: HashSet<WorkflowFeature>,
    /// Branch that the workflow should trigger on.
    pub(crate) branch: Option<String>,
    /// If the workflow configuration is disabled.
    pub(crate) disable: bool,
}

/// A badge configuration.
pub(crate) enum Id {
    /// An enabled badge.
    Enabled(String),
    /// A disabled badge.
    Disabled(String),
}

impl FromStr for Id {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars = s.chars();

        match (chars.next(), chars.as_str()) {
            (Some('-'), rest) if !rest.is_empty() => Ok(Id::Disabled(rest.to_owned())),
            (Some('+'), rest) if !rest.is_empty() => Ok(Id::Enabled(rest.to_owned())),
            (_, "") => Err(anyhow!("expected id after `+` and `-`")),
            _ => Err(anyhow!("expected `+` and `-` in id, but got `{s}`")),
        }
    }
}

/// Set of identifers.
#[derive(Debug, Default)]
pub(crate) struct IdSet {
    /// Explicit allowlist for badges to enabled which are already disabled.
    enabled: HashSet<String>,
    /// Explicit blocklist for badges to enabled.
    disabled: HashSet<String>,
}

impl IdSet {
    /// Merge with another set.
    pub(crate) fn merge_with(&mut self, other: Self) {
        self.enabled.extend(other.enabled);
        self.disabled.extend(other.disabled);
    }

    /// Test if an id is enabled.
    pub(crate) fn is_enabled(&self, id: &str, enabled: bool) -> bool {
        if enabled {
            !self.disabled.contains(id)
        } else {
            self.enabled.contains(id)
        }
    }
}

impl FromIterator<Id> for IdSet {
    #[inline]
    fn from_iter<T: IntoIterator<Item = Id>>(iter: T) -> Self {
        let mut enabled = HashSet::new();
        let mut disabled = HashSet::new();

        for badge in iter {
            match badge {
                Id::Enabled(badge) => {
                    enabled.insert(badge);
                }
                Id::Disabled(badge) => {
                    disabled.insert(badge);
                }
            }
        }

        Self { enabled, disabled }
    }
}

pub(crate) struct Config<'a> {
    pub(crate) base: RepoConfig,
    pub(crate) repos: BTreeMap<RelativePathBuf, RepoConfig>,
    pub(crate) defaults: &'a toml::Table,
}

impl Config<'_> {
    /// Get the branch for the given repo.
    pub(crate) fn branch(&self, repo: &Repo) -> Option<&str> {
        let branch = self.get_all(repo, |c| c.branch.as_ref()).last()?;
        Some(branch)
    }

    /// A workflow configuration.
    pub(crate) fn workflows(&self, repo: &RepoRef) -> Result<BTreeMap<String, WorkflowConfig>> {
        let ids = self.get_all(repo, |c| {
            c.filesystem_workflows.iter().chain(c.workflows.keys())
        });

        let mut out = BTreeMap::new();

        for id in ids {
            if out.contains_key(id) {
                continue;
            }

            let mut config = PartialWorkflowConfig::default();

            for repo in self.repos(repo) {
                if let Some(branch) = &repo.branch {
                    config.branch = Some(branch.clone());
                }

                if let Some(c) = repo.workflows.get(id) {
                    config.merge_with(c.clone());
                }
            }

            out.insert(
                id.to_owned(),
                WorkflowConfig {
                    name: config.name,
                    features: config.features,
                    branch: config.branch,
                    disable: config.disable.unwrap_or_default(),
                },
            );
        }

        Ok(out)
    }

    /// Generate a default workflow.
    pub(crate) fn workflow(
        &self,
        repo: &RepoRef,
        id: &str,
        params: RepoParams<'_>,
    ) -> Result<Option<String>> {
        for repo in self.repos(repo).rev() {
            if let Some(template) = repo.workflows.get(id).and_then(|r| r.template.as_ref()) {
                return Ok(Some(template.render(&params)?));
            }
        }

        Ok(None)
    }

    /// Get the current documentation template.
    pub(crate) fn documentation(&self, repo: &Repo) -> Option<&Template> {
        self.repos(repo)
            .rev()
            .flat_map(|r| r.documentation.as_ref())
            .next()
    }

    /// Get the current license template.
    pub(crate) fn license(&self, repo: &Repo) -> &str {
        self.repos(repo)
            .rev()
            .flat_map(|r| r.license.as_deref())
            .next()
            .unwrap_or(DEFAULT_LICENSE)
    }

    /// Get supported operating systems.
    pub(crate) fn os(&self, repo: &Repo) -> BTreeSet<&Os> {
        self.repos(repo).flat_map(|r| &r.os).collect()
    }

    /// Get the current license template.
    pub(crate) fn authors(&self, repo: &Repo) -> Vec<String> {
        self.repos(repo).flat_map(|r| &r.authors).cloned().collect()
    }

    /// Get the current license template.
    pub(crate) fn variables(&self, repo: &RepoRef) -> toml::Table {
        let mut variables = self.defaults.clone();

        for repo in self.repos(repo) {
            let mut current = repo.variables.clone();

            if let Some(branch) = &repo.branch {
                current.insert(
                    String::from("branch"),
                    toml::Value::String(branch.to_owned()),
                );
            }

            merge_map(&mut variables, current);
        }

        variables
    }

    /// Check if we should automatically include binaries.
    pub(crate) fn package_binaries(&self, repo: &RepoRef) -> Binaries {
        let mut binaries = Binaries::default();

        for repo in self.repos(repo) {
            binaries.merge_with(repo.package.binaries.clone());
        }

        binaries
    }

    /// Get all rpm files.
    pub(crate) fn package_files(&self, repo: &RepoRef) -> Vec<&PackageFile> {
        self.repos(repo).flat_map(|r| &r.package.files).collect()
    }

    /// Get the deployment configuration for the given repo.
    pub(crate) fn deploy(&self, repo: &RepoRef) -> Deploy {
        let mut deploy = Deploy::default();

        for repo in self.repos(repo) {
            deploy.merge_with(repo.deploy.clone());
        }

        deploy
    }

    /// Get the install configuration for the given repo.
    pub(crate) fn install(&self, repo: &RepoRef) -> Deploy {
        let mut install = Deploy::default();

        for repo in self.repos(repo) {
            install.merge_with(repo.install.clone());
        }

        install
    }

    /// Get the build configuration for the given repo, which is shared by
    /// installs and deployments.
    pub(crate) fn build(&self, repo: &RepoRef) -> Build {
        let mut build = Build::default();

        for repo in self.repos(repo) {
            build.merge_with(repo.build.clone());
        }

        build
    }

    /// Get all denied actions.
    pub(crate) fn action_deny(&self, repo: &RepoRef) -> Vec<&DenyAction> {
        self.repos(repo).flat_map(|r| &r.actions.deny).collect()
    }

    /// Get all latest actions.
    pub(crate) fn action_latest(&self, repo: &RepoRef) -> Vec<&LatestAction> {
        self.repos(repo).flat_map(|r| &r.actions.latest).collect()
    }

    /// Get all elements corresponding to the given field.
    pub(crate) fn get_all<'a, O: 'a, I>(
        &'a self,
        repo: &RepoRef,
        mut get: impl FnMut(&'a RepoConfig) -> I,
    ) -> impl Iterator<Item = &'a O>
    where
        I: IntoIterator<Item = &'a O>,
    {
        let a = get(&self.base).into_iter();
        a.chain(self.repos.get(repo.path()).into_iter().flat_map(get))
    }

    fn badges<F>(&self, path: &RelativePath, mut filter: F) -> impl Iterator<Item = &'_ ConfigBadge>
    where
        F: FnMut(&RepoConfig, &ConfigBadge, bool) -> bool,
    {
        let repo = self.repos.get(path);
        let repos = repo.into_iter().flat_map(|repo| repo.badges.iter());

        self.base.badges.iter().chain(repos).filter(move |b| {
            let enabled = filter(&self.base, b, b.enabled);
            repo.map(|repo| filter(repo, b, enabled)).unwrap_or(enabled)
        })
    }

    /// Iterator over lib badges for the given repo.
    pub(crate) fn lib_badges(&self, path: &RelativePath) -> impl Iterator<Item = &'_ ConfigBadge> {
        self.badges(path, |repo, b, enabled| repo.wants_lib_badge(b, enabled))
    }

    /// Iterator over readme badges for the given repo.
    pub(crate) fn readme_badges(
        &self,
        path: &RelativePath,
    ) -> impl Iterator<Item = &'_ ConfigBadge> {
        self.badges(path, |repo, b, enabled| repo.wants_readme_badge(b, enabled))
    }

    /// Get the header for the given repo.
    pub(crate) fn lib(&self, repo: &RepoRef) -> Option<&Template> {
        self.repos(repo).rev().flat_map(|r| r.lib.as_ref()).next()
    }

    /// Get readme template for the given repo.
    pub(crate) fn readme(&self, repo: &RepoRef) -> Option<&Template> {
        self.repos(repo)
            .rev()
            .flat_map(|r| r.readme.as_ref())
            .next()
    }

    /// Get crate for the given repo.
    pub(crate) fn name<'a>(&'a self, repo: &RepoRef) -> Option<&'a str> {
        self.repos(repo)
            .rev()
            .flat_map(|r| r.name.as_deref())
            .next()
    }

    /// Get Cargo.toml path for the given repo.
    pub(crate) fn cargo_toml<'a>(&'a self, repo: &RepoRef) -> Option<&'a RelativePath> {
        self.repos(repo)
            .rev()
            .flat_map(|r| r.cargo_toml.as_deref())
            .next()
    }

    /// Get Cargo.toml path for the given repo.
    pub(crate) fn is_enabled(&self, repo: &RepoRef, feature: &str) -> bool {
        let base = !self.base.disabled.contains(feature);

        let Some(repo) = self.repos.get(repo.path()) else {
            return base;
        };

        base && !repo.disabled.contains(feature)
    }

    /// Get version replacements.
    pub(crate) fn version<'a>(&'a self, repo: &RepoRef) -> Vec<&'a Replacement> {
        self.repos(repo).flat_map(|r| &r.version).collect()
    }

    /// Get crate for the given repo.
    pub(crate) fn upgrade(&self, repo: &RepoRef) -> Upgrade {
        let mut upgrade = Upgrade::default();

        for u in self.repos(repo).rev().map(|r| &r.upgrade) {
            upgrade.merge_with(u.clone());
        }

        upgrade
    }

    fn repos<'a>(&'a self, repo: &RepoRef) -> impl DoubleEndedIterator<Item = &'a RepoConfig> {
        [&self.base].into_iter().chain(self.repos.get(repo.path()))
    }
}

#[derive(Debug)]
pub(crate) struct ConfigBadge {
    pub(crate) id: Option<String>,
    enabled: bool,
    markdown: Option<Template>,
    html: Option<Template>,
}

impl ConfigBadge {
    pub(crate) fn markdown(&self, params: &RepoParams<'_>) -> Result<Option<String>> {
        let Some(template) = self.markdown.as_ref() else {
            return Ok(None);
        };

        Ok(Some(template.render(params)?))
    }

    pub(crate) fn html(&self, params: &RepoParams<'_>) -> Result<Option<String>> {
        let Some(template) = self.html.as_ref() else {
            return Ok(None);
        };

        Ok(Some(template.render(params)?))
    }
}

/// Context used when parsing configuration.
struct Cx<'a> {
    paths: Paths<'a>,
    current: RelativePathBuf,
    config_path: RelativePathBuf,
    keys: Keys,
    templating: &'a Templating,
    errors: RefCell<Vec<Error>>,
}

impl<'a> Cx<'a> {
    fn new(paths: Paths<'a>, current: &RelativePath, templating: &'a Templating) -> Self {
        Self {
            paths,
            current: current.to_owned(),
            config_path: current.join_normalized(KICK_TOML),
            keys: Keys::default(),
            templating,
            errors: RefCell::new(Vec::new()),
        }
    }

    /// Load the kick config.
    fn config(&self) -> Result<Option<toml::Value>, ErrorMarker> {
        let Some(string) = self
            .paths
            .read_to_string(&self.config_path)
            .map_err(self.map())?
        else {
            return Ok(None);
        };

        toml::from_str(&string).map_err(self.map())
    }

    fn map<E>(&self) -> impl FnOnce(E) -> ErrorMarker
    where
        E: fmt::Display,
    {
        move |error| self.capture(error)
    }

    fn capture(&self, error: impl fmt::Display) -> ErrorMarker {
        self.errors.borrow_mut().push(anyhow!(
            "{path}: {}: {error}",
            self.keys,
            path = self.paths.to_path(&self.config_path).display()
        ));

        ErrorMarker
    }

    /// Visit the given key, extracting it from the specified table.
    fn require_in_key<O>(
        &self,
        table: &mut toml::Table,
        key: &str,
        f: impl FnOnce(&Self, toml::Value) -> Result<O, ErrorMarker>,
    ) -> Result<O, ErrorMarker> {
        let Some(value) = table.remove(key) else {
            return Err(self.capture(format_args!("missing required key `{key}`")));
        };

        self.keys.field(key);
        let out = f(self, value);
        self.keys.pop();
        out
    }

    /// Visit the given key, extracting it from the specified table.
    fn in_key<O>(
        &self,
        table: &mut toml::Table,
        key: &str,
        f: impl FnOnce(&Self, toml::Value) -> Result<O, ErrorMarker>,
    ) -> Result<Option<O>, ErrorMarker> {
        let Some(value) = table.remove(key) else {
            return Ok(None);
        };

        self.keys.field(key);
        let out = f(self, value);
        self.keys.pop();
        Ok(Some(out?))
    }

    fn parse<O>(&self, value: toml::Value) -> Result<O, ErrorMarker>
    where
        O: FromStr<Err: fmt::Display>,
    {
        let value = self.string(value)?;
        O::from_str(value.as_str()).map_err(self.map())
    }

    /// Read the template stored at the given path.
    fn read_template(&self, value: toml::Value) -> Result<String, ErrorMarker> {
        let path = self.relative_path(value)?;
        let path = self.paths.to_path(self.current.join(path));

        match fs::read_to_string(&path) {
            Ok(template) => Ok(template),
            Err(err) => Err(self.capture(format_args!("reading {}: {}", path.display(), err))),
        }
    }

    /// Compile a template from a path.
    fn compile_path(&self, value: toml::Value) -> Result<Template, ErrorMarker> {
        let template = self.read_template(value)?;
        self.compile_str(template)
    }

    /// Compile a template.
    fn compile(&self, value: toml::Value) -> Result<Template, ErrorMarker> {
        let source = self.string(value)?;
        self.compile_str(source)
    }

    /// Compile a template from a string.
    fn compile_str(&self, source: impl AsRef<str>) -> Result<Template, ErrorMarker> {
        match self.templating.compile(source.as_ref()) {
            Ok(template) => Ok(template),
            Err(err) => Err(self.capture(err)),
        }
    }

    fn string(&self, value: toml::Value) -> Result<String, ErrorMarker> {
        match value {
            toml::Value::String(string) => Ok(string),
            other => Err(self.capture(format_args!("expected string, got {}", other.type_str()))),
        }
    }

    fn relative_path(&self, value: toml::Value) -> Result<RelativePathBuf, ErrorMarker> {
        let path = RelativePathBuf::from(self.string(value)?);

        if path.as_str().starts_with('/') {
            return Err(self.capture(format_args!("path must be relative, but got {path}")));
        }

        Ok(path)
    }

    fn boolean(&self, value: toml::Value) -> Result<bool, ErrorMarker> {
        match value {
            toml::Value::Boolean(value) => Ok(value),
            other => Err(self.capture(format_args!("expected boolean, got {}", other.type_str()))),
        }
    }

    fn port(&self, value: toml::Value) -> Result<u16, ErrorMarker> {
        match value {
            toml::Value::Integer(value) => match u16::try_from(value) {
                Ok(value) => Ok(value),
                Err(..) => {
                    Err(self.capture(format_args!("expected port between 0-65535, got {value}")))
                }
            },
            other => Err(self.capture(format_args!("expected integer, got {}", other.type_str()))),
        }
    }

    /// Process the elements of a standalone array.
    fn array<O>(
        &self,
        value: toml::Value,
        mut f: impl FnMut(&Self, toml::Value) -> Result<O, ErrorMarker>,
    ) -> Result<Vec<O>, ErrorMarker> {
        let toml::Value::Array(array) = value else {
            return Err(self.capture(format_args!("expected array, got {}", value.type_str())));
        };

        let mut out = Vec::with_capacity(array.len());
        let mut error = false;

        for (index, item) in array.into_iter().enumerate() {
            self.keys.index(index);

            match f(self, item) {
                Ok(item) => out.push(item),
                Err(ErrorMarker) => error = true,
            }

            self.keys.pop();
        }

        if error {
            return Err(ErrorMarker);
        }

        Ok(out)
    }

    fn table(&self, value: toml::Value) -> Result<toml::Table, ErrorMarker> {
        match value {
            toml::Value::Table(table) => Ok(table),
            other => Err(self.capture(format_args!("expected table, got {}", other.type_str()))),
        }
    }

    fn in_array<O, B>(
        &self,
        table: &mut toml::Table,
        key: &str,
        map: Option<(&str, &str)>,
        mut f: impl FnMut(&Self, toml::Value) -> Result<O, ErrorMarker>,
    ) -> Result<B, ErrorMarker>
    where
        B: FromIterator<O>,
    {
        let result = self.in_key(table, key, move |cx, value| match (value, map) {
            (toml::Value::Array(array), _) => {
                let mut error = false;

                let it = array.into_iter().enumerate().flat_map(|(index, item)| {
                    cx.keys.index(index);

                    let out = match f(cx, item) {
                        Ok(out) => Some(out),
                        Err(ErrorMarker) => {
                            error = true;
                            None
                        }
                    };

                    cx.keys.pop();
                    out
                });

                let out = B::from_iter(it);

                if error {
                    return Err(ErrorMarker);
                }

                Ok(out)
            }
            (toml::Value::Table(table), Some((key, value))) => {
                let mut error = false;

                let it = table.into_iter().enumerate().flat_map(|(index, (k, v))| {
                    let mut table = toml::Table::with_capacity(2);
                    table.insert(key.to_owned(), toml::Value::String(k));
                    table.insert(value.to_owned(), v);

                    cx.keys.index(index);

                    let out = match f(cx, toml::Value::Table(table)) {
                        Ok(out) => Some(out),
                        Err(ErrorMarker) => {
                            error = true;
                            None
                        }
                    };

                    cx.keys.pop();
                    out
                });

                let out = B::from_iter(it);

                if error {
                    return Err(ErrorMarker);
                }

                Ok(out)
            }
            (other, Some((key, value))) => Err(self.capture(format_args!(
                "expected array or map {{{key} => {value}}}, got {}",
                other.type_str()
            ))),
            (other, None) => {
                Err(self.capture(format_args!("expected array, got {}", other.type_str())))
            }
        })?;

        match result {
            Some(result) => Ok(result),
            None => Ok(B::from_iter(iter::empty())),
        }
    }

    fn in_table<K, V, O>(
        &self,
        table: &mut toml::Table,
        key: &str,
        mut f: impl FnMut(&Self, String, toml::Value) -> Result<(K, V), ErrorMarker>,
    ) -> Result<O, ErrorMarker>
    where
        K: Eq + Hash,
        O: FromIterator<(K, V)>,
    {
        let out = self.in_key(table, key, move |cx, value| {
            let table = cx.table(value)?;
            let mut error = false;

            let data = O::from_iter(table.into_iter().flat_map(|(key, item)| {
                cx.keys.field(&key);

                let out = match f(cx, key, item) {
                    Ok(out) => Some(out),
                    Err(ErrorMarker) => {
                        error = true;
                        None
                    }
                };

                cx.keys.pop();
                out
            }));

            if error {
                return Err(ErrorMarker);
            }

            Ok(data)
        })?;

        match out {
            Some(out) => Ok(out),
            _ => Ok(O::from_iter(iter::empty())),
        }
    }

    fn as_table<F, O>(
        &self,
        table: &mut toml::Table,
        key: &str,
        f: F,
    ) -> Result<Option<O>, ErrorMarker>
    where
        F: FnOnce(&Self, toml::Table) -> Result<O, ErrorMarker>,
    {
        self.in_key(table, key, move |cx, value| {
            let table = cx.table(value)?;
            f(cx, table)
        })
    }

    fn require_key<O>(
        &self,
        table: &mut toml::Table,
        key: &str,
        f: impl FnOnce(&Self, toml::Value) -> Result<O, ErrorMarker>,
    ) -> Result<O, ErrorMarker> {
        let Some(value) = table.remove(key) else {
            return Err(self.capture(format_args!("missing required key `{key}`")));
        };

        self.keys.field(key);
        let out = f(self, value);
        self.keys.pop();
        out
    }

    fn workflow_table(
        &self,
        table: &mut toml::Table,
    ) -> Result<PartialWorkflowConfig, ErrorMarker> {
        let name = self.in_key(table, "name", Self::string);
        let template = self.in_key(table, "template", Self::compile_path);
        let features = self.in_array(table, "features", None, Self::parse);
        let branch = self.in_key(table, "branch", Self::string);
        let disable = self.in_key(table, "disable", Self::boolean);

        Ok(PartialWorkflowConfig {
            name: name?,
            template: template?,
            features: features?,
            branch: branch?,
            disable: disable?,
        })
    }

    fn repo_table(&self, table: &mut toml::Table) -> Result<RepoConfig, ErrorMarker> {
        let name = self.in_key(table, "name", Self::string);
        let url = self.in_key(table, "url", Self::parse);

        let os: std::result::Result<_, ErrorMarker> =
            self.in_array(table, "os", None, |cx, item| {
                match cx.string(item)?.as_str() {
                    "windows" => Ok(Os::Windows),
                    "linux" => Ok(Os::Linux),
                    "macos" => Ok(Os::Mac),
                    other => Err(cx.capture(format_args!("unknown os: {other}"))),
                }
            });

        let branch = self.in_key(table, "branch", Self::string);
        let workflows = self.in_table(table, "workflows", |cx, id, value| {
            Ok((id, cx.workflow(value)?))
        });
        let license = self.in_key(table, "license", Self::string);
        let authors = self.in_array(table, "authors", None, Self::string);
        let documentation = self.in_key(table, "documentation", Self::compile);
        let lib = self.in_key(table, "lib", Self::compile_path);
        let readme = self.in_key(table, "readme", Self::compile_path);

        let badges = self.in_array(table, "badges", None, |cx, value| {
            cx.with_table(value, |cx, table| {
                let id = cx.in_key(table, "id", Self::string);
                let alt = cx.in_key(table, "alt", Self::string);
                let src = cx.in_key(table, "src", Self::string);
                let href = cx.in_key(table, "href", Self::string);
                let height = cx.in_key(table, "height", Self::string);
                let enabled = cx.in_key(table, "enabled", Self::boolean);

                let (markdown, html) =
                    if let (Ok(alt), Ok(Some(src)), Ok(Some(href)), Ok(Some(height))) =
                        (alt, src, href, height)
                    {
                        let alt =
                            FormatOptional(alt.as_ref(), |f, alt| write!(f, " alt=\"{alt}\""));

                        let markdown = cx.compile_str(format!(
                            "[<img{alt} src=\"{src}\" height=\"{height}\">]({href})"
                        ));

                        let html = cx.compile_str(format!(
                            "<a href=\"{href}\"><img{alt} src=\"{src}\" height=\"{height}\"></a>"
                        ));

                        (Some(markdown), Some(html))
                    } else {
                        (None, None)
                    };

                Ok(ConfigBadge {
                    id: id?,
                    enabled: enabled?.unwrap_or(true),
                    markdown: markdown.transpose()?,
                    html: html.transpose()?,
                })
            })
        });

        let lib_badges = self.in_array(table, "lib_badges", None, Self::parse);
        let readme_badges = self.in_array(table, "readme_badges", None, Self::parse);

        let variables = self.as_table(table, "variables", |_, table| Ok(table));

        let version = self.in_array(table, "version", None, |cx, value| {
            cx.with_table(value, |cx, table| {
                let package_name = cx.in_key(table, "crate", Self::string);
                let paths = cx.in_array(table, "paths", None, Self::relative_path);
                let pattern = cx.require_in_key(table, "pattern", Self::parse);

                Ok(Replacement {
                    package_name: package_name?,
                    paths: paths?,
                    pattern: pattern?,
                })
            })
        });

        let cargo_toml = self.in_key(table, "cargo_toml", Self::relative_path);

        let upgrade = self.in_key(table, "upgrade", Self::upgrade);

        let disabled = self.in_array(table, "disabled", None, Self::string);

        let package = self.in_key(table, "package", Self::package);

        let actions = self.in_key(table, "actions", Self::actions);

        let build = self.in_key(table, "build", Self::build);

        let deploy = self.in_key(table, "deploy", |cx, value| {
            cx.target(value, Section::Deploy)
        });

        let install = self.in_key(table, "install", |cx, value| {
            cx.target(value, Section::Install)
        });

        Ok(RepoConfig {
            sources: BTreeSet::from_iter([RepoSource::Config(self.current.to_owned())]),
            name: name?,
            urls: BTreeSet::from_iter(url?),
            os: os?,
            branch: branch?,
            filesystem_workflows: HashSet::new(),
            workflows: workflows?,
            license: license?,
            authors: authors?,
            documentation: documentation?,
            lib: lib?,
            readme: readme?,
            badges: badges?,
            cargo_toml: cargo_toml?,
            disabled: disabled?,
            lib_badges: lib_badges?,
            readme_badges: readme_badges?,
            variables: variables?.unwrap_or_default(),
            version: version?,
            upgrade: upgrade?.unwrap_or_default(),
            package: package?.unwrap_or_default(),
            actions: actions?.unwrap_or_default(),
            build: build?.unwrap_or_default(),
            deploy: deploy?.unwrap_or_default(),
            install: install?.unwrap_or_default(),
        })
    }

    fn with_table<F, O>(&self, value: toml::Value, f: F) -> Result<O, ErrorMarker>
    where
        F: FnOnce(&Self, &mut toml::Table) -> Result<O, ErrorMarker>,
    {
        let mut config = self.table(value)?;
        let out = f(self, &mut config);

        if !config.is_empty() {
            let keys = config.into_iter().map(|(key, _)| key).collect::<Vec<_>>();

            let what = match &keys[..] {
                [_] => "key",
                _ => "keys",
            };

            let keys = keys.join(", ");
            return Err(self.capture(format_args!("got unsupported {what}: {keys}")));
        }

        out
    }

    fn repo(&self, value: toml::Value) -> Result<RepoConfig, ErrorMarker> {
        self.with_table(value, Self::repo_table)
    }

    fn workflow(&self, value: toml::Value) -> Result<PartialWorkflowConfig, ErrorMarker> {
        self.with_table(value, Self::workflow_table)
    }

    fn upgrade(&self, value: toml::Value) -> Result<Upgrade, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let exclude = cx.in_array(table, "exclude", None, Self::string);
            Ok(Upgrade { exclude: exclude? })
        })
    }

    fn binaries(&self, value: toml::Value) -> Result<Binaries, ErrorMarker> {
        match value {
            toml::Value::Boolean(enabled) => Ok(Binaries {
                enabled: Some(enabled),
                exclude: BTreeSet::new(),
            }),
            value => self.with_table(value, |cx, table| {
                let enabled = cx.in_key(table, "enabled", Self::boolean);
                let exclude = cx.in_array(table, "exclude", None, Self::string);

                let binaries = Binaries {
                    enabled: Some(enabled?.unwrap_or(true)),
                    exclude: exclude?,
                };

                Ok(binaries)
            }),
        }
    }

    fn package_file(&self, value: toml::Value) -> Result<PackageFile, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let source = cx.require_key(table, "source", Self::relative_path);
            let dest = cx.require_key(table, "dest", Self::relative_path);
            let mode = cx.in_key(table, "mode", Self::parse);

            Ok(PackageFile {
                source: source?,
                dest: dest?,
                mode: mode?,
            })
        })
    }

    fn rpm_require(&self, value: toml::Value) -> Result<RpmRequire, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let package = cx.require_key(table, "package", Self::string);
            let version = cx.in_key(table, "version", Self::parse);

            Ok(RpmRequire {
                package: package?,
                version: version?.unwrap_or_default(),
            })
        })
    }

    fn deb_dependency(&self, value: toml::Value) -> Result<DebDependency, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let package = cx.require_key(table, "package", Self::string);
            let version = cx.in_key(table, "version", Self::parse);

            Ok(DebDependency {
                package: package?,
                version: version?.unwrap_or_default(),
            })
        })
    }

    fn rpm(&self, value: toml::Value) -> Result<RpmPackage, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let requires = cx.in_array(
                table,
                "requires",
                Some(("package", "version")),
                Self::rpm_require,
            );

            Ok(RpmPackage {
                requires: requires?,
            })
        })
    }

    fn deb(&self, value: toml::Value) -> Result<DebPackage, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let depends = cx.in_array(
                table,
                "depends",
                Some(("package", "version")),
                Self::deb_dependency,
            );

            Ok(DebPackage { depends: depends? })
        })
    }

    fn package(&self, value: toml::Value) -> Result<Package, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let binaries = cx.in_key(table, "binaries", Self::binaries);
            let files = cx.in_array(table, "files", None, Self::package_file);
            let rpm = cx.in_key(table, "rpm", Self::rpm);
            let deb = cx.in_key(table, "deb", Self::deb);

            let package = Package {
                binaries: binaries?.unwrap_or_default(),
                files: files?,
                rpm: rpm?.unwrap_or_default(),
                deb: deb?.unwrap_or_default(),
            };

            Ok(package)
        })
    }

    /// The hosts being deployed to, which is either a single host or a list of
    /// them.
    fn deploy_hosts(&self, value: toml::Value) -> Result<Vec<String>, ErrorMarker> {
        match value {
            toml::Value::Array(..) => self.array(value, Self::string),
            value => Ok(vec![self.string(value)?]),
        }
    }

    /// A command which `kick` runs itself, which cannot use sudo.
    fn local_command(&self, value: toml::Value) -> Result<ConfigCommand, ErrorMarker> {
        let command = self.config_command(value)?;

        if command.sudo {
            return Err(self.capture(
                "`sudo` is only supported for commands which are part of the install script",
            ));
        }

        Ok(command)
    }

    /// A command, either a command line, a list of arguments, or a table of
    /// either one and whether it is run under sudo.
    fn config_command(&self, value: toml::Value) -> Result<ConfigCommand, ErrorMarker> {
        let (line, sudo) = match value {
            value @ toml::Value::Table(..) => self.with_table(value, |cx, table| {
                let line = cx.require_key(table, "command", Self::command_line);
                let sudo = cx.in_key(table, "sudo", Self::boolean);
                Ok((line?, sudo?.unwrap_or_default()))
            })?,
            value => (self.command_line(value)?, false),
        };

        Ok(ConfigCommand { line, sudo })
    }

    fn command_line(&self, value: toml::Value) -> Result<CommandLine, ErrorMarker> {
        let line = match value {
            value @ toml::Value::String(..) => CommandLine::Line(self.string(value)?),
            value => CommandLine::Args(self.array(value, Self::string)?),
        };

        let empty = match &line {
            CommandLine::Line(line) => line.trim().is_empty(),
            CommandLine::Args(args) => args.is_empty(),
        };

        if empty {
            return Err(self.capture("expected a non-empty command"));
        }

        Ok(line)
    }

    fn build(&self, value: toml::Value) -> Result<Build, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let binary = cx.in_key(table, "binary", Self::string);
            let package = cx.in_key(table, "package", Self::string);
            let profile = cx.in_key(table, "profile", Self::string);
            let features = cx.in_array(table, "features", None, Self::string);
            let pre_build = cx.in_array(table, "pre_build", None, Self::local_command);
            let commands = cx.in_array(table, "commands", None, Self::local_command);

            Ok(Build {
                binary: binary?,
                package: package?,
                profile: profile?,
                features: features?,
                pre_build: pre_build?,
                commands: commands?,
            })
        })
    }

    fn deploy_file(&self, value: toml::Value) -> Result<DeployFile, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let source = cx.require_key(table, "source", Self::relative_path);
            let dest = cx.require_key(table, "dest", Self::string);
            let mode = cx.in_key(table, "mode", Self::parse);

            Ok(DeployFile {
                source: source?,
                dest: dest?,
                mode: mode?,
            })
        })
    }

    /// Read and validate a unit template stored at the given path.
    fn unit_template(&self, value: toml::Value) -> Result<Box<str>, ErrorMarker> {
        let source = self.read_template(value)?;

        if let Err(error) = systemd::validate(&source) {
            return Err(self.capture(error));
        }

        Ok(source.into())
    }

    fn systemd(&self, value: toml::Value) -> Result<Systemd, ErrorMarker> {
        match value {
            // NB: The boolean form asks for the built-in unit template.
            toml::Value::Boolean(enabled) => Ok(Systemd {
                enabled: Some(enabled),
                ..Systemd::default()
            }),
            // NB: The bare string form is the path to a unit template.
            value @ toml::Value::String(..) => Ok(Systemd {
                template: Some(self.unit_template(value)?),
                ..Systemd::default()
            }),
            value => {
                let mut table = self.table(value)?;

                let template = self.in_key(&mut table, "template", Self::unit_template);
                let name = self.in_key(&mut table, "name", Self::string);
                let enable = self.in_key(&mut table, "enable", Self::boolean);
                let scope = self.in_key(&mut table, "scope", Self::parse);
                let socket = self.in_key(&mut table, "socket", Self::systemd_socket);

                // NB: Everything which is left over is a variable the unit
                // template is rendered with. This is why the section cannot
                // reject keys it doesn't know about the way the others do, so
                // a misspelled option quietly becomes a variable which the
                // template doesn't use instead of being reported.
                Ok(Systemd {
                    template: template?,
                    name: name?,
                    enable: enable?,
                    scope: scope?,
                    socket: socket?,
                    variables: table,
                    ..Systemd::default()
                })
            }
        }
    }

    fn systemd_socket(&self, value: toml::Value) -> Result<SystemdSocket, ErrorMarker> {
        match value {
            // NB: The boolean form asks for the built-in socket template.
            toml::Value::Boolean(enabled) => Ok(SystemdSocket {
                enabled: Some(enabled),
                ..SystemdSocket::default()
            }),
            // NB: The bare string form is the path to a socket template.
            value @ toml::Value::String(..) => Ok(SystemdSocket {
                template: Some(self.unit_template(value)?),
                ..SystemdSocket::default()
            }),
            value => {
                let mut table = self.table(value)?;

                let template = self.in_key(&mut table, "template", Self::unit_template);
                let name = self.in_key(&mut table, "name", Self::string);

                // NB: As with the service, everything else is a variable.
                Ok(SystemdSocket {
                    template: template?,
                    name: name?,
                    variables: table,
                    ..SystemdSocket::default()
                })
            }
        }
    }

    /// The `[install]` or `[deploy]` section.
    fn target(&self, value: toml::Value, section: Section) -> Result<Deploy, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let default_profile = cx.in_key(table, "default_profile", Self::string);
            let profiles = cx.in_table(table, "profiles", |cx, name, value| {
                let profile = cx.with_table(value, |cx, table| cx.target_table(table, section))?;
                Ok((name, profile))
            });

            let mut deploy = cx.target_table(table, section)?;
            deploy.default_profile = default_profile?;
            deploy.profiles = profiles?;
            Ok(deploy)
        })
    }

    /// The options shared by a target section and its profiles.
    ///
    /// The options which only apply over ssh are only read for `[deploy]`, so
    /// that they are reported as unsupported in `[install]`.
    fn target_table(
        &self,
        table: &mut toml::Table,
        section: Section,
    ) -> Result<Deploy, ErrorMarker> {
        let cx = self;

        // NB: Options which have moved are reported with where they went,
        // since a configuration written for an older kick is the most likely
        // source of them.
        let mut moved = Ok(());

        for (old, new) in MOVED_TARGET_KEYS {
            let moved_to = cx.in_key(table, old, |cx, _| {
                Err::<(), _>(cx.capture(format_args!("`{old}` has moved, use {new} instead")))
            });

            if moved_to.is_err() {
                moved = Err(ErrorMarker);
            }
        }

        let remote = section == Section::Deploy;

        let kind = remote.then(|| cx.in_key(table, "kind", Self::parse));
        let host = remote.then(|| cx.in_key(table, "host", Self::deploy_hosts));
        let user = remote.then(|| cx.in_key(table, "user", Self::string));
        let port = remote.then(|| cx.in_key(table, "port", Self::port));
        let identity_file = remote.then(|| cx.in_key(table, "identity_file", Self::string));
        let options = remote.then(|| cx.in_array(table, "options", None, Self::string));
        let staging_dir = remote.then(|| cx.in_key(table, "staging_dir", Self::string));
        let commands = (section == Section::Install)
            .then(|| cx.in_array(table, "commands", None, Self::local_command));

        let sudo = cx.in_key(table, "sudo", Self::boolean);
        let bin_dir = cx.in_key(table, "bin_dir", Self::string);
        let unit_dir = cx.in_key(table, "unit_dir", Self::string);
        let build = cx.in_key(table, "build", |cx, value| match value {
            toml::Value::Array(..) => Err(cx.capture(
                "`build` as a list of commands has moved, use `commands` in the `[build]` section instead",
            )),
            value => cx.build(value),
        });
        let files = cx.in_array(table, "files", None, Self::deploy_file);
        let post_install = cx.in_array(table, "post_install", None, Self::config_command);
        let post_start = cx.in_array(table, "post_start", None, Self::config_command);
        let systemd = cx.in_key(table, "systemd", Self::systemd);

        moved?;

        Ok(Deploy {
            kind: kind.transpose()?.flatten(),
            default_profile: None,
            profiles: BTreeMap::new(),
            host: host.transpose()?.flatten().unwrap_or_default(),
            user: user.transpose()?.flatten(),
            port: port.transpose()?.flatten(),
            identity_file: identity_file.transpose()?.flatten(),
            options: options.transpose()?.unwrap_or_default(),
            sudo: sudo?,
            bin_dir: bin_dir?,
            unit_dir: unit_dir?,
            staging_dir: staging_dir.transpose()?.flatten(),
            build: build?.unwrap_or_default(),
            commands: commands.transpose()?.unwrap_or_default(),
            files: files?,
            post_install: post_install?,
            post_start: post_start?,
            systemd: systemd?,
        })
    }

    fn deny_action(&self, value: toml::Value) -> Result<DenyAction, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let name = cx.require_in_key(table, "name", Self::string);
            let reason = cx.in_key(table, "reason", Self::string);

            Ok(DenyAction {
                name: name?,
                reason: reason?,
            })
        })
    }

    fn latest_action(&self, value: toml::Value) -> Result<LatestAction, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let name = cx.require_in_key(table, "name", Self::string);
            let version = cx.require_in_key(table, "version", Self::string);

            Ok(LatestAction {
                name: name?,
                version: version?,
            })
        })
    }

    fn actions(&self, value: toml::Value) -> Result<Actions, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let deny = cx.in_array(table, "deny", Some(("name", "reason")), Self::deny_action);

            let latest = cx.in_array(
                table,
                "latest",
                Some(("name", "version")),
                Self::latest_action,
            );

            Ok(Actions {
                deny: deny?,
                latest: latest?,
            })
        })
    }
}

/// Load a configuration from the given path.
pub(crate) fn load<'a>(
    paths: Paths<'a>,
    templating: &Templating,
    defaults: &'a toml::Table,
) -> Result<Config<'a>> {
    let mut cx = Cx::new(paths, RelativePath::new(""), templating);

    let (base, mut repos) = 'out: {
        let Ok(Some(config)) = cx.config() else {
            break 'out (RepoConfig::default(), BTreeMap::new());
        };

        let Ok((base, repos)) = load_base(&mut cx, config) else {
            break 'out (RepoConfig::default(), BTreeMap::new());
        };

        (base, repos)
    };

    // Repos are only ever declared in configuration, so every repo here comes
    // from a `[repo."<path>"]` section. Layer each repo's own configuration on
    // top of what declared it.
    let declared = repos
        .iter()
        .map(|(path, config)| (path.to_owned(), config.sources.clone(), config.urls.clone()))
        .collect::<Vec<_>>();

    for (path, sources, urls) in declared {
        let path = cx.current.join(&path);
        let updates = load_repo(&mut cx, path.clone());
        let config = repos.entry(path).or_default();

        if let Ok(updates) = updates {
            config.merge_with(updates);
        }

        config.sources.extend(sources);
        config.urls.extend(urls);
    }

    let errors = cx.errors.into_inner();

    if !errors.is_empty() {
        let count = errors.len();

        for error in errors {
            tracing::error!("Error: {error}");

            for e in error.chain().skip(1) {
                tracing::error!("  Caused by: {e}");
            }
        }

        let what = match count {
            1 => "error",
            _ => "errors",
        };

        return Err(anyhow!(
            "{}: Failed to load configuration due to {count} {what}",
            cx.config_path
        ));
    }

    Ok(Config {
        base,
        repos,
        defaults,
    })
}

fn load_base(
    cx: &mut Cx<'_>,
    table: toml::Value,
) -> Result<(RepoConfig, BTreeMap<RelativePathBuf, RepoConfig>), ErrorMarker> {
    cx.with_table(table, |cx, table| {
        let base = cx.repo_table(table);

        let repos = cx.in_table(table, "repo", |cx, id, value| {
            Ok((RelativePathBuf::from(id), cx.repo(value)?))
        });

        Ok((base?, repos?))
    })
}

fn load_repo(cx: &mut Cx, current: RelativePathBuf) -> Result<RepoConfig, ErrorMarker> {
    let old_config_path = mem::replace(&mut cx.config_path, current.join(KICK_TOML));
    let old_current = mem::replace(&mut cx.current, current);

    let out = match cx.config() {
        Ok(config) => match config {
            Some(config) => cx.repo(config),
            None => Ok(RepoConfig::default()),
        },
        Err(error) => Err(error),
    };

    cx.config_path = old_config_path;
    cx.current = old_current;
    out
}

struct FormatOptional<T, F>(Option<T>, F)
where
    F: Fn(&mut fmt::Formatter<'_>, &T) -> fmt::Result;

impl<T, F> fmt::Display for FormatOptional<T, F>
where
    T: fmt::Display,
    F: Fn(&mut fmt::Formatter<'_>, &T) -> fmt::Result,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(value) = &self.0 {
            (self.1)(f, value)?;
        }

        Ok(())
    }
}

fn merge_map(target: &mut toml::Table, source: toml::Table) {
    for (key, value) in source {
        match target.entry(key) {
            toml::map::Entry::Vacant(e) => {
                e.insert(value);
            }
            toml::map::Entry::Occupied(e) => match (e.into_mut(), value) {
                (toml::Value::Table(target), toml::Value::Table(source)) => {
                    merge_map(target, source);
                }
                (toml::Value::Array(target), toml::Value::Array(source)) => {
                    target.extend(source);
                }
                (target, source) => {
                    *target = source;
                }
            },
        }
    }
}

/// Access `rpm.requies` through [`Config::get_all`].
pub(crate) fn rpm_requires(config: &RepoConfig) -> &[RpmRequire] {
    &config.package.rpm.requires
}

/// Access `deb.depends` through [`Config::get_all`].
pub(crate) fn deb_depends(config: &RepoConfig) -> &[DebDependency] {
    &config.package.deb.depends
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use relative_path::RelativePath;

    use super::{Build, CommandLine, ConfigCommand, Cx, Deploy, DeployKind, Section, SystemdScope};
    use crate::ctxt::Paths;
    use crate::shell::Shell;
    use crate::templates::Templating;

    fn line(line: &str, sudo: bool) -> ConfigCommand {
        ConfigCommand {
            line: CommandLine::Line(line.to_owned()),
            sudo,
        }
    }

    fn args(args: &[&str]) -> ConfigCommand {
        ConfigCommand {
            line: CommandLine::Args(args.iter().map(|a| (*a).to_owned()).collect()),
            sudo: false,
        }
    }

    fn parse_build(source: &str) -> (Option<Build>, usize) {
        let templating = Templating::new().unwrap();

        let paths = Paths {
            root: Path::new("."),
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let cx = Cx::new(paths, RelativePath::new(""), &templating);
        let value: toml::Value = toml::from_str(source).unwrap();
        let build = cx.build(value).ok();
        let errors = cx.errors.borrow().len();
        (build, errors)
    }

    fn parse_install(source: &str) -> (Option<Deploy>, usize) {
        parse_target(source, Section::Install)
    }

    fn parse_deploy(source: &str) -> (Option<Deploy>, usize) {
        parse_target(source, Section::Deploy)
    }

    fn parse_target(source: &str, section: Section) -> (Option<Deploy>, usize) {
        let templating = Templating::new().unwrap();

        let paths = Paths {
            root: Path::new("."),
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let cx = Cx::new(paths, RelativePath::new(""), &templating);
        let value: toml::Value = toml::from_str(source).unwrap();
        let deploy = cx.target(value, section).ok();
        let errors = cx.errors.borrow().len();
        (deploy, errors)
    }

    #[test]
    fn deploy_profiles() {
        let (deploy, errors) = parse_deploy(
            r#"
default_profile = "local"

[profiles.local]
kind = "local"
bin_dir = "~/.cargo/bin"

[profiles.local.systemd]
scope = "user"
working_directory = "~/repo/kanban"

[profiles.remote]
host = "moore"
"#,
        );

        assert_eq!(errors, 0);

        let deploy = deploy.unwrap();
        assert_eq!(deploy.default_profile.as_deref(), Some("local"));
        assert_eq!(deploy.kind, None);
        assert_eq!(
            deploy
                .profiles
                .keys()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["local", "remote"]
        );

        let local = &deploy.profiles["local"];
        assert_eq!(local.kind, Some(DeployKind::Local));

        let systemd = local.systemd.as_ref().unwrap();
        assert_eq!(systemd.scope, Some(SystemdScope::User));
        // NB: `scope` is an option, not a variable.
        assert!(!systemd.variables.contains_key("scope"));
        assert!(systemd.variables.contains_key("working_directory"));

        assert_eq!(deploy.profiles["remote"].host, ["moore"]);
    }

    #[test]
    fn deploy_socket() {
        let (deploy, errors) = parse_deploy(
            r#"
[systemd]
scope = "user"

[systemd.socket]
listen_stream = "%t/kanban/kanban.sock"
socket_mode = "0600"

[profiles.system.systemd.socket]
listen_stream = "/run/kanban/kanban.sock"
socket_group = "kanban"

[profiles.plain.systemd]
socket = false
"#,
        );

        assert_eq!(errors, 0);

        let deploy = deploy.unwrap();

        let systemd = deploy.systemd.as_ref().unwrap();
        // NB: `socket` is an option, not a variable.
        assert!(!systemd.variables.contains_key("socket"));

        let socket = systemd.socket.as_ref().unwrap();
        assert_eq!(socket.enabled, None);
        assert!(socket.template.is_none());
        assert_eq!(
            socket.variables["listen_stream"].as_str(),
            Some("%t/kanban/kanban.sock")
        );

        let system = deploy.with_profile("system").unwrap();
        let socket = system.systemd.unwrap().socket.unwrap();
        assert_eq!(
            socket.variables["listen_stream"].as_str(),
            Some("/run/kanban/kanban.sock")
        );
        assert_eq!(socket.variables["socket_mode"].as_str(), Some("0600"));
        assert_eq!(socket.variables["socket_group"].as_str(), Some("kanban"));

        let plain = deploy.with_profile("plain").unwrap();
        let socket = plain.systemd.unwrap().socket.unwrap();
        assert_eq!(socket.enabled, Some(false));

        let (deploy, errors) = parse_deploy(
            r#"
[systemd]
socket = true
"#,
        );

        assert_eq!(errors, 0);
        let socket = deploy.unwrap().systemd.unwrap().socket.unwrap();
        assert_eq!(socket.enabled, Some(true));

        let (_, errors) = parse_deploy(
            r#"
[systemd]
socket = 42
"#,
        );

        assert_eq!(errors, 1);
    }

    #[test]
    fn deploy_start_commands() {
        let (deploy, errors) = parse_deploy(
            r#"
post_install = ["kanban --db /var/lib/kanban/kanban.db install"]

[profiles.system]
post_install = [{ command = "systemd-sysusers", sudo = true }]
post_start = [{ command = ["echo", "all done"] }]
"#,
        );

        assert_eq!(errors, 0);

        let system = deploy.unwrap().with_profile("system").unwrap();

        assert_eq!(
            system.post_install,
            [
                line("kanban --db /var/lib/kanban/kanban.db install", false),
                line("systemd-sysusers", true),
            ]
        );

        assert_eq!(system.post_start, [args(&["echo", "all done"])]);

        for source in [
            r#"post_install = [""]"#,
            r#"post_install = [42]"#,
            r#"post_install = [[]]"#,
            r#"post_start = [{ sudo = true }]"#,
            r#"post_start = [{ command = "x", user = "root" }]"#,
            // Renamed to `post_install`.
            r#"pre_start = ["x"]"#,
            // Moved into `[build]`.
            r#"pre_build = ["x"]"#,
            r#"binary = "x""#,
            r#"build = ["x"]"#,
            r#"build_features = ["x"]"#,
            // Only available in `[install]`.
            r#"commands = ["x"]"#,
        ] {
            let (_, errors) = parse_deploy(source);
            assert_eq!(errors, 1, "{source}");
        }
    }

    #[test]
    fn deploy_without_profiles() {
        let (deploy, errors) = parse_deploy(r#"host = "moore""#);
        assert_eq!(errors, 0);

        let deploy = deploy.unwrap();
        assert_eq!(deploy.host, ["moore"]);
        assert!(deploy.profiles.is_empty());
        assert!(deploy.default_profile.is_none());
    }

    #[test]
    fn deploy_rejects_bad_profiles() {
        // Profiles do not nest.
        let (_, errors) = parse_deploy(
            r#"
[profiles.a.profiles.b]
host = "moore"
"#,
        );
        assert_eq!(errors, 1);

        let (_, errors) = parse_deploy(
            r#"
[profiles.a]
default_profile = "a"
"#,
        );
        assert_eq!(errors, 1);

        let (_, errors) = parse_deploy(r#"kind = "ftp""#);
        assert_eq!(errors, 1);

        let (_, errors) = parse_deploy(
            r#"
[systemd]
scope = "session"
"#,
        );
        assert_eq!(errors, 1);
    }

    #[test]
    fn deploy_merges_profiles_by_name() {
        let (base, _) = parse_deploy(
            r#"
[profiles.local]
kind = "local"

[profiles.local.systemd]
scope = "user"
"#,
        );

        let (more, _) = parse_deploy(
            r#"
default_profile = "local"

[profiles.local]
bin_dir = "~/bin"

[profiles.remote]
host = "moore"
"#,
        );

        let mut deploy = base.unwrap();
        deploy.merge_with(more.unwrap());

        assert_eq!(deploy.default_profile.as_deref(), Some("local"));

        let local = &deploy.profiles["local"];
        assert_eq!(local.kind, Some(DeployKind::Local));
        assert_eq!(local.bin_dir.as_deref(), Some("~/bin"));
        assert_eq!(
            local.systemd.as_ref().and_then(|s| s.scope),
            Some(SystemdScope::User)
        );
        assert!(deploy.profiles.contains_key("remote"));
    }

    #[test]
    fn build_section() {
        let (build, errors) = parse_build(
            r#"
binary = "kanban"
package = "kanban"
profile = "release-lto"
features = ["bundle"]
pre_build = ["trunk build --release", ["sh", "-c", "echo a b"]]
commands = [{ command = "cargo xtask dist" }]
"#,
        );

        assert_eq!(errors, 0);

        let build = build.unwrap();
        assert_eq!(build.binary.as_deref(), Some("kanban"));
        assert_eq!(build.package.as_deref(), Some("kanban"));
        assert_eq!(build.profile.as_deref(), Some("release-lto"));
        assert_eq!(build.features, ["bundle"]);
        assert_eq!(
            build.pre_build,
            [
                line("trunk build --release", false),
                args(&["sh", "-c", "echo a b"])
            ]
        );
        assert_eq!(build.commands, [line("cargo xtask dist", false)]);

        // NB: Nothing which kick runs itself goes through sudo.
        for source in [
            r#"pre_build = [{ command = "x", sudo = true }]"#,
            r#"build_features = ["x"]"#,
        ] {
            let (_, errors) = parse_build(source);
            assert_eq!(errors, 1, "{source}");
        }
    }

    #[test]
    fn build_layers_into_targets() {
        let (deploy, errors) = parse_deploy(
            r#"
[build]
features = ["a"]

[profiles.local.build]
profile = "dev"
features = ["b"]
"#,
        );

        assert_eq!(errors, 0);

        let mut build = Build {
            binary: Some(String::from("kanban")),
            ..Build::default()
        };

        build.merge_with(deploy.unwrap().with_profile("local").unwrap().build);
        assert_eq!(build.binary.as_deref(), Some("kanban"));
        assert_eq!(build.profile.as_deref(), Some("dev"));
        assert_eq!(build.features, ["a", "b"]);
    }

    #[test]
    fn install_section() {
        let (install, errors) = parse_install(
            r#"
commands = ["cargo install --path . --locked"]
post_install = ["kanban install"]
bin_dir = "~/bin"

[profiles.service.systemd]
args = ["serve"]
"#,
        );

        assert_eq!(errors, 0);

        let install = install.unwrap();
        assert_eq!(
            install.commands,
            [line("cargo install --path . --locked", false)]
        );
        assert_eq!(install.post_install, [line("kanban install", false)]);
        assert_eq!(install.bin_dir.as_deref(), Some("~/bin"));
        assert!(install.profiles["service"].systemd.is_some());
    }

    #[test]
    fn install_rejects_remote_options() {
        for source in [
            r#"commands = [""]"#,
            r#"commands = [{ command = "x", sudo = true }]"#,
            r#"kind = "local""#,
            r#"host = "moore""#,
            r#"user = "integration""#,
            r#"staging_dir = "x""#,
            r#"unknown = 1"#,
            "[profiles.a]
host = \"moore\"",
        ] {
            let (_, errors) = parse_install(source);
            assert_eq!(errors, 1, "{source}");
        }
    }

    #[test]
    fn command_forms() {
        let command = ConfigCommand::split("  cargo  install --path . ").unwrap();
        assert_eq!(command.argv(), ["cargo", "install", "--path", "."]);
        assert_eq!(command.to_shell(Shell::Bash), "  cargo  install --path . ");
        assert_eq!(ConfigCommand::split("   "), None);

        let command = args(&["echo", "a b", "$HOME"]);
        assert_eq!(command.argv(), ["echo", "a b", "$HOME"]);
        assert_eq!(command.to_shell(Shell::Bash), r#"echo "a b" "\$HOME""#);
    }
}
