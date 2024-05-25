use std::ffi::OsString;

use anyhow::{bail, ensure, Result};
use clap::Parser;

use crate::config::Os;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::Command;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Command to run.
    #[arg(value_name = "command")]
    command: OsString,
    /// If the specified operating system is different for the repo, execute the
    /// command using a compatibility layer which is appropriate for a supported
    /// operating system.
    ///
    /// * On Windows, if a project is Linux-specific WSL will be used.
    #[arg(long)]
    run_on_os: bool,
    /// Arguments to pass to the command to run.
    #[arg(
        trailing_var_arg = true,
        allow_hyphen_values = true,
        value_name = "args"
    )]
    args: Vec<OsString>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    with_repos!(
        cx,
        "run commands",
        format_args!("for: {opts:?}"),
        |cx, repo| { r#for(cx, repo, opts) }
    );

    Ok(())
}

#[tracing::instrument(name = "for", skip_all)]
fn r#for(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let path = cx.to_path(repo.path());

    let mut command = Command::new(&opts.command);
    command.args(&opts.args);
    command.current_dir(&path);

    let mut runner = command;
    let mut runner_name = None;

    if opts.run_on_os {
        let os = cx.config.os(repo);

        'ok: {
            if os.is_empty() || os.contains(&cx.os) {
                break 'ok;
            }

            if cx.os == Os::Windows && os.contains(&Os::Linux) {
                if let Some(wsl) = cx.system.wsl.first() {
                    let mut command = wsl.shell(&path);
                    command.arg(&opts.command);
                    command.args(&opts.args);
                    runner = command;
                    runner_name = Some("WSL");
                    break 'ok;
                }
            }

            bail!(
                "No supported runner for {os:?} on current system {:?}",
                cx.os
            );
        }
    }

    if let Some(runner_name) = runner_name {
        println!("{} ({runner_name}):", path.display());
    } else {
        println!("{}:", path.display());
    }

    let status = runner.status()?;
    ensure!(status.success(), status);
    Ok(())
}
