use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use musli::{Decode, Encode};

use crate::cargo::{BUILD_DEPENDENCIES, DEPENDENCIES, DEV_DEPENDENCIES, Dependency, Manifest};
use crate::changes::{AllowDirty, Change, NoVerify};
use crate::cli::WithRepos;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::Command;
use crate::utils::move_paths;
use crate::workspace::{CARGO_TOML, Crates};

/// Directory, relative to the package being published, where the original
/// manifest is kept while a modified one is being published. Cargo never
/// packages the `target` directory of a package.
const BACKUP_DIR: [&str; 2] = ["target", "kick-publish"];
/// File name of the kept manifest.
const BACKUP_NAME: &str = "Cargo.toml.keep";
/// Where older versions of kick kept the original manifest.
const LEGACY_BACKUP_NAME: &str = "Cargo.toml.keep";

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Provide a list of crates which we do not verify locally by adding
    /// --no-verify to cargo publish.
    #[arg(long)]
    no_verify: Vec<String>,
    /// Provide a list of crates which we do not verify locally by adding
    /// --allow-dirty to cargo publish.
    #[arg(long)]
    allow_dirty: Vec<String>,
    /// Provide a list of crates which we remove all dev-dependencies from
    /// since it contributes to circular dependencies during publishing.
    #[arg(long)]
    remove_dev: Vec<String>,
    /// Do not remove dev-dependencies which form a cycle with the crates being
    /// published.
    ///
    /// By default, when a crate has to be published before a crate it has a
    /// versioned dev-dependency on, that dev-dependency is temporarily removed
    /// from its Cargo.toml while publishing, since cargo would otherwise fail
    /// to resolve it against the registry. With this option the manifest is
    /// left alone and only --no-verify is passed instead.
    #[arg(long)]
    keep_circular_dev: bool,
    /// Skip publishing a crate.
    #[arg(long)]
    skip: Vec<String>,
    /// Perform a dry run by passing --dry-run to cargo publish.
    #[arg(long)]
    dry_run: bool,
    /// Options passed to cargo publish.
    #[arg(long = "option", short = 'O')]
    cargo_options: Vec<OsString>,
    /// List of crates to consider when publishing.
    crates: Vec<String>,
}

pub(crate) fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    with_repos.run(
        "cargo publish",
        format_args!("publish: {opts:?}"),
        |cx, repo| publish(cx, opts, repo),
    )?;

    Ok(())
}

#[tracing::instrument(skip_all)]
fn publish(cx: &Ctxt<'_>, opts: &Opts, repo: &Repo) -> Result<()> {
    let workspace = repo.workspace(cx)?;
    let no_verify = opts.no_verify.iter().cloned().collect::<HashSet<_>>();
    let allow_dirty = opts.allow_dirty.iter().cloned().collect::<HashSet<_>>();
    let remove_dev = opts.remove_dev.iter().cloned().collect::<HashSet<_>>();
    let skip = opts.skip.iter().cloned().collect::<HashSet<_>>();
    let filter = opts.crates.iter().cloned().collect::<HashSet<_>>();

    let filter = |name: &str| {
        if skip.contains(name) {
            return false;
        }

        if filter.is_empty() {
            return true;
        }

        filter.contains(name)
    };

    for planned in plan(workspace)? {
        let name = planned.name;

        if !filter(name) {
            continue;
        }

        let circular = !planned.circular_dev.is_empty();

        let no_verify = match (no_verify.contains(name), circular && opts.keep_circular_dev) {
            (true, _) => Some(NoVerify::Argument),
            (_, true) => Some(NoVerify::Circular),
            _ => None,
        };

        let remove_dev = remove_dev.contains(name);

        let circular_dev = if opts.keep_circular_dev || remove_dev {
            Vec::new()
        } else {
            planned.circular_dev
        };

        let allow_dirty = match (
            remove_dev,
            !circular_dev.is_empty(),
            allow_dirty.contains(name),
        ) {
            (true, _, _) => Some(AllowDirty::DevDependency),
            (_, true, _) => Some(AllowDirty::CircularDevDependency),
            (_, _, true) => Some(AllowDirty::Argument),
            _ => None,
        };

        cx.change(Change::Publish {
            name: name.to_owned(),
            manifest_dir: planned.manifest.dir().to_owned(),
            dry_run: opts.dry_run,
            no_verify,
            allow_dirty,
            remove_dev,
            args: opts.cargo_options.clone(),
            circular_dev,
            depends_on: planned.depends_on,
        });
    }

    Ok(())
}

