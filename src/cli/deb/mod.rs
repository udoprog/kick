use std::fs::{self, File};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use relative_path::RelativePath;
use relative_path::RelativePathBuf;

use crate::config::deb_depends;
use crate::config::VersionRequirement;
use crate::ctxt::Ctxt;
use crate::deb;
use crate::model::Repo;
use crate::packaging::InstallFile;
use crate::release::Version;

use crate::release::ReleaseOpts;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    release: ReleaseOpts,
    /// Output directory to write to.
    #[clap(long, value_name = "output")]
    output: Option<RelativePathBuf>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let release = opts.release.version(cx.env)?;

    with_repos!(
        cx,
        "Build deb",
        format_args!("deb: {:?}", opts),
        |cx, repo| { deb(cx, repo, opts, &release) }
    );

    Ok(())
}

#[tracing::instrument(skip_all)]
fn deb(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts, release: &Version<'_>) -> Result<()> {
    let workspace = repo.workspace(cx)?;

    let package = workspace.primary_package()?;
    let name = package.name()?;
    let arch = deb::Architecture::Amd64;

    let mut builder = deb::Builder::new(name, arch);

    if let Some(description) = package.description() {
        builder.description(description);
    }

    let debian_version = release.debian_version()?;
    builder.version(&debian_version);

    let output = match &opts.output {
        Some(output) => cx.to_path(repo.path().join(output)),
        None => cx.to_path(repo.path().join("target/deb")),
    };

    let output_path = output.join(format!("{name}-{debian_version}-{arch}.deb"));
    tracing::info!("Writing: {}", output_path.display());

    for install_file in crate::packaging::install_files(cx, repo)? {
        match install_file {
            InstallFile::Binary(name, source) => {
                let meta = source.metadata().with_context(|| {
                    anyhow!("Reading binary file metadata: {}", source.display())
                })?;

                let contents = fs::read(&source).with_context(|| {
                    anyhow!("Reading binary file contents: {}", source.display())
                })?;

                let modified = meta.modified().with_context(|| {
                    anyhow!("Reading binary file modified time: {}", source.display())
                })?;

                builder
                    .insert_file(RelativePath::new("usr/bin").join(&name))
                    .contents(contents)
                    .mode(0o755)
                    .mtime(modified)?;
            }
            InstallFile::File(file, source, dest) => {
                let meta = source.metadata().with_context(|| {
                    anyhow!("Reading source file metadata: {}", source.display())
                })?;

                let contents = fs::read(&source).with_context(|| {
                    anyhow!("Reading source file contents: {}", source.display())
                })?;

                let modified = meta.modified().with_context(|| {
                    anyhow!("Reading source file modified time: {}", source.display())
                })?;

                let builder = builder
                    .insert_file(dest)
                    .contents(contents)
                    .mtime(modified)?;

                if let Some(mode) = file.mode {
                    builder.mode(mode as u32);
                }
            }
        }
    }

    for dep in cx.config.get_all(repo, deb_depends) {
        let builder = builder.insert_depends(&dep.package);

        match &dep.version {
            VersionRequirement::Any => {}
            VersionRequirement::Constraint(constraint, version) => {
                builder.version(format_args!("{constraint} {version}"));
            }
        }
    }

    if !output.is_dir() {
        fs::create_dir_all(output)?;
    }

    let f = File::create(&output_path)
        .with_context(|| anyhow!("Creating {}", output_path.display()))?;
    builder.write_to(f).context("Writing .deb")?;
    Ok(())
}
