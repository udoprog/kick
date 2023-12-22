use std::ffi::{OsStr, OsString};

use anyhow::{ensure, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::Command;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Command to run.
    #[arg(value_name = "command")]
    command: OsString,
    /// Arguments to pass to the command to run.
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "args"
    )]
    args: Vec<OsString>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    with_repos!(cx, "For", format_args!("for: {opts:?}"), |cx, repo| {
        r#for(cx, repo, &opts.command, &opts.args)
    });

    Ok(())
}

#[tracing::instrument(name = "for", skip_all)]
fn r#for(cx: &Ctxt<'_>, repo: &Repo, command: &OsStr, args: &[OsString]) -> Result<()> {
    let path = cx.to_path(repo.path());

    let mut command = Command::new(command);
    command.args(args);
    command.current_dir(&path);

    println!("{}:", path.display());
    let status = command.status()?;
    ensure!(status.success(), status);
    Ok(())
}