/// A dev-dependency which is removed from a manifest while publishing since
/// it refers to a crate which is published later.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub(crate) struct CircularDev {
    /// The `cfg` of the `[target.<cfg>.dev-dependencies]` table the dependency
    /// is declared in, or `None` for `[dev-dependencies]`.
    pub(crate) target: Option<String>,
    /// The key the dependency is declared under.
    pub(crate) key: String,
    /// The package the dependency refers to.
    pub(crate) package: String,
}

impl fmt::Display for CircularDev {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.target {
            Some(target) => write!(f, "[target.'{target}'.dev-dependencies] ")?,
            None => write!(f, "[dev-dependencies] ")?,
        }

        write!(f, "{}", self.key)?;

        if self.key != self.package {
            write!(f, " (package {})", self.package)?;
        }

        Ok(())
    }
}

/// A crate in the order it should be published.
struct Planned<'a> {
    manifest: &'a Manifest,
    name: &'a str,
    /// Dev-dependencies which have to be removed while publishing since they
    /// refer to crates which are published later.
    circular_dev: Vec<CircularDev>,
    /// Crates in the workspace which have to be published before this one.
    depends_on: Vec<String>,
}

struct Node<'a> {
    manifest: &'a Manifest,
    name: &'a str,
    deps: Vec<Dep<'a>>,
}

/// Order the publishable crates in the workspace so that every crate is
/// published after the crates it depends on.
///
/// Cycles can only be broken through dev-dependencies. When no crate can be
/// published, the dev-dependencies of a crate which only waits on
/// dev-dependencies are dropped from the graph and reported as
/// [`CircularDev`]. Dev-dependencies without a version are ignored, since cargo
/// strips them when packaging.
fn plan(workspace: &Crates) -> Result<Vec<Planned<'_>>> {
    let mut nodes = Vec::new();

    for manifest in workspace.packages() {
        let Some(p) = manifest.as_package() else {
            continue;
        };

        if !p.is_publish() {
            continue;
        }

        nodes.push(Node {
            manifest,
            name: p.name()?,
            deps: Vec::new(),
        });
    }

    let index = nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.name, i))
        .collect::<HashMap<_, _>>();

    for node in &mut nodes {
        node.deps = collect_deps(node.manifest, workspace)?
            .into_iter()
            .filter(|d| d.name != node.name && index.contains_key(d.name))
            .collect();

        for dep in &node.deps {
            tracing::trace!("{} -> {dep}", node.name);
        }
    }

    let n = nodes.len();
    let edge = |i: usize, d: usize| index[nodes[i].deps[d].name];

    // Dependencies which have not been published yet, as indexes into the
    // deps of each node.
    let mut remaining = nodes
        .iter()
        .map(|n| (0..n.deps.len()).collect::<Vec<_>>())
        .collect::<Vec<_>>();

    let mut broken = vec![Vec::new(); n];
    let mut pending = vec![true; n];
    let mut ordered = Vec::with_capacity(n);

    while ordered.len() < n {
        let mut progress = false;

        for i in 0..n {
            if !pending[i] || !remaining[i].is_empty() {
                continue;
            }

            tracing::trace!("Adding: {}", nodes[i].name);
            pending[i] = false;
            ordered.push(i);
            progress = true;

            for (j, remaining) in remaining.iter_mut().enumerate() {
                remaining.retain(|&d| edge(j, d) != i);
            }
        }

        if progress {
            continue;
        }

        // Every pending crate waits on another pending crate, so there is a
        // cycle. Break it by dropping the dev-dependencies of a crate which
        // only waits on dev-dependencies, preferring one which is part of a
        // cycle itself so that unrelated dev-dependencies are kept.
        let only_dev = |i: usize| {
            pending[i]
                && remaining[i]
                    .iter()
                    .all(|&d| matches!(nodes[i].deps[d].kind, DepKind::Dev))
        };

        let on_cycle = |start: usize| {
            let mut visited = vec![false; n];
            let mut queue = remaining[start]
                .iter()
                .map(|&d| edge(start, d))
                .collect::<Vec<_>>();

            while let Some(i) = queue.pop() {
                if i == start {
                    return true;
                }

                if std::mem::replace(&mut visited[i], true) {
                    continue;
                }

                queue.extend(remaining[i].iter().map(|&d| edge(i, d)));
            }

            false
        };

        let candidate = (0..n)
            .find(|&i| only_dev(i) && on_cycle(i))
            .or_else(|| (0..n).find(|&i| only_dev(i)));

        let Some(i) = candidate else {
            let pending = (0..n)
                .filter(|&i| pending[i])
                .map(|i| {
                    let deps = remaining[i]
                        .iter()
                        .map(|&d| nodes[i].deps[d].to_string())
                        .collect::<Vec<_>>();
                    format!("{} -> [{}]", nodes[i].name, deps.join(", "))
                })
                .collect::<Vec<_>>();

            let ordered = ordered.iter().map(|&i| nodes[i].name).collect::<Vec<_>>();

            bail!(
                "Failed to order packages for publishing, since they have cyclic dependencies which are not dev-dependencies:\nPending: {}\nOrdered: {ordered:?}",
                pending.join(", ")
            );
        };

        tracing::trace!("Breaking dev-dependencies of: {}", nodes[i].name);
        let mut deps = std::mem::take(&mut remaining[i]);
        broken[i].append(&mut deps);
    }

    let mut output = Vec::with_capacity(n);

    for i in ordered {
        let node = &nodes[i];

        let mut circular_dev = Vec::new();

        for &d in &broken[i] {
            let dep = &node.deps[d];

            circular_dev.push(CircularDev {
                target: dep.target.map(str::to_owned),
                key: dep.key.to_owned(),
                package: dep.name.to_owned(),
            });
        }

        let mut depends_on = Vec::<String>::new();

        for (d, dep) in node.deps.iter().enumerate() {
            if broken[i].contains(&d) || depends_on.iter().any(|n| n == dep.name) {
                continue;
            }

            depends_on.push(dep.name.to_owned());
        }

        output.push(Planned {
            manifest: node.manifest,
            name: node.name,
            circular_dev,
            depends_on,
        });
    }

    Ok(output)
}

