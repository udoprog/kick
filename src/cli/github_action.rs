use std::collections::BTreeMap;

use anyhow::{bail, Result};
use clap::Parser;
use termcolor::{ColorChoice, StandardStream};

use crate::commands::{ActionConfig, BatchOptions, Session};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::rstr::{RStr, RString};

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[command(flatten)]
    batch_opts: BatchOptions,
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
    let id = RStr::new(&opts.id);

    let c = opts.batch_opts.build(cx, repo)?;

    let mut inputs = BTreeMap::new();

    for input in &opts.input {
        let Some((key, value)) = input.split_once('=') else {
            bail!("Inputs must be in the form of `<key>=<value>`")
        };

        inputs.insert(key.to_string(), RString::from(value));
    }

    let action = ActionConfig::new(&cx.current_os, id).with_inputs(inputs);

    let batch = action.new_use_batch(&c, id)?;

    let mut session = Session::new(&c);
    batch.commit(o, &c, &mut session)?;
    Ok(())
}
