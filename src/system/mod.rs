mod generic;
pub(crate) mod git;
mod node;
mod wsl;

use core::fmt;
use std::collections::HashSet;
use std::env;
use std::env::consts::EXE_EXTENSION;
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, bail, Context, Result};

pub(crate) use self::generic::Generic;
pub(crate) use self::git::Git;
pub(crate) use self::node::Node;
use self::node::NodeVersion;
pub(crate) use self::wsl::Wsl;

type ProbeFn = fn(&mut System, &Path) -> Result<()>;

const TESTS: &[(&str, ProbeFn, Allow)] = &[
    ("git", git_probe, Allow::None),
    ("wsl", wsl_probe, Allow::System32),
    ("powershell", powershell_probe, Allow::None),
    ("bash", bash_probe, Allow::None),
    ("node", node_probe, Allow::None),
];

#[cfg(windows)]
const MSYS_TESTS: &[(&str, ProbeFn, Allow)] = &[("bash", bash_probe, Allow::None)];

/// Detect system commands.
#[derive(Default)]
pub(crate) struct System {
    pub(crate) visited: HashSet<PathBuf>,
    pub(crate) git: Vec<Git>,
    pub(crate) wsl: Vec<Wsl>,
    pub(crate) powershell: Vec<Generic>,
    pub(crate) bash: Vec<Generic>,
    pub(crate) node: Vec<Node>,
}

impl System {
    /// Find a node version to use.
    pub(crate) fn find_node(&self, major: u32) -> Result<&Node> {
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

    fn walk_paths(&mut self, path: &mut PathBuf, tests: &[(&str, ProbeFn, Allow)]) -> Result<()> {
        for &(name, test, allow) in tests {
            path.push(name);
            path.set_extension(EXE_EXTENSION);

            if self.visited.insert(path.clone()) {
                let mut ignored = false;

                if cfg!(windows) {
                    if let Some(reason) = test_windows_ignored(path, allow) {
                        // Non-existant files will be I/O ignored, avoid
                        // spamming log entries for it.
                        if reason != IgnoreReason::Io {
                            tracing::debug!(path = ?path.display(), "Ignored: {reason}");
                        }

                        ignored = true;
                    }
                }

                if !ignored {
                    tracing::trace!(path = ?path.display(), "testing");
                    test(self, path).with_context(|| anyhow!("Testing {}", path.display()))?;
                }
            }

            path.pop();
        }

        Ok(())
    }
}

/// Detect everything useful we can find in the environment.
pub(crate) fn detect() -> Result<System> {
    let mut system = System::default();

    system.windows()?;

    if let Some(path) = env::var_os("GIT_PATH") {
        if let Ok(path) = PathBuf::from(path).canonicalize() {
            if system.visited.insert(path.clone()) && probe(&path, "--version")? {
                system.git.push(Git::new(path));
            }
        }
    }

    if let Some(path) = env::var_os("PATH") {
        for path in env::split_paths(&path) {
            let Ok(mut path) = path.canonicalize() else {
                continue;
            };

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

#[derive(Debug, Clone, Copy, PartialEq)]
enum Allow {
    None,
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
    if allow != Allow::System32 {
        if let Some(dir) = path.parent().and_then(|p| p.file_name()) {
            if dir.eq_ignore_ascii_case("System32") {
                return Some(IgnoreReason::System32);
            }
        }
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
