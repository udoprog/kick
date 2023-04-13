use anyhow::{Context, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::process::Command;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// Pass `--workspace` to `cargo upgrade`.
    #[arg(long)]
    workspace: bool,
    /// Pass `--exclude <package>` to `cargo upgrade`.
    #[arg(long)]
    exclude: Vec<String>,
    /// Extra upgrade arguments.
    upgrade_args: Vec<String>,
}

pub(crate) fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    for module in cx.modules() {
        upgrade(cx, opts, module).with_context(|| module.path().to_owned())?;
    }

    Ok(())
}

#[tracing::instrument(skip_all, fields(source = ?module.source(), path = module.path().as_str()))]
fn upgrade(cx: &Ctxt<'_>, opts: &Opts, module: &Module) -> Result<()> {
    let current_dir = crate::utils::to_path(module.path(), cx.root);
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

    if !command.status()?.success() {
        tracing::warn!(?command, "Command failed");
    }

    Ok(())
}
