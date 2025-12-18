use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt;

use anyhow::{Result, bail};
use clap::Parser;

use crate::cargo::Dependency;
use crate::changes::{AllowDirty, Change, NoVerify};
use crate::cli::WithRepos;
use crate::ctxt::Ctxt;
use crate::model::Repo;

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
    /// Provide a list of crates which we remove [dev-dependencies] from since
    /// it contributes to circular dependencies during publishing.
    #[arg(long)]
    remove_dev: Vec<String>,
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

    let mut candidates = Vec::new();
    let mut deps = HashMap::<_, Vec<_>>::new();
    let mut pending = HashSet::new();

    for manifest in workspace.packages() {
        let Some(p) = manifest.as_package() else {
            continue;
        };

        if !p.is_publish() {
            continue;
        }

        pending.insert(p.name()?);
        candidates.push((manifest, p));
    }

    for &(m, package) in &candidates {
        let from = package.name()?;

        let a = m
            .dependencies(workspace)
            .map(|d| d.iter().map(Dep::runtime));

        let b = m
            .dev_dependencies(workspace)
            .map(|d| d.iter().map(Dep::dev));

        let c = m
            .build_dependencies(workspace)
            .map(|d| d.iter().map(Dep::build));

        let it = a
            .into_iter()
            .flatten()
            .chain(b.into_iter().flatten())
            .chain(c.into_iter().flatten());

        for dep in it {
            let d = dep?;

            if d.name == from || !pending.contains(d.name) {
                continue;
            }

            tracing::trace!("{from} -> {d}");
            deps.entry(from.to_string()).or_default().push(d);
        }
    }

    if tracing::enabled!(tracing::Level::TRACE) {
        for (dependent, deps) in &deps {
            for dep in deps {
                tracing::trace!("Found: {dependent} -> {dep}");
            }
        }
    }

    let mut ordered = Vec::new();
    let mut reduced_no_verify = HashSet::new();

    'outer: while !pending.is_empty() {
        let start = pending.len();

        for &(manifest, package) in &candidates {
            let name = package.name()?;

            if !pending.contains(name) {
                tracing::trace!("Not pending: {name}");
                continue;
            }

            if let Some(deps) = deps.get(name)
                && !deps.is_empty()
            {
                tracing::trace!("Has dependencies: {deps:?}");
                continue;
            }

            for (_, deps) in deps.iter_mut() {
                deps.retain(|d| d.name != name);
            }

            tracing::trace!("Adding: {name}");
            pending.remove(name);
            ordered.push((manifest, package));
        }

        if start != pending.len() {
            continue;
        }

        // Find a the first non-runtime dependency, remove it and try again.
        for (dependent, deps) in deps.iter_mut() {
            if let Some(index) = deps
                .iter()
                .position(|d| !matches!(d.kind, DepKind::Runtime))
            {
                deps.remove(index);
                reduced_no_verify.insert(dependent.clone());
                continue 'outer;
            }
        }

        // No dependencies to tweak, so we can't do anything else. Provide a
        // helpful error message.
        let ordered = ordered
            .iter()
            .map(|(_, p)| p.name())
            .collect::<Result<Vec<_>>>()?;

        bail!(
            "Failed to order packages for publishing:\nPending: {pending:?}\nOrdered: {ordered:?}\nDependencies: {deps:?}"
        );
    }

    for (manifest, p) in ordered.into_iter() {
        let name = p.name()?;

        if !filter(name) {
            continue;
        }

        let no_verify = match (no_verify.contains(name), reduced_no_verify.contains(name)) {
            (true, _) => Some(NoVerify::Argument),
            (_, true) => Some(NoVerify::Circular),
            _ => None,
        };

        let remove_dev = remove_dev.contains(name);

        let allow_dirty = match (remove_dev, allow_dirty.contains(name)) {
            (true, _) => Some(AllowDirty::DevDependency),
            (_, true) => Some(AllowDirty::Argument),
            _ => None,
        };

        cx.change(Change::Publish {
            name: name.to_owned(),
            manifest_dir: manifest.dir().to_owned(),
            dry_run: opts.dry_run,
            no_verify,
            allow_dirty,
            remove_dev,
            args: opts.cargo_options.clone(),
        });
    }

    Ok(())
}

#[derive(Debug)]
enum DepKind {
    Runtime,
    Dev,
    Build,
}

struct Dep<'a> {
    name: &'a str,
    kind: DepKind,
}

impl<'a> Dep<'a> {
    #[inline]
    fn runtime(dep: Dependency<'a>) -> Result<Self> {
        Self::with_kind(DepKind::Runtime, dep)
    }

    #[inline]
    fn dev(dep: Dependency<'a>) -> Result<Self> {
        Self::with_kind(DepKind::Dev, dep)
    }

    #[inline]
    fn build(dep: Dependency<'a>) -> Result<Self> {
        Self::with_kind(DepKind::Build, dep)
    }

    #[inline]
    fn with_kind(kind: DepKind, dep: Dependency<'a>) -> Result<Self> {
        Ok(Self {
            name: *dep.package()?,
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

        Ok(())
    }
}

impl fmt::Debug for Dep<'_> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Dep({self})")
    }
}
