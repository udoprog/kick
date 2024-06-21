use std::collections::BTreeMap;

use anyhow::{bail, Result};
use clap::Parser;
use termcolor::{ColorChoice, StandardStream};

use crate::commands::{ActionConfig, Batch, BatchConfig, BatchOptions, Prepare};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::rstr::{RStr, RString};
use crate::shell::Shell;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[command(flatten)]
    batch_opts: BatchOptions,
    /// The default shell to use when printing command invocations.
    ///
    /// By default this is `bash` for unix-like environments and `powershell`
    /// for windows.
    #[arg(long, value_name = "shell")]
    shell: Option<Shell>,
    /// The workflow to run.
    #[arg(value_name = "id")]
    id: String,
    /// Inputs to the action.
    #[arg(value_name = "key=value")]
    input: Vec<String>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut o = StandardStream::stdout(ColorChoice::Auto);

    with_repos!(
        cx,
        "run action",
        format_args!("for: {opts:?}"),
        |cx, repo| { action(&mut o, cx, repo, opts) }
    );

    Ok(())
}

#[tracing::instrument(skip_all)]
fn action(o: &mut StandardStream, cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let repo_path = cx.to_path(repo.path());

    let mut prepare = Prepare::default();
    prepare.actions_mut().insert_action(&opts.id)?;

    let id = RStr::new(&opts.id);

    let default_shell = opts.shell.unwrap_or_else(|| cx.os.shell());

    let mut inputs = BTreeMap::new();

    for input in &opts.input {
        let Some((key, value)) = input.split_once('=') else {
            bail!("Inputs must be in the form of `<key>=<value>`")
        };

        inputs.insert(key.to_string(), RString::from(value));
    }

    let mut c = BatchConfig::new(cx, &repo_path, default_shell);
    c.add_opts(&opts.batch_opts)?;

    let action = ActionConfig::default().with_inputs(inputs);

    let batch = Batch::with_use(cx, id, &action)?;
    batch.prepare(&c, &mut prepare)?;

    prepare.prepare(o, &c)?;
    batch.commit(o, &c, prepare.runners())?;
    Ok(())
}
