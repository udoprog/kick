pub(crate) mod cargo;
pub(crate) mod ci;
pub(crate) mod readme;

use std::io::Write;

use anyhow::{anyhow, Context, Result};
use clap::Parser;

use crate::changes;
use crate::ctxt::Ctxt;
use crate::model::{Repo, UpdateParams};
use crate::urls::{UrlError, Urls};

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Perform URL checks where we go out and try and fetch every references
    /// URL.
    #[arg(long)]
    url_checks: bool,
}

pub(crate) async fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut urls = Urls::default();

    with_repos!(cx, "Check", format_args!("check: {opts:?}"), |cx, repo| {
        check(cx, repo, &mut urls)
    });

    let o = std::io::stdout();
    let mut o = o.lock();

    for (url, test) in urls.bad_urls() {
        let path = &test.path;
        let (line, column, string) =
            changes::temporary_line_fix(&test.file, test.range.start, test.line_offset)?;

        if let Some(error) = &test.error {
            writeln!(o, "{path}:{line}:{column}: bad url: `{url}`: {error}")?;
        } else {
            writeln!(o, "{path}:{line}:{column}: bad url: `{url}`")?;
        }

        writeln!(o, "{string}")?;
    }

    if opts.url_checks {
        url_checks(&mut o, urls).await?;
    }

    Ok(())
}

/// Run checks for a single repo.
#[tracing::instrument(skip_all, fields(source = ?repo.source(), path = repo.path().as_str()))]
fn check(cx: &Ctxt<'_>, repo: &Repo, urls: &mut Urls) -> Result<()> {
    let crates = repo.workspace(cx)?;
    let primary_crate = crates.primary_package()?;
    let primary_crate_params = cx.repo_params(&primary_crate, repo)?;

    let documentation = match &cx.config.documentation(repo) {
        Some(documentation) => Some(documentation.render(&primary_crate_params)?),
        None => None,
    };

    let repo_url = repo.url().to_string();

    let update_params = UpdateParams {
        license: Some(cx.config.license(repo)),
        readme: Some(readme::README_MD),
        repository: Some(&repo_url),
        homepage: Some(&repo_url),
        documentation: documentation.as_deref(),
        authors: cx.config.authors(repo),
    };

    for package in crates.packages() {
        if package.is_publish() {
            cargo::work_cargo_toml(cx, &crates, &package, &update_params)?;
        }
    }

    if cx.config.is_enabled(repo.path(), "ci") {
        ci::build(cx, &primary_crate, repo, &crates).with_context(|| anyhow!("ci change"))?;
    }

    if cx.config.is_enabled(repo.path(), "readme") {
        readme::build(
            cx,
            repo.path(),
            repo,
            primary_crate.manifest(),
            &primary_crate_params,
            urls,
            true,
            false,
        )?;

        for package in crates.packages() {
            if !package.is_publish() {
                continue;
            }

            let params = cx.repo_params(&package, repo)?;

            readme::build(
                cx,
                package.manifest().dir(),
                repo,
                package.manifest(),
                &params,
                urls,
                package.manifest().dir() != repo.path(),
                true,
            )?;
        }
    }

    Ok(())
}

/// Perform url checks.
async fn url_checks<O>(o: &mut O, urls: Urls) -> Result<()>
where
    O: Write,
{
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);

    let total = urls.check_urls();
    let checks = urls.check_urls_task(3, tx);
    tokio::pin!(checks);
    let mut count = 1;
    let mut completed = false;

    loop {
        tokio::select! {
            result = checks.as_mut(), if !completed => {
                result?;
                completed = true;
            }
            result = rx.recv() => {
                let result = match result {
                    Some(result) => result,
                    None => break,
                };

                match result {
                    Ok(_) => {}
                    Err(UrlError { url, status, tests }) => {
                        writeln!(o, "{count:>3}/{total} {url}: {status}")?;

                        for test in tests {
                            let path = &test.path;
                            let (line, column, string) = crate::changes::temporary_line_fix(&test.file, test.range.start, test.line_offset)?;
                            writeln!(o, "  {path}:{line}:{column}: {string}")?;
                        }
                    }
                }

                count += 1;
            }
        }
    }

    Ok(())
}
