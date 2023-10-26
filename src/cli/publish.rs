use std::collections::{HashMap, HashSet};
use std::ffi::OsString;

use anyhow::{bail, Context, Result};
use clap::Parser;

use crate::changes::Change;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::workspace;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// Provide a list of crates which we do not verify locally by adding
    /// `--no-verify` to cargo publish.
    #[arg(long = "no-verify", name = "crate")]
    no_verify: Vec<String>,
    /// Skip publishing a crate.
    #[arg(long = "skip", name = "crate")]
    skip: Vec<String>,
    /// Perform a dry run by passing `--dry-run` to cargo publish.
    #[arg(long)]
    dry_run: bool,
    /// Options passed to `cargo publish`.
    cargo_publish: Vec<OsString>,
}

pub(crate) fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    for repo in cx.repos() {
        publish(cx, opts, repo).with_context(|| repo.path().to_owned())?;
    }

    Ok(())
}

#[tracing::instrument(skip_all, fields(source = ?repo.source(), path = repo.path().as_str()))]
fn publish(cx: &Ctxt<'_>, opts: &Opts, repo: &Repo) -> Result<()> {
    let Some(workspace) = workspace::open(cx, repo)? else {
        bail!("not a workspace");
    };

    let no_verify = opts.no_verify.iter().cloned().collect::<HashSet<_>>();
    let skip = opts.skip.iter().cloned().collect::<HashSet<_>>();

    let mut packages = Vec::new();
    let mut deps = HashMap::<_, Vec<_>>::new();
    let mut rev = HashMap::<_, u32>::new();
    let mut pending = HashSet::new();

    for package in workspace.packages() {
        if !package.is_publish() {
            continue;
        }

        let from = package.name()?;

        if let Some(dependencies) = package.manifest().dependencies(&workspace) {
            for dep in dependencies.iter() {
                let to = dep.package()?;

                deps.entry(from.to_string())
                    .or_default()
                    .push(to.to_string());

                *rev.entry(to.to_string()).or_default() += 1;
            }
        }

        packages.push(package);
        pending.insert(from.to_string());
    }

    let mut ordered = Vec::new();

    while !pending.is_empty() {
        let start = pending.len();

        for package in &packages {
            let name = package.name()?;

            if !pending.contains(name) {
                continue;
            }

            let revs = rev.get(name).copied().unwrap_or_default();

            if revs != 0 {
                continue;
            }

            for dep in deps.remove(name).into_iter().flatten() {
                let n = rev.entry(dep).or_default();
                *n = (*n).saturating_sub(1);
            }

            pending.remove(name);
            ordered.push(package);
        }

        if start == pending.len() {
            bail!("failed to order packages for publishing");
        }
    }

    for package in ordered.into_iter().rev() {
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
