use std::collections::BTreeSet;
use std::path::Path;
use std::process::Stdio;
use std::str;

use anyhow::{Result, bail};
use semver::Version;

use crate::config::{Distribution, Os};
use crate::process::Command;
use crate::workflows::Eval;

use super::{ActionRunners, Actions, Remediations, SessionConfig};

const CURL: &str = "curl --proto '=https' --tlsv1.2 -sSf";
const DEBIAN_WANTED: &[&'static str] = &["gcc", "pkg-config", "libssl-dev"];
const FEDORA_WANTED: &[&'static str] = &["gcc", "openssl-devel"];

/// Preparations that need to be done before running a batch.
pub(crate) struct Session {
    /// WSL distributions that need to be available.
    pub(super) dists: BTreeSet<Distribution>,
    /// Whether we are running on the current distro.
    pub(super) is_same: bool,
    /// Loaded distributions.
    prepared_dists: BTreeSet<Distribution>,
    /// Prepared node versions.
    prepared_node_versions: BTreeSet<(Distribution, Version)>,
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
    pub(crate) fn new(c: &SessionConfig<'_, '_>) -> Self {
        Self {
            dists: BTreeSet::new(),
            is_same: false,
            prepared_dists: BTreeSet::new(),
            prepared_node_versions: BTreeSet::new(),
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
        config: &SessionConfig<'_, '_>,
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

        while let Some(version) = self.actions.found_node_versions.pop_first() {
            self.prepare_node_version(&version, config, &mut suggestions)?;
        }

        Ok(suggestions)
    }

    fn prepare_wsl(
        &mut self,
        config: &SessionConfig,
        suggestions: &mut Remediations,
    ) -> Result<()> {
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
                    config.cx.system.debian_ensure_wsl_packages(
                        &config.path,
                        dist,
                        DEBIAN_WANTED,
                        suggestions,
                    )?;
                }
                _ => {}
            }
        }

        Ok(())
    }

    /// Prepare the same distro.
    fn prepare_same(
        &mut self,
        config: &SessionConfig,
        suggestions: &mut Remediations,
    ) -> Result<()> {
        let os = &config.cx.os;
        let dist = &config.cx.dist;

        match os {
            Os::Windows => {}
            Os::Linux => match dist {
                Distribution::Ubuntu | Distribution::Debian => {
                    config
                        .cx
                        .system
                        .debian_ensure_packages(dist, DEBIAN_WANTED, suggestions)?;
                }
                Distribution::Fedora => {
                    config
                        .cx
                        .system
                        .fedora_ensure_packages(dist, FEDORA_WANTED, suggestions)?;
                }
                Distribution::Other => {}
            },
            Os::Mac => {}
            Os::Other(..) => {}
        }

        Ok(())
    }

    fn prepare_node_version(
        &mut self,
        version: &Version,
        config: &SessionConfig,
        suggestions: &mut Remediations,
    ) -> Result<()> {
        let mut to_be_done = Vec::new();

        for &dist in &self.dists {
            if !self.prepared_node_versions.insert((dist, version.clone())) {
                continue;
            }

            to_be_done.push(dist);
        }

        if to_be_done.is_empty() {
            return Ok(());
        }

        match config.cx.os {
            Os::Windows => {
                let Some(wsl) = config.cx.system.wsl.first() else {
                    bail!("WSL not available");
                };

                let available = wsl.list()?;

                let available = available
                    .into_iter()
                    .map(Distribution::from_string_ignore_case)
                    .collect::<BTreeSet<_>>();

                for dist in to_be_done {
                    if !available.contains(&dist) {
                        tracing::warn!(
                            "Cannot ensure node version {version} in WSL distro {dist} because the distro is not available"
                        );
                        continue;
                    }

                    match dist {
                        Distribution::Ubuntu | Distribution::Debian => {
                            config.cx.system.debian_ensure_wsl_packages(
                                &config.path,
                                dist,
                                [format!("nodejs{}", version.major)],
                                suggestions,
                            )?;
                        }
                        _ => {}
                    }
                }
            }
            Os::Linux => {
                for dist in to_be_done {
                    match dist {
                        Distribution::Ubuntu | Distribution::Debian => {
                            config.cx.system.debian_ensure_packages(
                                dist,
                                [format!("nodejs{}", version.major)],
                                suggestions,
                            )?;
                        }
                        Distribution::Fedora => {
                            config.cx.system.fedora_ensure_packages(
                                dist,
                                [format!("nodejs{}", version.major)],
                                suggestions,
                            )?;
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
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
