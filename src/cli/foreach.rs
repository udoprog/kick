use std::process::Command;

use anyhow::{anyhow, Context, Error, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::utils::CommandRepr;

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
        foreach(cx, opts, module, command, args).with_context(|| module.path().to_owned())?;
    }

    Ok(())
}

#[tracing::instrument(skip(cx, opts, module, command, args), fields(path = module.path().as_str()))]
fn foreach(
    cx: &Ctxt<'_>,
    opts: &Opts,
    module: &Module,
    command: &str,
    args: &[String],
) -> Result<()> {
    let current_dir = crate::utils::to_path(module.path(), cx.root);
    tracing::info!("{}", CommandRepr::new(&opts.command));

    let status = Command::new(command)
        .args(args)
        .current_dir(&current_dir)
        .status()
        .with_context(|| Error::msg(CommandRepr::new(&opts.command).to_string()))?;

    tracing::trace!(?status);
    Ok(())
}
