use anyhow::Result;
use clap::Parser;
use termcolor::{ColorChoice, StandardStream};

use crate::cli::WithRepos;
use crate::cli::deploy::{self, Common};
use crate::config::Section;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[command(flatten)]
    common: Common,
}

pub(crate) fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    let mut o = StandardStream::stdout(ColorChoice::Auto);

    // NB: An install is a deployment to the machine kick is running on, which
    // is configured through the `[install]` section instead.
    let opts = deploy::Opts::local(opts.common.clone());

    with_repos.run("install", format_args!("install: {opts:?}"), |cx, repo| {
        deploy::deploy(&mut o, cx, &opts, repo, Section::Install)
    })?;

    Ok(())
}
