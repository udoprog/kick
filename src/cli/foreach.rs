use std::process::{Command, Stdio};

use anyhow::{anyhow, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::utils::CommandRepr;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// Only run for git repos which have cached changes.
    #[arg(long)]
    cached: bool,
    /// Filter by the specified modules.
    #[arg(long = "module", short = 'm', name = "module")]
    modules: Vec<String>,
    /// Command to run.
    command: Vec<String>,
}

pub(crate) fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    let Some((command, args)) = opts.command.split_first() else {
        return Err(anyhow!("missing command"));
    };

    for module in &cx.modules {
        if crate::should_skip(&opts.modules, module) {
            continue;
        }

        let current_dir = module.path.to_path(cx.root);

        if opts.cached {
            let status = Command::new("git")
                .args(["diff", "--cached", "--exit-code"])
                .stdout(Stdio::null())
                .current_dir(&current_dir)
                .status()?;

            if status.success() {
                continue;
            }
        }

        tracing::info!(
            path = module.path.as_str(),
            "{}",
            CommandRepr::new(&opts.command)
        );
        let status = Command::new(command)
            .args(args)
            .current_dir(&current_dir)
            .status()?;
        tracing::info!(?status);
    }

    Ok(())
}
