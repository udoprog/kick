use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::process::Stdio;

use anyhow::{bail, Context, Result};
use clap::Parser;
use toml_edit::Item;

use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::process::Command;
use crate::workspace;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// Actually publish packages, and don't just pretend.
    #[arg(long)]
    run: bool,
    /// Provide a list of crates which we do not verify locally by adding
    /// `--no-verify` to cargo publish.
    #[arg(long = "no-verify", name = "crate")]
    no_verify: Vec<String>,
    /// Perform a dry run by passing `--dry-run` to cargo publish.
    #[arg(long)]
    dry_run: bool,
    /// Options passed to `cargo publish`.
    cargo_publish: Vec<OsString>,
}

pub(crate) fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    for module in cx.modules() {
        publish(cx, opts, module).with_context(|| module.path().to_owned())?;
    }

    Ok(())
}

#[tracing::instrument(skip_all, fields(source = ?module.source(), path = module.path().as_str()))]
fn publish(cx: &Ctxt<'_>, opts: &Opts, module: &Module) -> Result<()> {
    let Some(workspace) = workspace::open(cx, module)? else {
        bail!("not a workspace");
    };

    let no_verify = opts.no_verify.iter().cloned().collect::<HashSet<_>>();

    let mut packages = Vec::new();
    let mut deps = HashMap::<_, Vec<_>>::new();
    let mut rev = HashMap::<_, u32>::new();
    let mut pending = HashSet::new();

    for package in workspace.packages() {
        if !package.manifest.is_publish()? {
            continue;
        }

        let from = package.manifest.crate_name()?;

        if let Some(table) = package.manifest.dependencies() {
            for (key, value) in table {
                let to = package_name(key, value);
                deps.entry(from.to_string())
                    .or_default()
                    .push(to.to_string());
                *rev.entry(to.to_string()).or_default() += 1;
            }
        }

        packages.push(package.clone());
        pending.insert(from.to_string());
    }

    let mut ordered = Vec::new();

    while !pending.is_empty() {
        let start = pending.len();

        for package in &packages {
            let name = package.manifest.crate_name()?;

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
        let name = package.manifest.crate_name()?;

        if !opts.run {
            tracing::info!(
                "{}: would publish: {} (with --run)",
                package.manifest_dir,
                name
            );
            continue;
        }

        tracing::info!("{}: publishing: {}", package.manifest_dir, name);

        let mut command = Command::new("cargo");

        command.args(["publish"]);

        if no_verify.contains(name) {
            command.arg("--no-verify");
        }

        if opts.dry_run {
            command.arg("--dry-run");
        }

        command
            .args(&opts.cargo_publish)
            .stdin(Stdio::null())
            .current_dir(package.manifest_dir.to_path(cx.root));

        let status = command.status()?;

        if !status.success() {
            bail!("{}: failed to publish: {status}", package.manifest_dir);
        }

        tracing::info!("{status}");
    }

    Ok(())
}

/// Extract package name.
fn package_name<'a>(key: &'a str, dep: &'a Item) -> &'a str {
    if let Some(Item::Value(value)) = dep.get("package") {
        if let Some(value) = value.as_str() {
            return value;
        }
    }

    key
}
