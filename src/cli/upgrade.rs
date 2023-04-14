use anyhow::{Context, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::module_sets::ModuleSet;
use crate::process::Command;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Pass `--workspace` to `cargo upgrade`.
    #[arg(long)]
    workspace: bool,
    /// Pass `--exclude <package>` to `cargo upgrade`.
    #[arg(long)]
    exclude: Vec<String>,
    /// Store the outcome if this run into the sets `good` and `bad`, to be used
    /// later with `--set <id>` command.
    ///
    /// The `good` set will contain modules for which the `cargo upgrade`
    /// command exited successfully, while the `bad` set for which they failed.
    #[arg(long)]
    store_sets: bool,
    /// Extra upgrade arguments.
    upgrade_args: Vec<String>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut good = ModuleSet::default();
    let mut bad = ModuleSet::default();

    for module in cx.modules() {
        upgrade(cx, opts, module, &mut good, &mut bad).with_context(|| module.path().to_owned())?;
    }

    let hint = format!("upgrade: {:?}", opts);
    cx.sets.save("good", good, opts.store_sets, &hint);
    cx.sets.save("bad", bad, opts.store_sets, &hint);
    Ok(())
}

#[tracing::instrument(skip_all, fields(source = ?module.source(), path = module.path().as_str()))]
fn upgrade(
    cx: &Ctxt<'_>,
    opts: &Opts,
    module: &Module,
    good: &mut ModuleSet,
    bad: &mut ModuleSet,
) -> Result<()> {
    let current_dir = module.path().to_path(cx.root);
    let upgrade = cx.config.upgrade(module.path());

    let mut command = Command::new("cargo");
    command.arg("upgrade");

    for exclude in upgrade.exclude.iter().chain(&opts.exclude) {
        command.args(["--exclude", exclude]);
    }

    if opts.workspace {
        command.arg("--workspace");
    }

    for arg in &opts.upgrade_args {
        command.arg(arg);
    }

    command.current_dir(&current_dir);

    if command.status()?.success() {
        good.insert(module);
    } else {
        tracing::warn!(?command, "Command failed");
        bad.insert(module);
    }

    Ok(())
}
