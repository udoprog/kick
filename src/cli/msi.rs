use std::env::consts::{self, EXE_EXTENSION};

use anyhow::{Result, bail};
use clap::Parser;
use relative_path::RelativePathBuf;

use crate::cli::WithRepos;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::release::ReleaseOpts;
use crate::wix::Wix;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    release: ReleaseOpts,
    /// Output directory to write to.
    #[clap(long, value_name = "output")]
    output: Option<RelativePathBuf>,
}

pub(crate) async fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    with_repos
        .run_async(
            "build .msi",
            format_args!("msi: {opts:?}"),
            async |cx, repo| msi(cx, repo, opts).await,
            |_| Ok(()),
        )
        .await?;

    Ok(())
}

#[tracing::instrument(skip_all)]
async fn msi(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let release = opts.release.version(cx, repo)?;
    let root = cx.to_path(repo.path());
    let workspace = repo.workspace(cx)?;

    let package = workspace.primary_package()?.ensure_package()?;
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
    builder
        .build(
            wsx_path,
            &target_wixobj,
            cx.to_path(repo.path()),
            binary_name,
            &binary_path,
            release.msi_version()?,
        )
        .await?;
    builder.link(&target_wixobj, installer_path).await?;
    Ok(())
}
