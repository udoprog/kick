use std::collections::BTreeSet;
use std::path::Path;
use std::process::Stdio;
use std::str;

use anyhow::{bail, ensure, Result};

use crate::config::{Distribution, Os};
use crate::process::Command;
use crate::workflows::Eval;

use super::{ActionRunners, Actions, BatchConfig, Remediations};

const CURL: &str = "curl --proto '=https' --tlsv1.2 -sSf";
const DEBIAN_WANTED: &[&str] = &["gcc", "pkg-config", "nodejs", "libssl-dev"];
const FEDORA_WANTED: &[&str] = &["gcc", "nodejs", "openssl-devel"];
const NODE_VERSION: u32 = 22;

/// Preparations that need to be done before running a batch.
pub(crate) struct Session {
    /// WSL distributions that need to be available.
    pub(super) dists: BTreeSet<Distribution>,
    /// Whether we are running on the current distro.
    pub(super) is_same: bool,
    /// Loaded distributions.
    prepared_dists: BTreeSet<Distribution>,
    /// Whether we have prepared the same distro.
    is_same_prepare: bool,
    /// Actions that need to be synchronized.
    pub(super) actions: Actions,
    /// Runners associated with actions.
    runners: ActionRunners,
    /// Files that should be removed at the end of the session.
    remove_paths: Vec<Box<Path>>,
    /// Unique sequence number.
    sequence: u32,
    /// Keep temporary files.
    keep: bool,
}

impl Session {
    /// Construct a new preparation.
    pub(crate) fn new(c: &BatchConfig<'_, '_>) -> Self {
        Self {
            dists: BTreeSet::new(),
            is_same: false,
            prepared_dists: BTreeSet::new(),
            is_same_prepare: false,
            actions: Actions::default(),
            runners: ActionRunners::default(),
            remove_paths: Vec::new(),
            sequence: 0,
            keep: c.keep,
        }
    }

    /// Access a unique sequence number.
    pub(crate) fn sequence(&mut self) -> u32 {
        let sequence = self.sequence;
        self.sequence += 1;
        sequence
    }

    /// Mark a file that should be removed.
    pub(super) fn remove_path(&mut self, path: impl AsRef<Path>) {
        self.remove_paths.push(Box::from(path.as_ref()));
    }

    /// Access actions to prepare.
    pub(super) fn actions_mut(&mut self) -> &mut Actions {
        &mut self.actions
    }

    /// Access prepared runners, if they are available.
    pub(crate) fn runners(&self) -> &ActionRunners {
        &self.runners
    }

    /// Run all preparations.
    pub(super) fn prepare(
        &mut self,
        config: &BatchConfig<'_, '_>,
        eval: &Eval,
    ) -> Result<Remediations> {
        let mut suggestions = Remediations::default();

        if !self.dists.is_empty() {
            self.prepare_wsl(config, &mut suggestions)?;
        }

        if self.is_same && !self.is_same_prepare {
            self.prepare_same(config, &mut suggestions)?;
            self.is_same_prepare = true;
        }

        self.actions
            .synchronize(&mut self.runners, config.cx, eval)?;
        Ok(suggestions)
    }

