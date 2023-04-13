pub(crate) mod cargo;
pub(crate) mod ci;
pub(crate) mod readme;

use std::io::Write;

use anyhow::{anyhow, Context, Result};
use clap::Parser;

use crate::changes;
use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::model::ModuleParams;
use crate::model::UpdateParams;
use crate::urls::UrlError;
use crate::urls::Urls;
use crate::workspace::Package;
use crate::workspace::Workspace;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// Perform URL checks where we go out and try and fetch every references URL.
    #[arg(long)]
    url_checks: bool,
}

pub(crate) async fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut urls = Urls::default();

    for module in cx.modules() {
        tracing::info!("checking: {}", module.path());

        let workspace = module.workspace(cx)?;
        let primary_crate = workspace.primary_crate()?;
        let params = cx.module_params(primary_crate, module)?;

        check(cx, module, &workspace, primary_crate, params, &mut urls)
            .with_context(|| module.path().to_owned())?;
    }

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

/// Run a single module.
#[tracing::instrument(skip_all, fields(source = ?module.source(), path = module.path().as_str()))]
fn check(
    cx: &Ctxt<'_>,
    module: &Module,
    workspace: &Workspace,
    primary_crate: &Package,
    primary_crate_params: ModuleParams<'_>,
    urls: &mut Urls,
) -> Result<()> {
    let documentation = match &cx.config.documentation(module) {
        Some(documentation) => Some(documentation.render(&primary_crate_params)?),
        None => None,
    };

    let module_url = module.url().to_string();

    let update_params = UpdateParams {
        license: Some(cx.config.license(module)),
        readme: Some(readme::README_MD),
        repository: Some(&module_url),
        homepage: Some(&module_url),
        documentation: documentation.as_deref(),
        authors: cx.config.authors(module),
    };

    for package in workspace.packages() {
        if package.manifest.is_publish()? {
            cargo::work_cargo_toml(cx, package, &update_params)?;
        }
    }

    if cx.config.is_enabled(module.path(), "ci") {
        ci::build(cx, primary_crate, module, workspace)
            .with_context(|| anyhow!("ci change: {}", cx.config.job_name(module)))?;
    }

    if cx.config.is_enabled(module.path(), "readme") {
        readme::build(
            cx,
            module.path(),
            module,
            primary_crate,
            &primary_crate_params,
            urls,
            true,
            false,
        )?;

        for package in workspace.packages() {
            if !package.manifest.is_publish()? {
                continue;
            }

            let params = cx.module_params(package, module)?;

            readme::build(
                cx,
                &package.manifest_dir,
                module,
                package,
                &params,
                urls,
                package.manifest_dir != *module.path(),
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
