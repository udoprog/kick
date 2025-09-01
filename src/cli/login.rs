use std::fs;
use std::io::{BufRead, Write};

use anyhow::{Context, Result};
use clap::Parser;

use crate::GITHUB_TOKEN;
use crate::ctxt::Ctxt;
use crate::env::GithubTokenSource;

const URL: &str = "https://github.com/settings/personal-access-tokens";

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {}

pub(crate) fn entry(cx: &mut Ctxt<'_>, _: &Opts) -> Result<()> {
    let out = std::io::stdout();
    let input = std::io::stdin();

    let mut out = out.lock();
    let mut input = input.lock();

    if cx.github_auth().is_some() {
        writeln!(out, "Github authentication is already available")?;

        if cx.git_credentials.is_some() {
            writeln!(out, "Fetched authentication from git credentials manager")?;
        }

        for token in &cx.env.github_tokens {
            match &token.source {
                GithubTokenSource::Environment => {
                    writeln!(out, "Found github token from environment GITHUB_TOKEN")?;
                }
                GithubTokenSource::CommandLine => {
                    writeln!(out, "Found github token from command line argument")?;
                }
                GithubTokenSource::Path(path) => {
                    writeln!(out, "Found github token from path: {}", path.display())?;
                }
            }
        }

        return Ok(());
    }

    let config = cx.paths.config.context("missing config path")?;

    writeln!(
        out,
        "Navigate to Github ({URL}) and generate a new token, paste it below:"
    )?;

    let mut line = String::new();
    input.read_line(&mut line)?;

    let line = line.trim();

    fs::create_dir_all(config).with_context(|| config.display().to_string())?;

    let github_token = config.join(GITHUB_TOKEN);

    (|| {
        let mut f = fs::File::create(&github_token)?;
        let mut p = f.metadata()?.permissions();
        crate::fs::set_secure(&mut p);
        f.set_permissions(p)?;
        f.write_all(line.as_bytes())?;
        Ok::<_, anyhow::Error>(())
    })()
    .with_context(|| github_token.display().to_string())?;

    writeln!(out, "Wrote {}", github_token.display())?;
    Ok(())
}
