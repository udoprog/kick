use anyhow::{anyhow, Context, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::process::Command;
use crate::sets::Set;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    #[arg(long)]
    /// Store the outcome if this run into the sets `good` and `bad`, to
    /// be used later with `--set <id>` command.
    store_sets: bool,
    /// Command to run.
    command: Vec<String>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let Some((command, args)) = opts.command.split_first() else {
        return Err(anyhow!("missing command"));
    };

    let mut good = opts.store_sets.then(Set::default);
    let mut bad = opts.store_sets.then(Set::default);

    for module in cx.modules() {
        r#for(cx, module, command, args, good.as_mut(), bad.as_mut())
            .with_context(|| module.path().to_owned())?;
    }

    if let Some(set) = good {
        cx.sets.save("good", set);
    }

    if let Some(set) = bad {
        cx.sets.save("bad", set);
    }

    Ok(())
}

#[tracing::instrument(name = "for", skip_all, fields(path = module.path().as_str()))]
fn r#for(
    cx: &Ctxt<'_>,
    module: &Module,
    command: &str,
    args: &[String],
    good: Option<&mut Set>,
    bad: Option<&mut Set>,
) -> Result<()> {
    let current_dir = crate::utils::to_path(module.path(), cx.root);

    let mut command = Command::new(command);
    command.args(args);
    command.current_dir(&current_dir);

    tracing::info!("{}", command.display());

    if !command.status()?.success() {
        tracing::warn!(?command, "command failed");

        if let Some(set) = bad {
            set.insert(module);
        }
    } else if let Some(set) = good {
        set.insert(module);
    }

    Ok(())
}
