use anyhow::{anyhow, Context, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::module_sets::ModuleSet;
use crate::process::Command;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    #[arg(long)]
    /// Store the outcome if this run into the sets `good` and `bad`, to be used
    /// later with `--set <id>` command.
    ///
    /// The `good` set will contain modules for which the command exited
    /// successfully, while the `bad` set for which they failed.
    store_sets: bool,
    /// Command to run.
    command: Vec<String>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let Some((command, args)) = opts.command.split_first() else {
        return Err(anyhow!("missing command"));
    };

    let mut good = ModuleSet::default();
    let mut bad = ModuleSet::default();

    for module in cx.modules() {
        r#for(cx, module, command, args, &mut good, &mut bad)
            .with_context(|| module.path().to_owned())?;
    }

    cx.sets.save("good", good, opts.store_sets);
    cx.sets.save("bad", bad, opts.store_sets);
    Ok(())
}

#[tracing::instrument(name = "for", skip_all, fields(path = module.path().as_str()))]
fn r#for(
    cx: &Ctxt<'_>,
    module: &Module,
    command: &str,
    args: &[String],
    good: &mut ModuleSet,
    bad: &mut ModuleSet,
) -> Result<()> {
    let current_dir = module.path().to_path(cx.root);

    let mut command = Command::new(command);
    command.args(args);
    command.current_dir(&current_dir);

    tracing::info!("{}", command.display());

    if command.status()?.success() {
        good.insert(module);
    } else {
        tracing::warn!(?command, "command failed");
        bad.insert(module);
    }

    Ok(())
}
