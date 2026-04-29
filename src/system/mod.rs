mod dnf;
mod generic;
pub(crate) mod git;
mod node;
mod wsl;

use std::collections::BTreeSet;
use std::collections::HashSet;
use std::env;
use std::env::consts::EXE_EXTENSION;
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{Context, Result, anyhow, bail, ensure};

use crate::commands::Remediations;
use crate::config::Distribution;
use crate::process::Command;

pub(crate) use self::dnf::Dnf;
pub(crate) use self::generic::Generic;
pub(crate) use self::git::Git;
pub(crate) use self::node::Node;
use self::node::NodeVersion;
pub(crate) use self::wsl::Wsl;

type ProbeFn = fn(&mut System, &Path) -> Result<()>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Name {
    Exact(&'static str),
    Prefix(&'static str),
}

const TESTS: &[(Name, ProbeFn, Allow)] = &[
    (Name::Exact("git"), git_probe, Allow::Default),
    (Name::Exact("wsl"), wsl_probe, Allow::System32),
    (Name::Exact("powershell"), powershell_probe, Allow::Default),
    (Name::Exact("bash"), bash_probe, Allow::Default),
    (Name::Exact("podman"), podman_probe, Allow::Default),
    (Name::Exact("dnf"), dnf_probe, Allow::Default),
    (Name::Exact("sudo"), sudo_probe, Allow::Default),
    (Name::Exact("dpkg-query"), dpkg_query_probe, Allow::Default),
    (Name::Exact("apt"), apt_probe, Allow::Default),
    (Name::Prefix("node"), node_probe, Allow::Default),
];

#[cfg(windows)]
const MSYS_TESTS: &[(Name, ProbeFn, Allow)] =
    &[(Name::Exact("bash"), bash_msys64_probe, Allow::Default)];

/// Detect system commands.
#[derive(Default)]
pub(crate) struct System {
    visited: HashSet<(PathBuf, Name)>,
    pub(crate) git: Vec<Git>,
    pub(crate) wsl: Vec<Wsl>,
    pub(crate) powershell: Vec<Generic>,
    pub(crate) bash: Vec<Generic>,
    pub(crate) node: Vec<Node>,
    pub(crate) podman: Vec<Generic>,
    pub(crate) dnf: Vec<Dnf>,
    pub(crate) sudo: Vec<Generic>,
    pub(crate) dpkg_query: Vec<Generic>,
    pub(crate) apt: Vec<Generic>,
}

impl System {
    /// Find a node version to use.
    pub(crate) fn find_node(&self, major: u64) -> Result<&Node> {
        if self.node.is_empty() {
            bail!("Could not find any node installations on the system");
        }

        let Some(node) = self.node.iter().find(|n| n.version.major >= major) else {
            let alternatives = self
                .node
                .iter()
                .map(|n| n.version.to_string())
                .collect::<Vec<_>>()
                .join(", ");

            bail!("Could not find node {major} on the system, alternatives: {alternatives}");
        };

        Ok(node)
    }

    /// Enumerate all tools.
    pub(crate) fn tools(&self) -> impl Iterator<Item = (&'static str, &Path, Option<String>)> + '_ {
        let a = self.git.iter().map(|c| ("git", c.path.as_path(), None));

        let b = self.wsl.iter().map(|c| ("wsl", c.path.as_path(), None));

        let c = self
            .powershell
            .iter()
            .map(|c| ("powershell", c.path.as_path(), None));

        let d = self.bash.iter().map(|c| ("bash", c.path.as_path(), None));

        let e = self
            .podman
            .iter()
            .map(|c| ("podman", c.path.as_path(), None));

        let f = self
            .node
            .iter()
            .map(|c| ("node", c.path.as_path(), Some(c.version.to_string())));

        a.chain(b).chain(c).chain(d).chain(e).chain(f)
    }

    #[cfg(windows)]
    fn windows(&mut self) -> Result<()> {
        let msys = Path::new("C:\\msys64");

        if msys.is_dir() {
            let mut path = msys.to_owned();
            path.push("usr");
            path.push("bin");
            self.walk_paths(&mut path, MSYS_TESTS)?;
        }

        Ok(())
    }

    #[cfg(not(windows))]
    fn windows(&mut self) -> Result<()> {
        Ok(())
    }

    /// Visit a full path with the given symbolic name.
    fn visit_path(&mut self, path: &Path, name: Name) -> bool {
        let Some(parent) = path.parent() else {
            return false;
        };

        let Ok(parent) = parent.canonicalize() else {
            return false;
        };

        self.visit_dir(parent, name)
    }

    /// Visit a directory.
    fn visit_dir(&mut self, path: PathBuf, test: Name) -> bool {
        self.visited.insert((path, test))
    }

    fn probe_path(&mut self, path: &Path, test_fn: ProbeFn, allow: Allow) -> Result<()> {
        let mut ignored = false;

        if cfg!(windows)
            && let Some(reason) = test_windows_ignored(path, allow)
        {
            // Non-existant files will be I/O ignored, avoid
            // spamming log entries for it.
            if reason != IgnoreReason::Io {
                tracing::debug!(path = ?path.display(), "Ignored: {reason}");
            }

            ignored = true;
        }

        if !ignored {
            tracing::trace!(path = ?path.display(), "testing");
            test_fn(self, path).with_context(|| anyhow!("Testing {}", path.display()))?;
        }

        Ok(())
    }

    fn walk_paths(&mut self, path: &mut PathBuf, tests: &[(Name, ProbeFn, Allow)]) -> Result<()> {
        let Ok(canonical) = path.canonicalize() else {
            return Ok(());
        };

        for &(name, test_fn, allow) in tests {
            if self.visit_dir(canonical.clone(), name) {
                match name {
                    Name::Exact(exact) => {
                        path.push(exact);
                        path.set_extension(EXE_EXTENSION);
                        self.probe_path(path, test_fn, allow)?;
                    }
                    Name::Prefix(prefix) => {
                        let iter = match fs::read_dir(&canonical) {
                            Ok(iter) => iter,
                            Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                            Err(e) => {
                                return Err(e)
                                    .context(format!("Reading directory {}", canonical.display()));
                            }
                        };

                        for e in iter {
                            let e = match e {
                                Ok(e) => e,
                                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                                Err(e) => {
                                    return Err(e).context(format!(
                                        "Reading directory {}",
                                        canonical.display()
                                    ));
                                }
                            };

                            let file_name = e.file_name();

                            let Some(file_name) = file_name.to_str() else {
                                continue;
                            };

                            if file_name.starts_with(prefix) {
                                path.push(file_name);
                                self.probe_path(path, test_fn, allow)?;
                                path.pop();
                            }
                        }
                    }
                }

                path.pop();
            }
        }

        Ok(())
    }

    pub(crate) fn debian_ensure_packages(
        &self,
        what: impl fmt::Display,
        packages: impl IntoIterator<Item: AsRef<str>>,
        suggestions: &mut Remediations,
    ) -> Result<()> {
        let Some(sudo) = self.sudo.first() else {
            bail!("sudo not available");
        };

        let Some(dpkg_query) = self.dpkg_query.first() else {
            bail!("dpkg-query not available");
        };

        let Some(apt) = self.apt.first() else {
            bail!("apt not available");
        };

        let mut wanted = BTreeSet::new();

        for package in packages {
            wanted.insert(package.as_ref().to_owned());
        }

        dpkg_query_list_installed(dpkg_query.command(), |package| {
            wanted.remove(package);
        })?;

        if !wanted.is_empty() {
            let packages = wanted.into_iter().collect::<Vec<_>>();
            let packages = packages.join(" ");

            let mut command = sudo.command();
            command.arg("--");
            command.arg(&apt.path);
            command.args(["install", "--yes"]);
            command.arg(&packages);

            suggestions.command(format_args!("Some packages in {what} are missing"), command);
        }

        Ok(())
    }

    pub(crate) fn debian_ensure_wsl_packages(
        &self,
        path: impl AsRef<Path>,
        dist: Distribution,
        packages: impl IntoIterator<Item: AsRef<str>>,
        suggestions: &mut Remediations,
    ) -> Result<()> {
        let path = path.as_ref();

        let Some(wsl) = self.wsl.first() else {
            bail!("WSL not available");
        };

        let mut wanted = BTreeSet::new();

        for package in packages {
            wanted.insert(package.as_ref().to_owned());
        }

        let mut command = wsl.shell(path, dist);
        command.args(["dpkg-query"]);

        dpkg_query_list_installed(command, |package| {
            wanted.remove(package);
        })?;

        if !wanted.is_empty() {
            let packages = wanted.into_iter().collect::<Vec<_>>();
            let packages = packages.join(" ");

            let mut command = wsl.shell(path, dist);

            command.args(["bash", "-i", "-c"]).arg(format!(
                "sudo apt update && sudo apt install --yes {packages}"
            ));

            suggestions.command(format_args!("Some packages in {dist} are missing"), command);
        }

        Ok(())
    }

    pub(crate) fn fedora_ensure_packages(
        &self,
        what: impl fmt::Display,
        packages: impl IntoIterator<Item: AsRef<str>>,
        suggestions: &mut Remediations,
    ) -> Result<()> {
        let Some(sudo) = self.sudo.first() else {
            bail!("sudo not available");
        };

        let Some(dnf) = self.dnf.first() else {
            bail!("dnf not available");
        };

        let mut wanted = BTreeSet::new();

        for package in packages {
            wanted.insert(package.as_ref().to_owned());
        }

        for package in dnf.list_installed()? {
            wanted.remove(package.as_str());
        }

        if !wanted.is_empty() {
            let packages = wanted.into_iter().collect::<Vec<_>>();
            let packages = packages.join(" ");

            let mut command = sudo.command();
            command.arg("--");
            command.arg(&dnf.path);
            command.args(["install", "--assumeyes"]);
            command.arg(&packages);

            suggestions.command(format_args!("Some packages in {what} are missing"), command);
        }

        Ok(())
    }
}

/// Detect everything useful we can find in the environment.
pub(crate) fn detect() -> Result<System> {
    let mut system = System::default();

    system.windows()?;

    if let Some(path) = env::var_os("GIT_PATH") {
        let path = PathBuf::from(path);

        if system.visit_path(&path, Name::Exact("git")) && probe(&path, "--version")? {
            system.git.push(Git::new(path));
        }
    }

    if let Some(path) = env::var_os("PATH") {
        for mut path in env::split_paths(&path) {
            system.walk_paths(&mut path, TESTS)?;
        }
    }

    system.visited = HashSet::new();
    Ok(system)
}

fn git_probe(s: &mut System, path: &Path) -> Result<()> {
    if probe(path, "--version")? {
        s.git.push(Git::new(path.to_owned()))
    }

    Ok(())
}

fn wsl_probe(s: &mut System, path: &Path) -> Result<()> {
    if probe(path, "--version")? {
        s.wsl.push(Wsl::new(path.to_owned()))
    }

    Ok(())
}

fn powershell_probe(s: &mut System, path: &Path) -> Result<()> {
    if probe(path, "-Help")? {
        s.powershell.push(Generic::new(path.to_owned()));
    }

    Ok(())
}

fn node_probe(s: &mut System, path: &Path) -> Result<()> {
    if let Some(version) = probe_with_out(path, "--version")?.and_then(NodeVersion::parse) {
        s.node.push(Node::new(path.to_owned(), version));
    }

    Ok(())
}

fn bash_probe(s: &mut System, path: &Path) -> Result<()> {
    if probe(path, "--version")? {
        s.bash.push(Generic::new(path.to_owned()));
    }

    Ok(())
}

fn podman_probe(s: &mut System, path: &Path) -> Result<()> {
    if probe(path, "--version")? {
        s.podman.push(Generic::new(path.to_owned()));
    }

    Ok(())
}

fn dnf_probe(s: &mut System, path: &Path) -> Result<()> {
    if probe(path, "--version")? {
        s.dnf.push(Dnf::new(path.to_owned()));
    }

    Ok(())
}

fn sudo_probe(s: &mut System, path: &Path) -> Result<()> {
    if probe(path, "--version")? {
        s.sudo.push(Generic::new(path.to_owned()));
    }

    Ok(())
}

fn dpkg_query_probe(s: &mut System, path: &Path) -> Result<()> {
    if probe(path, "--version")? {
        s.dpkg_query.push(Generic::new(path.to_owned()));
    }

    Ok(())
}

fn apt_probe(s: &mut System, path: &Path) -> Result<()> {
    if probe(path, "--version")? {
        s.apt.push(Generic::new(path.to_owned()));
    }

    Ok(())
}

#[cfg_attr(not(windows), allow(unused))]
fn bash_msys64_probe(s: &mut System, path: &Path) -> Result<()> {
    if probe(path, "--version")? {
        let mut generic = Generic::new(path.to_owned());

        if let Some(path) = path.parent() {
            generic.add_path(path.to_owned());
        }

        s.bash.push(generic);
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Allow {
    Default,
    System32,
}

#[derive(PartialEq)]
enum IgnoreReason {
    Io,
    System32,
    ZeroSized,
}

impl fmt::Display for IgnoreReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            IgnoreReason::Io => write!(f, "Failed to get metadata"),
            IgnoreReason::System32 => write!(f, "System32 binary (Probably WSL)"),
            IgnoreReason::ZeroSized => write!(f, "Zero-sized Execution Alias (Probably WSL)"),
        }
    }
}

/// Test if the path should be ignored through the policy.
fn test_windows_ignored(path: &Path, allow: Allow) -> Option<IgnoreReason> {
    if allow != Allow::System32
        && let Some(dir) = path.parent().and_then(|p| p.file_name())
        && dir.eq_ignore_ascii_case("System32")
    {
        return Some(IgnoreReason::System32);
    }

    let Ok(m) = fs::metadata(path) else {
        return Some(IgnoreReason::Io);
    };

    // On Windows, empty files are used as App Execution Aliases.
    if m.len() == 0 {
        return Some(IgnoreReason::ZeroSized);
    }

    None
}

fn probe<C, A>(command: C, arg: A) -> Result<bool>
where
    C: AsRef<OsStr>,
    A: AsRef<OsStr>,
{
    let command = command.as_ref();
    let arg = arg.as_ref();

    match std::process::Command::new(command)
        .arg(arg)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
    {
        Ok(status) => Ok(status.success()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e).context(format!(
            "{} {}",
            command.to_string_lossy(),
            arg.to_string_lossy()
        )),
    }
}

fn probe_with_out<C, A>(command: C, arg: A) -> Result<Option<String>>
where
    C: AsRef<OsStr>,
    A: AsRef<OsStr>,
{
    let command = command.as_ref();
    let arg = arg.as_ref();

    match std::process::Command::new(command)
        .arg(arg)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
    {
        Ok(output) => {
            if !output.status.success() {
                return Ok(None);
            }

            let Ok(string) = String::from_utf8(output.stdout) else {
                return Ok(None);
            };

            Ok(Some(string))
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context(format!(
            "{} {}",
            command.to_string_lossy(),
            arg.to_string_lossy()
        )),
    }
}

fn dpkg_query_list_installed(mut command: Command, mut visit: impl FnMut(&str)) -> Result<()> {
    let output = command
        .args(["-W", "-f", "\\${db:Status-Status} \\${Package}\n"])
        .stdout(Stdio::piped())
        .output()?;

    ensure!(
        output.status.success(),
        "Failed to query installed packages: {}",
        output.status
    );

    for line in output.stdout.split(|&b| b == b'\n') {
        let Ok(line) = str::from_utf8(line) else {
            continue;
        };

        if let Some(("installed", package)) = line.split_once(' ') {
            visit(package);
        }
    }

    Ok(())
}
