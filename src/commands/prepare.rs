use std::collections::BTreeSet;
use std::process::Stdio;
use std::str;

use anyhow::{bail, ensure, Result};

use crate::config::Distribution;
use crate::process::Command;

use super::{ActionRunners, Actions, BatchConfig, Remediations};

const CURL: &str = "curl --proto '=https' --tlsv1.2 -sSf";
const DEBIAN_WANTED: &[&str] = &["gcc", "nodejs"];
const NODE_VERSION: u32 = 22;

/// Preparations that need to be done before running a batch.
pub(crate) struct Prepare {
    /// WSL distributions that need to be available.
    pub(super) dists: BTreeSet<Distribution>,
    /// Loaded distributions.
    pub(super) prepared_dists: BTreeSet<Distribution>,
    /// Actions that need to be synchronized.
    pub(super) actions: Actions,
    /// Runners associated with actions.
    runners: ActionRunners,
    /// If the preparation has changed.
    pub(super) changed_actions: bool,
}

impl Prepare {
    /// Construct a new preparation.
    pub(crate) fn new() -> Self {
        Self {
            dists: BTreeSet::new(),
            prepared_dists: BTreeSet::new(),
            actions: Actions::default(),
            runners: ActionRunners::default(),
            changed_actions: false,
        }
    }

    /// Access actions to prepare.
    pub(super) fn actions_mut(&mut self) -> &mut Actions {
        &mut self.actions
    }

    /// Run all preparations.
    pub(super) fn prepare(&mut self, config: &BatchConfig<'_, '_>) -> Result<Remediations> {
        let mut suggestions = Remediations::default();

        if !self.dists.is_empty() {
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
                    suggestions
                        .command(format_args!("WSL distro {dist} is missing rustup"), command);
                }

                match dist {
                    Distribution::Ubuntu | Distribution::Debian => {
                        let mut wanted = BTreeSet::new();

                        for &package in DEBIAN_WANTED {
                            wanted.insert(package);
                        }

                        if has_wsl {
                            let output = wsl
                                .shell(&config.path, dist)
                                .args([
                                    "dpkg-query",
                                    "-W",
                                    "-f",
                                    "\\${db:Status-Status} \\${Package}\n",
                                ])
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
                                    wanted.remove(package);
                                }
                            }
                        }

                        let wants_node_js = wanted.remove("nodejs");

                        if !wanted.is_empty() {
                            let packages = wanted.into_iter().collect::<Vec<_>>();
                            let packages = packages.join(" ");

                            let mut command = wsl.shell(&config.path, dist);
                            command.args(["bash", "-i", "-c"]).arg(format!(
                                "sudo apt update && sudo apt install --yes {packages}"
                            ));
                            suggestions.command(
                                format_args!("Some packages in {dist} are missing"),
                                command,
                            );
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
        }

        if self.changed_actions {
            self.actions.synchronize(&mut self.runners, config.cx)?;
            self.changed_actions = false;
        }

        Ok(suggestions)
    }

    /// Access prepared runners, if they are available.
    pub(crate) fn runners(&self) -> &ActionRunners {
        &self.runners
    }
}