/// Collect every dependency of a manifest which matters for publishing,
/// including those in `[target.<cfg>.*]` tables.
fn collect_deps<'a>(manifest: &'a Manifest, workspace: &'a Crates) -> Result<Vec<Dep<'a>>> {
    let mut tables = Vec::new();

    let top = [
        (DepKind::Runtime, manifest.dependencies(workspace)),
        (DepKind::Dev, manifest.dev_dependencies(workspace)),
        (DepKind::Build, manifest.build_dependencies(workspace)),
    ];

    for (kind, deps) in top {
        tables.extend(deps.map(|deps| (kind, None, deps)));
    }

    for (kind, key) in [
        (DepKind::Runtime, DEPENDENCIES),
        (DepKind::Dev, DEV_DEPENDENCIES),
        (DepKind::Build, BUILD_DEPENDENCIES),
    ] {
        for (target, deps) in manifest.target_dependencies(workspace, key) {
            tables.push((kind, Some(target), deps));
        }
    }

    let mut output = Vec::new();

    for (kind, target, deps) in tables {
        for dep in deps.iter() {
            // Cargo strips dev-dependencies without a version when packaging,
            // so they don't constrain the publishing order.
            if matches!(kind, DepKind::Dev) && !dep.has_version()? {
                continue;
            }

            output.push(Dep::new(kind, target, dep)?);
        }
    }

    Ok(output)
}

#[derive(Debug, Clone, Copy)]
enum DepKind {
    Runtime,
    Dev,
    Build,
}

struct Dep<'a> {
    name: &'a str,
    key: &'a str,
    target: Option<&'a str>,
    kind: DepKind,
}

impl<'a> Dep<'a> {
    #[inline]
    fn new(kind: DepKind, target: Option<&'a str>, dep: Dependency<'a>) -> Result<Self> {
        Ok(Self {
            name: *dep.package()?,
            key: dep.key(),
            target,
            kind,
        })
    }
}

impl fmt::Display for Dep<'_> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.name)?;

        match &self.kind {
            DepKind::Runtime => (),
            DepKind::Dev => {
                write!(f, " (dev)")?;
            }
            DepKind::Build => {
                write!(f, " (build)")?;
            }
        }

        if let Some(target) = self.target {
            write!(f, " (target {target})")?;
        }

        Ok(())
    }
}

impl fmt::Debug for Dep<'_> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Dep({self})")
    }
}

/// Remove dev-dependencies from a manifest before it is published.
///
/// If `all` is set, every dev-dependency table is removed, otherwise only the
/// `circular` entries. Returns a description of what was removed.
pub(crate) fn strip_dev_dependencies(
    manifest: &mut Manifest,
    all: bool,
    circular: &[CircularDev],
) -> Vec<String> {
    let mut removed = Vec::new();

    if all {
        if manifest.remove_all(DEV_DEPENDENCIES) {
            removed.push(String::from("all dev-dependencies due to `--remove-dev`"));
        }

        return removed;
    }

    for dev in circular {
        if manifest.remove_dev_dependency(dev.target.as_deref(), &dev.key) {
            removed.push(format!("{dev}, since `{}` is published later", dev.package));
        } else {
            tracing::warn!("{}: Missing dev-dependency {dev}", manifest.path());
        }
    }

    removed
}

