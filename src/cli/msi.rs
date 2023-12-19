use std::env::consts::{self, EXE_EXTENSION};

use anyhow::{bail, Result};
use clap::Parser;
use relative_path::RelativePathBuf;

use crate::ctxt::{self, Ctxt};
use crate::model::Repo;
use crate::release::Release;
use crate::repo_sets::RepoSet;
use crate::wix::Wix;
use crate::workspace;

use crate::release::ReleaseOpts;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    version: ReleaseOpts,
    /// Output directory to write to.
    #[clap(long, value_name = "output")]
    output: Option<RelativePathBuf>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let release = opts.version.make()?;

    let mut good = RepoSet::default();
    let mut bad = RepoSet::default();

    for repo in cx.repos() {
        if let Err(error) = msi(cx, repo, opts, &release) {
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
fn msi(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts, release: &Release) -> Result<(), anyhow::Error> {
    let root = cx.to_path(repo.path());

    let Some(workspace) = workspace::open(cx, repo)? else {
        bail!("Not a workspace");
    };

    let package = workspace.primary_package()?;
    let name = package.name()?;

    let binary_path = root
        .join("target")
        .join("release")
        .join(name)
        .with_extension(EXE_EXTENSION);

    let target = root.join("target").join("wix");
    let wsx_path = root.join("wix").join(format!("{name}.wxs"));

    if !wsx_path.is_file() {
        bail!("Missing: {}", wsx_path.display());
    }

    let output = match &opts.output {
        Some(output) => cx.to_path(repo.path().join(output)),
        None => target.clone(),
    };

    let base = format!(
        "{name}-{release}-{os}-{arch}",
        os = consts::OS,
        arch = consts::ARCH
    );

    let target_wixobj = target.join(format!("{base}.wixobj"));
    let installer_path = output.join(format!("{base}.msi"));

    let Some(binary_name) = binary_path.file_name().and_then(|name| name.to_str()) else {
        bail!("Missing or invalid file name: {}", binary_path.display());
    };

    let builder = Wix::find()?;
    builder.build(
        wsx_path,
        &target_wixobj,
        ctxt::empty_or_dot(repo.path().to_path(cx.root)),
        binary_name,
        &binary_path,
        release.msi_version()?,
    )?;
    builder.link(&target_wixobj, installer_path)?;
    Ok(())
}