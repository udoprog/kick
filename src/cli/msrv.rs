use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;

use crate::cargo::rust_version::NO_PUBLISH_VERSION_OMIT;
use crate::cargo::{self, RustVersion};
use crate::changes::Change;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::Command;

/// Oldest version where rust-version was introduced.
const RUST_VERSION_SUPPORTED: RustVersion = RustVersion::new(1, 56);
/// Oldest version to test by default.
const EARLIEST: RustVersion = RUST_VERSION_SUPPORTED;
/// Final fallback version to use if *nothing* else can be figured out.
const LATEST: RustVersion = RustVersion::new(1, 68);
/// Default command to build.
const DEFAULT_COMMAND: [&str; 2] = ["cargo", "build"];

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
    /// By default, packages which are publish = false will not be built. This
    /// causes such packages to be included.
    #[arg(long)]
    include_no_publish: bool,
    /// Earliest minor version to test. Default: 2021.
    ///
    /// Supports the following special values, apart from minor version numbers:
    /// * 2018 - The first Rust version to support 2018 edition.
    /// * 2021 - The first Rust version to support 2021 edition.
    /// * rust-version - The rust-version specified in the Cargo.toml of the
    ///   project. Note that the first version to support rust-version is 2021.
    /// * workspace - The first Rust version to support workspaces.
    /// * rustc - The version reported by your local rustc.
    #[arg(long, value_name = "version-spec")]
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
    #[arg(long, value_name = "version-spec")]
    latest: Option<String>,
    /// Command to test with.
    ///
    /// This is run through `rustup run <version> <command>`, the default
    /// command is `cargo build`. The command will be run with the argument
    /// `--manifest-path <path>`, which will be the path to the `Cargo.toml` of
    /// the package being built.
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

    let mut restore = Restore::default();

    let mut packages = Vec::new();

    for p in crates.packages() {
        if !p.is_publish() && !opts.include_no_publish {
            continue;
        }

        packages.push(p);
    }

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
                bail!("Failed to install Rust {version}");
            }
        }

        for manifest in crates.manifests() {
            let original = manifest.path().with_extension("toml.original");
            let original_path = cx.to_path(original);
            let manifest_path = cx.to_path(manifest.path());

            let mut manifest = manifest.clone();

            let mut save = false;

            if !opts.no_remove_dev_dependencies {
                tracing::info!("{}: Removing dev-dependencies", manifest_path.display());
                save |= manifest.remove_all(cargo::DEV_DEPENDENCIES);
            }

            if version < RUST_VERSION_SUPPORTED {
                tracing::info!("{}: Removing rust-version (since rust-version=\"{version}\" is less than {RUST_VERSION_SUPPORTED})", manifest_path.display());
                save |= manifest.remove_rust_version();
            } else {
                tracing::info!(
                    "{}: Setting rust-version=\"{version}\"",
                    manifest_path.display()
                );
                save |= manifest.set_rust_version(&version);
            }

            if let Some(mut package) = manifest.as_package_mut() {
                if !package.is_publish() && version < NO_PUBLISH_VERSION_OMIT {
                    tracing::info!("{}: Setting version = \"0.0.0\" (since publish = false and rust-version=\"{version}\" is less than {NO_PUBLISH_VERSION_OMIT})", manifest_path.display());
                    save |= package.set_version("0.0.0");
                }
            }

            if save {
                move_paths(&manifest_path, &original_path)?;
                tracing::trace!("Saving {}", manifest.path());
                manifest.save_to(&manifest_path)?;
                restore
                    .paths
                    .push((original_path.to_owned(), manifest_path));
            }
        }

        if !opts.keep_cargo_lock && cargo_lock.is_file() {
            move_paths(&cargo_lock, &cargo_lock_original)?;
            restore
                .paths
                .push((cargo_lock_original.clone(), cargo_lock.clone()));
        }

        tracing::trace!(?current_dir, "Testing against rust {version}");

        let mut all_ok = true;

        for p in &packages {
            let manifest_path = cx.to_path(p.manifest().path());

            let mut rustup = Command::new("rustup");
            rustup.args(["run", &version_string, "--"]);

            if !opts.command.is_empty() {
                rustup.args(&opts.command[..]);
            } else {
                rustup.args(DEFAULT_COMMAND);
            }

            rustup.args([OsStr::new("--manifest-path"), manifest_path.as_os_str()]);
            rustup.current_dir(&current_dir);

            if !opts.verbose {
                rustup.stdout(Stdio::null()).stderr(Stdio::null());
            }

            tracing::info!("Testing Rust {version}: {}", rustup.display());
            let status = rustup.status().context("Command through `rustup run`")?;

            if !status.success() {
                all_ok = false;
                break;
            }
        }

        if all_ok {
            tracing::info!("Rust {version}: ok");
            candidates.ok();
        } else {
            tracing::info!("Rust {version}: failed");
            candidates.fail();
        }

        restore.restore();
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
        Some("2018") => Some(cargo::rust_version::EDITION_2018),
        Some("2021") => Some(cargo::rust_version::EDITION_2021),
        Some("workspace") => Some(cargo::rust_version::WORKSPACE),
        Some("rust-version") => rust_version.copied(),
        Some(n) => Some(RustVersion::new(1, n.parse()?)),
        None => None,
    })
}

fn move_paths(from: &Path, to: &Path) -> Result<()> {
    tracing::trace!("moving {} -> {}", from.display(), to.display());

    if to.exists() {
        let _ = fs::remove_file(to).with_context(|| anyhow!("{}", to.display()));
    }

    fs::rename(from, to).with_context(|| anyhow!("{} -> {}", from.display(), to.display()))?;
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

#[derive(Default)]
struct Restore {
    paths: Vec<(PathBuf, PathBuf)>,
}

impl Restore {
    fn restore(&mut self) {
        for (from, to) in self.paths.drain(..) {
            if let Err(error) = move_paths(&from, &to) {
                tracing::error!("Failed to restore {}: {}", from.display(), error);
            }
        }
    }
}

impl Drop for Restore {
    fn drop(&mut self) {
        self.restore();
    }
}
