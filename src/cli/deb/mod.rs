use std::fs;
use std::path::Path;

use anyhow::bail;
use anyhow::{Context, Result, anyhow};
use clap::Parser;
use relative_path::RelativePath;

use crate::cli::WithRepos;
use crate::config::PackageFile;
use crate::config::VersionRequirement;
use crate::config::deb_depends;
use crate::ctxt::Ctxt;
use crate::deb;
use crate::model::Repo;
use crate::packaging::{self, Mode, Packager};
use crate::release::ReleaseOpts;

use super::output::OutputOpts;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    release: ReleaseOpts,
    #[clap(flatten)]
    output: OutputOpts,
}

pub(crate) fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    with_repos.run("build .deb", format_args!("deb: {opts:?}"), |cx, repo| {
        deb(cx, repo, opts)
    })?;

    Ok(())
}

#[tracing::instrument(skip_all)]
fn deb(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let release = opts.release.version(cx, repo)?;
    let workspace = repo.workspace(cx)?;

    let package = workspace.primary_package()?.ensure_package()?;
    let name = package.name()?;
    let arch = deb::Architecture::Amd64;

    let mut builder = deb::Builder::new(name, arch);

    if let Some(description) = package.description() {
        builder.description(description);
    }

    let debian_version = release.debian_version()?;
    builder.version(&debian_version);

    let mut packager = DebianPackager {
        builder: &mut builder,
    };

    let n = packaging::install_files(&mut packager, cx, repo)?;

    if n > 0 {
        bail!("Stopping due to {n} error(s)");
    };

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

struct DebianPackager<'a> {
    builder: &'a mut deb::Builder,
}

impl Packager for DebianPackager<'_> {
    fn add_binary(&mut self, name: &str, path: &Path) -> Result<()> {
        let meta = path.metadata().context("reading metadata")?;
        let mtime = meta.modified().context("reading modified time")?;
        let contents = fs::read(path).context("reading contents of file")?;

        self.builder
            .insert_file(RelativePath::new("usr/bin").join(name))
            .contents(contents)
            .mode(Mode::EXECUTABLE)
            .mtime(mtime)?;

        Ok(())
    }

    fn add_file(&mut self, file: &PackageFile, path: &Path, dest: &RelativePath) -> Result<()> {
        let meta = path.metadata().context("reading metadata")?;
        let mtime = meta.modified().context("reading modified time")?;
        let contents = fs::read(path).context("reading contents of file")?;

        let mode = if let Some(mode) = file.mode {
            mode
        } else {
            let (mode, _) = crate::packaging::infer_mode(path)?;
            mode
        };

        self.builder
            .insert_file(dest)
            .contents(contents)
            .mtime(mtime)?
            .mode(mode);

        Ok(())
    }
}
