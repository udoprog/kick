use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;

use crate::changes::Change;
use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::rust_version::{self, RustVersion};
use crate::utils::CommandRepr;
use crate::workspace::Workspace;

/// Oldest version where rust-version was introduced.
const RUST_VERSION_SUPPORTED: RustVersion = RustVersion::new(1, 56);
/// Oldest version to test by default.
const EARLIEST: RustVersion = RUST_VERSION_SUPPORTED;
/// Final fallback version to use if *nothing* else can be figured out.
const LATEST: RustVersion = RustVersion::new(1, 68);
/// Default command to build.
const DEFAULT_COMMAND: [&str; 2] = ["cargo", "build"];

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// Verbose output.
    #[arg(long)]
    verbose: bool,
    /// Keep the existing Cargo.lock file. By default this is moved out of the
    /// way, to test that version selection selects a version of all
    /// dependencies which can compile.
    #[arg(long)]
    keep_cargo_lock: bool,
    /// Don't save the new MSRV in project `Cargo.toml` files.
    #[arg(long)]
    no_save: bool,
    /// Do not remove [dev-dependencies].
    #[arg(long)]
    no_remove_dev_dependencies: bool,
    /// Earliest minor version to test. Default: 2021.
    ///
    /// Supports the following special values, apart from minor version numbers:
    /// * 2018 - The first Rust version to support 2018 edition.
    /// * 2021 - The first Rust version to support 2021 edition.
    /// * rust-version - The rust-version specified in the Cargo.toml of the
    ///   project. Note that the first version to support rust-version is 2021.
    /// * workspace - The first Rust version to support workspaces.
    /// * rustc - The version reported by your local rustc.
    #[arg(long, verbatim_doc_comment)]
    earliest: Option<String>,
    /// Latest minor version to test. Default is `rustc`.
    ///
    /// Supports the following special values, apart from minor version numbers:
    /// * 2018 - The first Rust version to support 2018 edition.
    /// * 2021 - The first Rust version to support 2021 edition.
    /// * rust-version - The rust-version specified in the Cargo.toml of the
    ///   project. Note that the first version to support rust-version is 2021.
    /// * workspace - The first Rust version to support workspaces.
    /// * rustc - The version reported by your local rustc.
    #[arg(long, verbatim_doc_comment)]
    latest: Option<String>,
    /// Command to test with.
    ///
    /// This is run through `rustup run <version> <command>`, the default
    /// command is `cargo build --all-targets`.
    command: Vec<String>,
}

#[tracing::instrument(skip_all)]
pub(crate) fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    for module in cx.modules() {
        let span = tracing::info_span!("build", source = ?module.source(), module = module.path().as_str());
        let _enter = span.enter();
        let workspace = module.workspace(cx)?;
        build(cx, &workspace, module, opts).with_context(|| module.path().to_owned())?;
    }

    Ok(())
}

