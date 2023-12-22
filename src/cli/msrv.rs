use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;

use crate::changes::Change;
use crate::ctxt::Ctxt;
use crate::manifest;
use crate::model::Repo;
use crate::process::Command;
use crate::rust_version::{self, RustVersion};

/// Oldest version where rust-version was introduced.
const RUST_VERSION_SUPPORTED: RustVersion = RustVersion::new(1, 56, None);
/// Oldest version to test by default.
const EARLIEST: RustVersion = RUST_VERSION_SUPPORTED;
/// Final fallback version to use if *nothing* else can be figured out.
const LATEST: RustVersion = RustVersion::new(1, 68, None);
/// Default command to build.
const DEFAULT_COMMAND: [&str; 3] = ["cargo", "build", "--workspace"];

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Verbose output.
    #[arg(long)]
    verbose: bool,
    /// Keep the existing Cargo.lock file. By default this is moved out of the
    /// way, to test that version selection selects a version of all
    /// dependencies which can compile.
    #[arg(long)]
    keep_cargo_lock: bool,
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
    #[arg(long, verbatim_doc_comment, value_name = "version-spec")]
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
    #[arg(long, verbatim_doc_comment, value_name = "version-spec")]
    latest: Option<String>,
    /// Command to test with.
    ///
    /// This is run through `rustup run <version> <command>`, the default
    /// command is `cargo build --workspace`.
    #[arg(value_name = "command")]
    command: Vec<String>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    with_repos!(
        cx,
        "find msrv",
        format_args!("msrv: {opts:?}"),
        |cx, repo| { msrv(cx, repo, opts) }
    );

    Ok(())
}

#[tracing::instrument(skip_all)]
fn msrv(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let crates = repo.workspace(cx)?;
    let primary = crates.primary_package()?;

    let current_dir = cx.to_path(repo.path());
    let rust_version = primary.rust_version();

    let opts_earliest = parse_minor_version(cx, opts.earliest.as_deref(), rust_version.as_ref())?;
    let opts_latest = parse_minor_version(cx, opts.latest.as_deref(), rust_version.as_ref())?;

    let earliest = opts_earliest.unwrap_or(EARLIEST);

    let latest = opts_latest
        .or(cx.rustc_version)
        .or(rust_version)
        .unwrap_or(LATEST)
        .max(earliest);

    let cargo_lock = cx.to_path(primary.manifest().dir().join("Cargo.lock"));
    let cargo_lock_original = cargo_lock.with_extension("lock.original");

    tracing::info!("Testing Rust {earliest}-{latest}");
    let mut candidates = Bisect::new(earliest, latest);

    while let Some(version) = candidates.current() {
        let version_string = version.to_string();

        let output = Command::new("rustup")
            .args(["run", &version_string, "rustc", "--version"])
            .stdout(Stdio::null())
            .output()?;

        if !output.status.success() {
            tracing::info!("Installing rust {version}");

            let status = Command::new("rustup")
                .args([
                    "toolchain",
                    "install",
                    "--profile",
                    "minimal",
                    &version_string,
                ])
                .status()?;

            if !status.success() {
                bail!("failed to install Rust {version}");
            }
        }

        let mut restore = Vec::new();

        for p in crates.packages() {
            let original = p.manifest().path().with_extension("toml.original");
            let original_path = cx.to_path(original);
            let manifest_path = cx.to_path(p.manifest().path());

            let mut manifest = p.manifest().clone();

            let mut save = if opts.no_remove_dev_dependencies {
                false
            } else {
                manifest.remove(manifest::DEV_DEPENDENCIES)
            };

            save |= if version < RUST_VERSION_SUPPORTED {
                manifest.set_rust_version(&version)?;
                true
            } else {
                manifest.remove_rust_version()
            };

            if save {
                move_paths(&manifest_path, &original_path)?;
                tracing::trace!("Saving {}", p.manifest().path());
                manifest.save_to(&manifest_path)?;
                restore.push((original_path.to_owned(), manifest_path));
            }
        }

        if !opts.keep_cargo_lock && cargo_lock.is_file() {
            move_paths(&cargo_lock, &cargo_lock_original)?;
            restore.push((cargo_lock_original.clone(), cargo_lock.clone()));
        }

        tracing::trace!(?current_dir, "Testing against rust {version}");

        let mut rustup = Command::new("rustup");
        rustup.args(["run", &version_string, "--"]);

        if !opts.command.is_empty() {
            rustup.args(&opts.command[..]);
        } else {
            rustup.args(DEFAULT_COMMAND);
        }

        rustup.current_dir(&current_dir);

        if !opts.verbose {
            rustup.stdout(Stdio::null()).stderr(Stdio::null());
        }

        tracing::info!("Testing Rust {version}: {}", rustup.display());

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
        bail!("No MSRV found");
    };

    if version >= RUST_VERSION_SUPPORTED {
        cx.change(Change::SetRustVersion {
            repo: (**repo).clone(),
            version,
        });
    } else {
        cx.change(Change::RemoveRustVersion {
            repo: (**repo).clone(),
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
        Some(n) => Some(RustVersion::new(1, n.parse()?, None)),
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

        Some(RustVersion::new(1, self.current, None))
    }

    /// Return a successfully tested version.
    fn get(&self) -> Option<RustVersion> {
        if *self.versions.get(&self.current)? {
            return Some(RustVersion::new(1, self.current, None));
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
