use std::fs;
use std::path::PathBuf;

use anyhow::{bail, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::release::Release;
use crate::repo_sets::RepoSet;
use crate::wix;
use crate::workspace;

use crate::release::ReleaseOpts;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    version: ReleaseOpts,
    /// Output directory to write to.
    #[clap(long, value_name = "output")]
    output: Option<PathBuf>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let release = opts.version.make()?;
    let file_version = release.msi_version()?;

    let mut good = RepoSet::default();
    let mut bad = RepoSet::default();

    for repo in cx.repos() {
        if let Err(error) = msi(cx, repo, opts, &release, &file_version) {
            tracing::error!("Failed to build msi");

            for cause in error.chain() {
                tracing::error!("Caused by: {cause}");
            }

            bad.insert(repo);
        } else {
            good.insert(repo);
        }
    }

    let hint = format!("for: {:?}", opts);
    cx.sets.save("good", good, &hint);
    cx.sets.save("bad", bad, &hint);
    Ok(())
}

#[tracing::instrument(skip_all, fields(source = ?repo.source(), path = repo.path().as_str()))]
fn msi(
    cx: &Ctxt<'_>,
    repo: &Repo,
    opts: &Opts,
    release: &Release,
    file_version: &str,
) -> Result<(), anyhow::Error> {
    let root = cx.to_path(repo.path());

    let Some(workspace) = workspace::open(cx, repo)? else {
        bail!("Not a workspace");
    };

    let package = workspace.primary_package()?;
    let name = package.name()?;
    let wix_dir = root.join("wix");
    let wsx_file = wix_dir.join("main.wxs");

    if !wsx_file.is_file() {
        bail!("Missing: {}", wsx_file.display());
    }

    let output = match &opts.output {
        Some(output) => output,
        None => &wix_dir,
    };

    if !output.is_dir() {
        fs::create_dir_all(output)?;
    }

    let builder = wix::Builder::new(output, name, release)?;
    builder.build(wsx_file, file_version)?;
    builder.link()?;
    Ok(())
}