fn build(cx: &Ctxt<'_>, workspace: &Workspace, module: &Module, opts: &Opts) -> Result<()> {
    let primary = workspace.primary_crate()?;

    let current_dir = crate::utils::to_path(module.path(), cx.root);
    let rust_version = primary.rust_version()?;

    let opts_earliest = parse_minor_version(cx, opts.earliest.as_deref(), rust_version.as_ref())?;
    let opts_latest = parse_minor_version(cx, opts.latest.as_deref(), rust_version.as_ref())?;

    let earliest = opts_earliest.unwrap_or(EARLIEST);
    let latest = opts_latest
        .or(cx.rustc_version)
        .or(rust_version)
        .unwrap_or(LATEST)
        .max(earliest);

    let cargo_lock = crate::utils::to_path(primary.manifest_dir.join("Cargo.lock"), cx.root);
    let cargo_lock_original = cargo_lock.with_extension("lock.original");

    tracing::info!("Testing Rust {earliest}-{latest}");
    let mut candidates = Bisect::new(earliest, latest);

    while let Some(current) = candidates.current() {
        let version = format!("{current}");

        let output = Command::new("rustup")
            .args(["run", &version, "rustc", "--version"])
            .stdout(Stdio::null())
            .output()?;

        if !output.status.success() {
            tracing::info!("Installing rust {version}");

            let status = Command::new("rustup")
                .args(["toolchain", "install", "--profile", "minimal", &version])
                .status()?;

            if !status.success() {
                bail!("failed to install Rust {version}");
            }
        }

        let mut restore = Vec::new();
        let mut packages = workspace.packages().cloned().collect::<Vec<_>>();

        for p in &mut packages {
            let original = p.manifest_path.with_extension("toml.original");
            let original_path = crate::utils::to_path(original, cx.root);
            let manifest_path = crate::utils::to_path(&p.manifest_path, cx.root);

            let mut save = if opts.no_remove_dev_dependencies {
                false
            } else {
                p.manifest.remove_dev_dependencies()
            };

            save |= if current < RUST_VERSION_SUPPORTED {
                p.manifest.set_rust_version(&version)?;
                true
            } else {
                p.manifest.remove_rust_version()
            };

            if save {
                move_paths(&manifest_path, &original_path)?;
                tracing::info!("Saving {}", p.manifest_path);
                p.manifest.save_to(&manifest_path)?;
                restore.push((original_path, manifest_path));
            }
        }

        if !opts.keep_cargo_lock && cargo_lock.is_file() {
            move_paths(&cargo_lock, &cargo_lock_original)?;
            restore.push((cargo_lock_original.clone(), cargo_lock.clone()));
        }

        tracing::trace!(?current_dir, "Testing against rust {version}");

        let mut rustup = Command::new("rustup");
        rustup.args(["run", &version, "--"]);

        if !opts.command.is_empty() {
            tracing::info!(
                "Testing Rust {version}: {}",
                CommandRepr::new(&opts.command[..])
            );
            rustup.args(&opts.command[..]);
        } else {
            tracing::info!(
                "Testing Rust {version}: {}",
                CommandRepr::new(&DEFAULT_COMMAND[..])
            );
            rustup.args(DEFAULT_COMMAND);
        }

        rustup.current_dir(&current_dir);

        if !opts.verbose {
            rustup.stdout(Stdio::null()).stderr(Stdio::null());
        }

        let status = rustup.status().context("Command through `rustup run`")?;

        if status.success() {
            tracing::info!("Rust {version}: ok");
            candidates.ok();
        } else {
            tracing::info!("Rust {version}: failed");
            candidates.fail();
        }

        for (from, to) in restore {
            move_paths(&from, &to)?;
        }
    }

    let Some(version) = candidates.get() else {
        tracing::warn!("No MSRV found");
        return Ok(());
    };

    if version >= RUST_VERSION_SUPPORTED {
        cx.change(Change::SetRustVersion {
            module: module.clone(),
            version,
        });
    } else {
        cx.change(Change::RemoveRustVersion {
            module: module.clone(),
            version,
        });
    }

    Ok(())
}

fn parse_minor_version(
    cx: &Ctxt<'_>,
    string: Option<&str>,
    rust_version: Option<&RustVersion>,
) -> Result<Option<RustVersion>> {
    Ok(match string {
        Some("rustc") => cx.rustc_version,
        Some("2018") => Some(rust_version::EDITION_2018),
        Some("2021") => Some(rust_version::EDITION_2021),
        Some("workspace") => Some(rust_version::WORKSPACE),
        Some("rust-version") => rust_version.copied(),
        Some(n) => Some(RustVersion::new(1, n.parse()?)),
        None => None,
    })
}

fn move_paths(from: &Path, to: &Path) -> Result<()> {
    tracing::trace!("moving {} -> {}", from.display(), to.display());

    if to.exists() {
        let _ = std::fs::remove_file(to).with_context(|| anyhow!("{}", to.display()));
    }

    std::fs::rename(from, to).with_context(|| anyhow!("{} -> {}", from.display(), to.display()))?;
    Ok(())
}

struct Bisect {
    versions: HashMap<u64, bool>,
    earliest: u64,
    current: u64,
    latest: u64,
}

impl Bisect {
    fn new(earliest: RustVersion, latest: RustVersion) -> Self {
        Self {
            versions: HashMap::new(),
            earliest: earliest.minor,
            current: midpoint(earliest.minor, latest.minor),
            latest: latest.minor,
        }
    }

    /// Get the next version that needs to be tested.
    fn current(&self) -> Option<RustVersion> {
        if self.versions.contains_key(&self.current) {
            return None;
        }

        Some(RustVersion::new(1, self.current))
    }

    /// Return a successfully tested version.
    fn get(&self) -> Option<RustVersion> {
        if *self.versions.get(&self.current)? {
            return Some(RustVersion::new(1, self.current));
        }

        None
    }

    fn ok(&mut self) {
        self.versions.insert(self.current, true);
        self.latest = self.current;
        self.current = midpoint(self.earliest, self.latest);
    }

    fn fail(&mut self) {
        self.versions.insert(self.current, false);
        self.earliest = (self.current + 1).min(self.latest);
        self.current = midpoint(self.earliest, self.latest);
    }
}

fn midpoint(start: u64, end: u64) -> u64 {
    (start + (end - start) / 2).clamp(start, end)
}
