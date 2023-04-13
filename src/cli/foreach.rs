use anyhow::{anyhow, Context, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::process::Command;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// Command to run.
    command: Vec<String>,
}

pub(crate) fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    let Some((command, args)) = opts.command.split_first() else {
        return Err(anyhow!("missing command"));
    };

    for module in cx.modules() {
        foreach(cx, module, command, args).with_context(|| module.path().to_owned())?;
    }

    Ok(())
}

#[tracing::instrument(skip_all, fields(path = module.path().as_str()))]
fn foreach(cx: &Ctxt<'_>, module: &Module, command: &str, args: &[String]) -> Result<()> {
    let current_dir = crate::utils::to_path(module.path(), cx.root);

    let mut command = Command::new(command);
    command.args(args);
    command.current_dir(&current_dir);

    if !command.status()?.success() {
        tracing::warn!(?command, "command failed");
    }

    Ok(())
}
