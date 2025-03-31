use std::fs;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use relative_path::RelativePath;

use crate::cli::WithRepos;
use crate::config::deb_depends;
use crate::config::VersionRequirement;
use crate::ctxt::Ctxt;
use crate::deb;
use crate::model::Repo;
use crate::packaging::InstallFile;
use crate::release::ReleaseOpts;

use super::output::OutputOpts;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    release: ReleaseOpts,
    #[clap(flatten)]
    output: OutputOpts,
}

pub(crate) fn entry<'repo>(with_repos: impl WithRepos<'repo>, opts: &Opts) -> Result<()> {
    with_repos.run("build .deb", format_args!("deb: {:?}", opts), |cx, repo| {
        deb(cx, repo, opts)
    })?;

    Ok(())
}

#[tracing::instrument(skip_all)]
fn deb(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let release = opts.release.version(cx, repo)?;
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

    let output = opts.output.make_directory(cx, repo, "deb");
    let mut f = output.create_file(format_args!("{name}-{debian_version}-{arch}.deb"))?;
    builder
        .write_to(&mut f)
        .with_context(|| anyhow!("Writing deb to {}", f.path().display()))?;
    Ok(())
}