/// Fail if an interrupted publish left a modified manifest behind.
pub(crate) fn check_leftover_backup(dir: &Path) -> Result<()> {
    for path in [
        backup_dir(dir).join(BACKUP_NAME),
        dir.join(LEGACY_BACKUP_NAME),
    ] {
        if path.exists() {
            bail!(
                "{}: Found the original manifest of an interrupted publish, so {} might be modified. Move it back over {CARGO_TOML}, or remove it if it is stale, before publishing again",
                path.display(),
                dir.join(CARGO_TOML).display(),
            );
        }
    }

    Ok(())
}

fn backup_dir(dir: &Path) -> PathBuf {
    BACKUP_DIR
        .iter()
        .fold(dir.to_owned(), |path, c| path.join(c))
}

/// Keeps the original manifest of a package while a modified one is
/// published, and puts it back when dropped. Since kick catches Ctrl-C, this
/// also happens when publishing is interrupted.
pub(crate) struct ManifestBackup {
    manifest: PathBuf,
    backup: PathBuf,
    /// Directories which were created for the backup and should be removed.
    created: Vec<PathBuf>,
    armed: bool,
}

impl ManifestBackup {
    /// Back up the manifest in `dir` and replace it with `manifest`.
    pub(crate) fn replace(dir: &Path, manifest: &Manifest) -> Result<Self> {
        check_leftover_backup(dir)?;

        let mut this = Self {
            manifest: dir.join(CARGO_TOML),
            backup: backup_dir(dir).join(BACKUP_NAME),
            created: Vec::new(),
            armed: false,
        };

        let mut current = dir.to_owned();

        for c in BACKUP_DIR {
            current.push(c);

            if !current.is_dir() {
                fs::create_dir(&current)
                    .with_context(|| anyhow!("Creating {}", current.display()))?;
                this.created.push(current.clone());
            }
        }

        fs::copy(&this.manifest, &this.backup).with_context(|| {
            anyhow!(
                "Copying {} to {}",
                this.manifest.display(),
                this.backup.display()
            )
        })?;

        this.armed = true;

        manifest
            .save_to(&this.manifest)
            .with_context(|| anyhow!("Writing {}", this.manifest.display()))?;

        Ok(this)
    }

    /// Put the original manifest back.
    pub(crate) fn restore(&mut self) -> Result<()> {
        if std::mem::take(&mut self.armed) {
            move_paths(&self.backup, &self.manifest)?;
        }

        for dir in self.created.drain(..).rev() {
            // Only removes the directory if it is empty, which it isn't if
            // something else put files in it.
            _ = fs::remove_dir(&dir);
        }

        Ok(())
    }
}

impl Drop for ManifestBackup {
    #[inline]
    fn drop(&mut self) {
        if let Err(error) = self.restore() {
            tracing::error!(
                "Failed to restore {} from {}: {error}",
                self.manifest.display(),
                self.backup.display()
            );
        }
    }
}

/// The outcome of running cargo publish.
pub(crate) struct CargoPublish {
    pub(crate) status: ExitStatus,
    /// Cargo reported that the crate is already published.
    pub(crate) already_exists: bool,
}

/// Run a `cargo publish` command for the crate `name`, passing its output
/// through while checking whether the crate is already published.
pub(crate) fn run_cargo_publish(command: &mut Command, name: &str) -> Result<CargoPublish> {
    if io::stderr().is_terminal() && std::env::var_os("CARGO_TERM_COLOR").is_none() {
        command.env("CARGO_TERM_COLOR", "always");
    }

    command.stderr(Stdio::piped());

    let mut child = command.spawn()?;
    let stderr = child.stderr()?;

    let mut already_exists = false;
    let mut reader = BufReader::new(stderr);
    let mut line = Vec::new();
    let mut out = io::stderr();

    loop {
        line.clear();

        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }

        _ = out.write_all(&line);
        already_exists |= is_already_exists(&String::from_utf8_lossy(&line), name);
    }

    let status = child.wait()?;

    Ok(CargoPublish {
        status,
        already_exists,
    })
}

/// Test if a line of cargo output says that the crate `name` already exists
/// on the registry, like `error: crate foo@1.0.0 already exists on crates.io
/// index`.
fn is_already_exists(line: &str, name: &str) -> bool {
    let Some((_, rest)) = line.split_once(&format!("crate {name}@")) else {
        return false;
    };

    rest.contains(" already exists on ")
}

#[cfg(test)]
mod tests;