    fn prepare_wsl(&mut self, config: &BatchConfig, suggestions: &mut Remediations) -> Result<()> {
        let Some(wsl) = config.cx.system.wsl.first() else {
            bail!("WSL not available");
        };

        let available = wsl.list()?;

        let available = available
            .into_iter()
            .map(Distribution::from_string_ignore_case)
            .collect::<BTreeSet<_>>();

        for &dist in &self.dists {
            if !self.prepared_dists.insert(dist) {
                continue;
            }

            let mut has_wsl = true;

            if dist != Distribution::Other && !available.contains(&dist) {
                let mut command = Command::new(&wsl.path);

                command
                    .arg("--install")
                    .arg(dist.to_string())
                    .arg("--no-launch");

                suggestions.command(format_args!("WSL distro {dist} is missing"), command);

                match dist {
                    Distribution::Ubuntu | Distribution::Debian => {
                        let mut command = Command::new("ubuntu");
                        command.arg("install");
                        suggestions.command(
                            format_args!("WSL distro {dist} needs to be configured"),
                            command,
                        );
                    }
                    _ => {}
                }

                has_wsl = false;
            }

            let has_rustup = if has_wsl {
                let mut command = wsl.shell(&config.path, dist);

                let status = command
                    .args(["rustup", "--version"])
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()?;

                status.success()
            } else {
                false
            };

            if !has_rustup {
                let mut command = wsl.shell(&config.path, dist);
                command
                    .args(["bash", "-i", "-c"])
                    .arg(format!("{CURL} https://sh.rustup.rs | sh -s -- -y"));
                suggestions.command(format_args!("WSL distro {dist} is missing rustup"), command);
            }

            match dist {
                Distribution::Ubuntu | Distribution::Debian => {
                    let mut wanted = BTreeSet::new();

                    for &package in DEBIAN_WANTED {
                        wanted.insert(package);
                    }

                    if has_wsl {
                        let mut command = wsl.shell(&config.path, dist);

                        command.args(["dpkg-query"]);

                        dpkg_query_list_installed(command, |package| {
                            wanted.remove(package);
                        })?;
                    }

                    let wants_node_js = wanted.remove("nodejs");

                    if !wanted.is_empty() {
                        let packages = wanted.into_iter().collect::<Vec<_>>();
                        let packages = packages.join(" ");

                        let mut command = wsl.shell(&config.path, dist);

                        command.args(["bash", "-i", "-c"]).arg(format!(
                            "sudo apt update && sudo apt install --yes {packages}"
                        ));

                        suggestions
                            .command(format_args!("Some packages in {dist} are missing"), command);
                    }

                    if wants_node_js {
                        let mut command = wsl.shell(&config.path, dist);
                        command.args(["bash", "-i", "-c"]).arg(format!(
                            "{CURL} https://deb.nodesource.com/setup_{NODE_VERSION}.x | sudo -E bash - && sudo apt-get install -y nodejs"
                        ));
                        suggestions.command(
                            format_args!("Missing a modern nodejs version in {dist}"),
                            command,
                        );
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Prepare the same distro.
    fn prepare_same(&mut self, config: &BatchConfig, suggestions: &mut Remediations) -> Result<()> {
        let os = &config.cx.current_os;
        let dist = &config.cx.dist;

        match os {
            Os::Windows => {}
            Os::Linux => {
                let Some(sudo) = config.cx.system.sudo.first() else {
                    bail!("sudo not available");
                };

                match dist {
                    Distribution::Ubuntu | Distribution::Debian => {
                        let Some(dpkg_query) = config.cx.system.dpkg_query.first() else {
                            bail!("dpkg-query not available");
                        };

                        let Some(apt) = config.cx.system.apt.first() else {
                            bail!("apt not available");
                        };

                        let mut wanted = BTreeSet::new();

                        for &package in FEDORA_WANTED {
                            wanted.insert(package);
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

                            suggestions.command(
                                format_args!("Some packages in {dist} are missing"),
                                command,
                            );
                        }
                    }
                    Distribution::Fedora => {
                        let Some(dnf) = config.cx.system.dnf.first() else {
                            bail!("dnf not available");
                        };

                        let mut wanted = BTreeSet::new();

                        for &package in FEDORA_WANTED {
                            wanted.insert(package);
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

                            suggestions.command(
                                format_args!("Some packages in {dist} are missing"),
                                command,
                            );
                        }
                    }
                    Distribution::Other => {}
                }
            }
            Os::Mac => {}
            Os::Other(..) => {}
        }

        Ok(())
    }

    /// Clean up any remaining temporary files.
    fn cleanup(&mut self) {
        if self.keep {
            return;
        }

        for path in self.remove_paths.drain(..) {
            tracing::trace!(?path, "Removing file");
            _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.cleanup();
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
