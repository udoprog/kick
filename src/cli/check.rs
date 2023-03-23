use std::io::Write;

use anyhow::{Context, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::urls::UrlError;
use crate::urls::Urls;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// Perform URL checks where we go out and try and fetch every references URL.
    #[arg(long)]
    url_checks: bool,
}

/// Entrypoint to run action.
#[tracing::instrument(skip(cx, opts))]
pub(crate) async fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut urls = Urls::default();

    for module in cx.modules() {
        tracing::info!("checking: {}", module.path());

        let workspace = module.workspace(cx)?;
        let primary_crate = workspace.primary_crate()?;
        let params = cx.module_params(primary_crate, module)?;

        crate::validation::build(cx, module, &workspace, primary_crate, params, &mut urls)
            .with_context(|| module.path().to_owned())?;
    }

    let o = std::io::stdout();
    let mut o = o.lock();

    for (url, test) in urls.bad_urls() {
        let path = &test.path;
        let (line, column, string) =
            crate::validation::temporary_line_fix(&test.file, test.range.start, test.line_offset)?;

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
                            let (line, column, string) = crate::validation::temporary_line_fix(&test.file, test.range.start, test.line_offset)?;
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
