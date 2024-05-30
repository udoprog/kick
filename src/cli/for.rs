use std::ffi::OsString;
use std::path::Path;

use anyhow::{bail, ensure, Result};
use clap::Parser;

use crate::config::Os;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::process::Command;
use crate::wsl::Wsl;

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
    first_os: bool,
    /// Executes the command over all supported operating systems.
    #[arg(long)]
    each_os: bool,
    /// Environment variables to pass to the command to run. Only specifying
    /// `<key>` means that the specified environment variable should be passed
    /// through.
    ///
    /// For WSL, this constructs the WSLENV environment variable, which dictates
    /// what environments are passed in.
    #[arg(long, short = 'E', value_name = "<key>[=<value>]")]
    env: Vec<String>,
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

    let mut runners = Vec::new();

    if opts.first_os || opts.each_os {
        if opts.first_os && opts.each_os {
            bail!("Cannot specify both --first-os and --each-os");
        }

        let limit = if opts.first_os { 1 } else { usize::MAX };

        for os in cx.config.os(repo).into_iter().take(limit) {
            if cx.os == *os {
                runners.push(setup_same(&path, opts));
                continue;
            }

            if cx.os == Os::Windows && *os == Os::Linux {
                if let Some(wsl) = cx.system.wsl.first() {
                    runners.push(setup_wsl(&path, wsl, opts));
                    continue;
                }
            }

            bail!(
                "No supported runner for {os:?} on current system {:?}",
                cx.os
            );
        }
    }

    if runners.is_empty() {
        runners.push(setup_same(&path, opts));
    }

    for mut runner in runners {
        for e in &opts.env {
            if let Some((key, value)) = e.split_once('=') {
                runner.command.env(key, value);
            }
        }

        if let Some((key, value)) = runner.extra_env {
            runner.command.env(key, value);
        }

        if let Some(name) = runner.name {
            println!("{} ({name}):", path.display());
        } else {
            println!("{}:", path.display());
        }

        let status = runner.command.status()?;
        ensure!(status.success(), status);
    }

    Ok(())
}

struct Runner {
    command: Command,
    name: Option<&'static str>,
    extra_env: Option<(&'static str, String)>,
}

impl Runner {
    fn new(command: Command) -> Self {
        Self {
            command,
            name: None,
            extra_env: None,
        }
    }
}

fn setup_same(path: &Path, opts: &Opts) -> Runner {
    let mut command = Command::new(&opts.command);
    command.args(&opts.args);
    command.current_dir(path);
    Runner::new(command)
}

fn setup_wsl(path: &Path, wsl: &Wsl, opts: &Opts) -> Runner {
    let mut command = wsl.shell(path);
    command.arg(&opts.command);
    command.args(&opts.args);

    let mut wslenv = String::new();

    for e in &opts.env {
        if !wslenv.is_empty() {
            wslenv.push(':');
        }

        if let Some((key, _)) = e.split_once('=') {
            wslenv.push_str(key);
        } else {
            wslenv.push_str(e);
        }
    }

    let mut runner = Runner::new(command);
    runner.name = Some("WSL");
    runner.extra_env = Some(("WSLENV", wslenv));
    runner
}
