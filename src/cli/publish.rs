use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsString;

use anyhow::{bail, Result};
use clap::Parser;

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
    let mut deps = HashMap::<_, BTreeSet<_>>::new();
    let mut rev = HashMap::<_, BTreeSet<_>>::new();
    let mut pending = HashSet::new();

    for package in workspace.packages() {
        if !package.is_publish() {
            continue;
        }

        let from = package.name()?;

        let a = package.manifest().dependencies(&workspace);
        let b = package.manifest().dev_dependencies(&workspace);
        let c = package.manifest().build_dependencies(&workspace);

        let it = [a, b, c].into_iter().flatten().flat_map(|d| d.iter());

        for dep in it {
            let to = dep.package()?;

            deps.entry(from.to_string())
                .or_default()
                .insert(to.to_string());

            rev.entry(to.to_string())
                .or_default()
                .insert(from.to_string());
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

            if matches!(rev.get(name), Some(revs) if !revs.is_empty()) {
                continue;
            }

            for dep in deps.remove(name).into_iter().flatten() {
                rev.entry(dep).or_default().remove(name);
            }

            pending.remove(name);
            ordered.push(package);
        }

        if start == pending.len() {
            bail!("Failed to order packages for publishing: {pending:?}");
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
