use std::collections::BTreeMap;

use anyhow::{Result, bail};
use clap::Parser;
use termcolor::{ColorChoice, StandardStream};

use crate::commands::{ActionConfig, BatchOptions, Session};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::rstr::{RStr, RString};

use crate::cli::WithRepos;

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

pub(crate) async fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    with_repos
        .run_async(
            "run action",
            format_args!("for: {opts:?}"),
            async |cx, repo| action(cx, repo, opts).await,
            |_| Ok(()),
        )
        .await?;

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn action(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let mut o = StandardStream::stdout(ColorChoice::Auto);

    let id = RStr::new(&opts.id);

    let c = opts.batch_opts.build(cx, repo)?;

    let mut inputs = BTreeMap::new();

    for input in &opts.input {
        let Some((key, value)) = input.split_once('=') else {
            bail!("Inputs must be in the form of `<key>=<value>`")
        };

        inputs.insert(key.to_string(), RString::from(value));
    }

    let action = ActionConfig::new(&cx.os, id)
        .with_inputs(inputs)
        .repo_from_name();

    let batch = action.new_use_batch(&c, id)?;

    let mut session = Session::new(&c);
    batch.commit(&mut o, &c, &mut session).await?;
    Ok(())
}
