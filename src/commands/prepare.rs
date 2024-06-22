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
#[derive(Default)]
pub(crate) struct Prepare {
    /// WSL distributions that need to be available.
    pub(super) wsl: BTreeSet<Distribution>,
    /// Actions that need to be synchronized.
    pub(super) actions: Option<Actions>,
    /// Runners associated with actions.
    runners: Option<ActionRunners>,
}

impl Prepare {
    /// Access actions to prepare.
    pub(crate) fn actions_mut(&mut self) -> &mut Actions {
        self.actions.get_or_insert_with(Actions::default)
    }

    /// Run all preparations.
    pub(crate) fn prepare(&mut self, c: &BatchConfig<'_, '_>) -> Result<Remediations> {
        let mut suggestions = Remediations::default();

        if !self.wsl.is_empty() {
            let Some(wsl) = c.cx.system.wsl.first() else {
                bail!("WSL not available");
            };

            let available = wsl.list()?;

            let available = available
                .into_iter()
                .map(Distribution::from_string_ignore_case)
                .collect::<BTreeSet<_>>();

            for &dist in &self.wsl {
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
                    let mut command = wsl.shell(&c.path, dist);
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
                    let mut command = wsl.shell(&c.path, dist);
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
                                .shell(&c.path, dist)
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

                            let mut command = wsl.shell(&c.path, dist);
                            command.args(["bash", "-i", "-c"]).arg(format!(
                                "sudo apt update && sudo apt install --yes {packages}"
                            ));
                            suggestions.command(
                                format_args!("Some packages in {dist} are missing"),
                                command,
                            );
                        }

                        if wants_node_js {
                            let mut command = wsl.shell(&c.path, dist);
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

        if let Some(actions) = self.actions.take() {
            self.runners = Some(actions.synchronize(c.cx)?);
        }

        Ok(suggestions)
    }

    /// Access prepared runners, if they are available.
    pub(crate) fn runners(&self) -> Option<&ActionRunners> {
        self.runners.as_ref()
    }
}
