use anyhow::{anyhow, bail, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::Command;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Command to run.
    #[arg(value_name = "command")]
    command: Vec<String>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let Some((command, args)) = opts.command.split_first() else {
        return Err(anyhow!("missing command"));
    };

    with_repos!(cx, "For", format_args!("for: {opts:?}"), |cx, repo| {
        r#for(cx, repo, command, args)
    });

    Ok(())
}

#[tracing::instrument(name = "for", skip_all, fields(path = repo.path().as_str()))]
fn r#for(cx: &Ctxt<'_>, repo: &Repo, command: &str, args: &[String]) -> Result<()> {
    let mut command = Command::new(command);
    command.args(args);
    command.current_dir(cx.to_path(repo.path()));

    tracing::info!("{}", command.display());

    if !command.status()?.success() {
        bail!("command failed");
    }

    Ok(())
}
