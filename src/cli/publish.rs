use core::fmt;
use std::collections::{HashMap, HashSet};
use std::ffi::OsString;

use anyhow::{bail, Result};
use clap::Parser;

use crate::cargo::Dependency;
use crate::changes::Change;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::workspace;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Provide a list of crates which we do not verify locally by adding
    /// `--no-verify` to cargo publish.
    #[arg(long = "no-verify", value_name = "crate")]
    no_verify: Vec<String>,
    /// Skip publishing a crate.
    #[arg(long = "skip", value_name = "crate")]
    skip: Vec<String>,
    /// Perform a dry run by passing `--dry-run` to cargo publish.
    #[arg(long)]
    dry_run: bool,
    /// Options passed to `cargo publish`.
    cargo_publish: Vec<OsString>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    with_repos!(
        cx,
        "cargo publish",
        format_args!("publish: {opts:?}"),
        |cx, repo| { publish(cx, opts, repo) }
    );

    Ok(())
}

#[tracing::instrument(skip_all)]
fn publish(cx: &Ctxt<'_>, opts: &Opts, repo: &Repo) -> Result<()> {
    let Some(workspace) = workspace::open(cx, repo)? else {
        bail!("Not a workspace");
    };

    let no_verify = opts.no_verify.iter().cloned().collect::<HashSet<_>>();
    let skip = opts.skip.iter().cloned().collect::<HashSet<_>>();

    let mut packages = Vec::new();
    let mut deps = HashMap::<_, Vec<_>>::new();
    let mut pending = HashSet::new();

    for package in workspace.packages() {
        if !package.is_publish() {
            continue;
        }

        pending.insert(package.name()?);
        packages.push(package);
    }

    for package in &packages {
        let from = package.name()?;

        let m = package.manifest();

        let a = m
            .dependencies(&workspace)
            .map(|d| d.iter().map(Dep::runtime));

        let b = m
            .dev_dependencies(&workspace)
            .map(|d| d.iter().map(Dep::dev));

        let c = m
            .build_dependencies(&workspace)
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
    let mut non_runtime_purged = false;

    while !pending.is_empty() {
        let start = pending.len();

        for package in &packages {
            let name = package.name()?;

            if !pending.contains(name) {
                tracing::trace!("Not pending: {name}");
                continue;
            }

            if let Some(deps) = deps.get(name) {
                if !deps.is_empty() {
                    tracing::trace!("Has dependencies: {deps:?}");
                    continue;
                }
            }

            for (_, deps) in deps.iter_mut() {
                deps.retain(|d| d.name != name);
            }

            tracing::trace!("Adding: {name}");
            pending.remove(name);
            ordered.push(package);
        }

        if start == pending.len() {
            if !non_runtime_purged {
                let mut any_removed = false;

                for (_, deps) in deps.iter_mut() {
                    let mut removed = false;

                    deps.retain(|d| {
                        if matches!(d.kind, DepKind::Runtime) {
                            true
                        } else {
                            removed = true;
                            false
                        }
                    });

                    any_removed |= removed;
                }

                non_runtime_purged = true;

                if any_removed {
                    continue;
                }
            }

            let ordered = ordered
                .iter()
                .map(|p| p.name())
                .collect::<Result<Vec<_>>>()?;

            bail!("Failed to order packages for publishing:\nPending: {pending:?}\nOrdered: {ordered:?}\nDependencies: {deps:?}");
        }
    }

    for package in ordered.into_iter() {
        let name = package.name()?;

        if skip.contains(name) {
            continue;
        }

        cx.change(Change::Publish {
            name: name.to_owned(),
            manifest_dir: package.manifest().dir().to_owned(),
            dry_run: opts.dry_run,
            no_verify: no_verify.contains(name),
            args: opts.cargo_publish.clone(),
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
    fn runtime(dep: Dependency<'a>) -> Result<Self> {
        Self::with_kind(DepKind::Runtime, dep)
    }

    fn dev(dep: Dependency<'a>) -> Result<Self> {
        Self::with_kind(DepKind::Dev, dep)
    }

    fn build(dep: Dependency<'a>) -> Result<Self> {
        Self::with_kind(DepKind::Build, dep)
    }

    fn with_kind(kind: DepKind, dep: Dependency<'a>) -> Result<Self> {
        Ok(Self {
            name: *dep.package()?,
            kind,
        })
    }
}

impl fmt::Display for Dep<'_> {
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
        write!(f, "Dep({})", self)
    }
}
