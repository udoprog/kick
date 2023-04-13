use anyhow::{anyhow, Context, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::process::Command;
use crate::sets::Set;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    #[arg(long)]
    /// Save successfully executed commands to the 'success' set.
    save_success: bool,
    #[arg(long)]
    /// Save successfully executed commands to the 'failed' set.
    save_failed: bool,
    /// Command to run.
    command: Vec<String>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let Some((command, args)) = opts.command.split_first() else {
        return Err(anyhow!("missing command"));
    };

    let mut success = opts.save_success.then(Set::default);
    let mut failed = opts.save_failed.then(Set::default);

    for module in cx.modules() {
        r#for(cx, module, command, args, success.as_mut(), failed.as_mut())
            .with_context(|| module.path().to_owned())?;
    }

    if let Some(set) = success {
        cx.sets.save("success", set);
    }

    if let Some(set) = failed {
        cx.sets.save("failed", set);
    }

    Ok(())
}

#[tracing::instrument(name = "for", skip_all, fields(path = module.path().as_str()))]
fn r#for(
    cx: &Ctxt<'_>,
    module: &Module,
    command: &str,
    args: &[String],
    success: Option<&mut Set>,
    failed: Option<&mut Set>,
) -> Result<()> {
    let current_dir = crate::utils::to_path(module.path(), cx.root);

    let mut command = Command::new(command);
    command.args(args);
    command.current_dir(&current_dir);

    tracing::info!("{}", command.display());

    if !command.status()?.success() {
        tracing::warn!(?command, "command failed");

        if let Some(set) = failed {
            set.add(module);
        }
    } else if let Some(set) = success {
        set.add(module);
    }

    Ok(())
}
