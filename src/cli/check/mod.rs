pub(crate) mod cargo;
pub(crate) mod ci;
pub(crate) mod readme;

use std::collections::{BTreeSet, HashMap};
use std::io::Write;

use anyhow::{Context, Result};
use clap::Parser;
use semver::Version;

use crate::changes;
use crate::ctxt::Ctxt;
use crate::model::{Repo, UpdateParams};
use crate::urls::{UrlError, Urls};
use crate::version_groups::{self, ResolvedGroups};
use crate::workspace::Crates;

use crate::cli::WithRepos;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Perform URL checks where we go out and try and fetch every references
    /// URL.
    #[arg(long)]
    url_checks: bool,
}

pub(crate) async fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    let mut urls = Urls::default();

    with_repos.run("check", format_args!("check: {opts:?}"), |cx, repo| {
        check(cx, repo, &mut urls)
    })?;

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

#[tracing::instrument(skip_all)]
fn check(cx: &Ctxt<'_>, repo: &Repo, urls: &mut Urls) -> Result<()> {
    let crates = repo.workspace(cx)?;
    let primary_manifest = crates.primary_package()?;
    let primary_package = primary_manifest.ensure_package()?;
    let primary_params = cx.repo_params(primary_package, repo)?;

    let documentation = match &cx.config.documentation(repo) {
        Some(documentation) => Some(documentation.render(&primary_params)?),
        None => None,
    };

    let repo_url = repo.url().to_string();

    let update_params = UpdateParams {
        license: Some(cx.config.license(repo)),
        readme: Some(readme::README_MD),
        repository: Some(&repo_url),
        documentation: documentation.as_deref(),
        authors: cx.config.authors(repo),
    };

    let group_versions = version_group_mismatches(cx, repo, crates)?;

    for manifest in crates.packages() {
        let rust_version = primary_package.rust_version();

        let group_version = match manifest.as_package() {
            Some(package) => group_versions.get(package.name()?),
            None => None,
        };

        cargo::work_cargo_toml(
            cx,
            crates,
            manifest,
            &update_params,
            rust_version,
            group_version,
        )?;
    }

    if cx.config.is_enabled(repo, "ci") {
        ci::build(cx, primary_manifest, repo, crates).context("ci change")?;
    }

    if cx.config.is_enabled(repo, "readme") {
        readme::build(
            cx,
            repo.path(),
            repo,
            primary_manifest,
            &primary_params,
            urls,
            true,
            false,
        )?;

        for manifest in crates.packages() {
            let Some(package) = manifest.as_package() else {
                continue;
            };

            if !package.is_publish() {
                continue;
            }

            let params = cx.repo_params(package, repo)?;

            readme::build(
                cx,
                manifest.dir(),
                repo,
                manifest,
                &params,
                urls,
                manifest.dir() != repo.path(),
                true,
            )?;
        }
    }

    Ok(())
}

/// Find crates whose version disagree with the rest of their
/// `[[version_group]]`, and the version they are expected to have.
fn version_group_mismatches(
    cx: &Ctxt<'_>,
    repo: &Repo,
    crates: &Crates,
) -> Result<HashMap<String, Version>> {
    let groups = cx.config.version_groups(repo);

    if groups.is_empty() {
        return Ok(HashMap::new());
    }

    let workspace_version = version_groups::workspace_version(crates)?;
    let mut known = BTreeSet::new();
    let mut versions = Vec::new();

    for manifest in crates.packages() {
        let Some(package) = manifest.as_package() else {
            continue;
        };

        let name = package.name()?;
        known.insert(name);

        if !package.is_publish() {
            continue;
        }

        let version = version_groups::package_version(package, workspace_version.as_ref())?;
        versions.push((name, version.version().cloned()));
    }

    let groups = ResolvedGroups::resolve(groups, &known)?;

    Ok(version_groups::mismatches(
        &groups,
        versions.iter().map(|(name, v)| (*name, v.as_ref())),
    ))
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
