mod find_requires;
mod find_requires_by_elf;

use std::env::consts::{ARCH, EXE_EXTENSION};
use std::fs;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use relative_path::{RelativePath, RelativePathBuf};

use crate::config::{RpmFile, RpmOp};
use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::model::Repo;
use crate::release::{Release, ReleaseEnv};
use crate::workspace;

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
    let env = ReleaseEnv::new();
    let release = opts.release.make(&env)?;

    with_repos!(
        cx,
        "Build rpm",
        format_args!("rpm: {:?}", opts),
        |cx, repo| { rpm(cx, repo, opts, &release) }
    );

    Ok(())
}

#[tracing::instrument(skip_all, fields(source = ?repo.source(), path = repo.path().as_str()))]
fn rpm(
    cx: &Ctxt<'_>,
    repo: &Repo,
    opts: &Opts,
    release: &Release<'_>,
) -> Result<(), anyhow::Error> {
    let root = cx.to_path(repo.path());

    let Some(workspace) = workspace::open(cx, repo)? else {
        bail!("Not a workspace");
    };

    let package = workspace.primary_package()?;
    let name = package.name()?;
    let license = package.license().context("Missing license")?;
    let description = package.description().context("Missing description")?;

    let binary_path = root
        .join("target")
        .join("release")
        .join(name)
        .with_extension(EXE_EXTENSION);

    let requires = if find_requires::detect() {
        find_requires::find(&binary_path)?
    } else {
        find_requires_by_elf::find(&binary_path)?
    };

    let version = release.to_string();

    let output = match &opts.output {
        Some(output) => cx.to_path(repo.path().join(output)),
        None => root.join("target").join("rpm"),
    };

    let output_path = output.join(format!("{name}-{release}-{ARCH}.rpm"));

    tracing::info!("Writing: {}", output_path.display());

    let mut pkg = rpm::PackageBuilder::new(name, &version, license, ARCH, description)
        .compression(rpm::CompressionType::Gzip);

    pkg = pkg
        .with_file(
            &binary_path,
            rpm::FileOptions::new(format!("/usr/bin/{}", name))
                .mode(rpm::FileMode::Regular { permissions: 0o755 }),
        )
        .with_context(|| anyhow!("Adding binary: {}", binary_path.display()))?;

    for file in cx.config.rpm_files(repo) {
        let from = cx.to_path(repo.path());

        let source = RelativePath::new(&file.source);
        let glob = Glob::new(&from, source);
        let dest = RelativePath::new(&file.dest);

        if glob.is_exact() {
            let Some(file_name) = source.file_name() else {
                bail!("Missing file name: {source}");
            };

            let source = cx.to_path(repo.path().join(&file.source));
            let dest = dest.join(file_name);
            pkg = add_file(pkg, file, &source, &dest)?;
        } else {
            let matcher = glob.matcher();

            for source in matcher {
                let relative = source?;

                let Some(file_name) = relative.file_name() else {
                    bail!("Missing file name: {relative}");
                };

                let source = cx.to_path(repo.path().join(&relative));
                let dest = dest.join(file_name);
                pkg = add_file(pkg, file, &source, &dest)?;
            }
        }
    }

    fn add_file(
        pkg: rpm::PackageBuilder,
        file: &RpmFile,
        source: &Path,
        dest: &RelativePath,
    ) -> Result<rpm::PackageBuilder> {
        tracing::info!("Adding {} to {dest}", source.display());

        let mut options = rpm::FileOptions::new(dest.as_str());

        if let Some(mode) = file.mode {
            options = options.mode(mode);
        }

        pkg.with_file(source, options)
            .with_context(|| anyhow!("Adding file: {}", source.display()))
    }

    for require in cx.config.rpm_requires(repo) {
        if let Some((op, version)) = &require.version {
            let dep = match op {
                RpmOp::Gt => rpm::Dependency::greater(&require.package, version.to_string()),
                RpmOp::Ge => rpm::Dependency::greater_eq(&require.package, version.to_string()),
                RpmOp::Lt => rpm::Dependency::less(&require.package, version.to_string()),
                RpmOp::Le => rpm::Dependency::less_eq(&require.package, version.to_string()),
                RpmOp::Eq => rpm::Dependency::eq(&require.package, version.to_string()),
            };

            pkg = pkg.requires(dep);
        } else {
            pkg = pkg.requires(rpm::Dependency::any(&require.package));
        }
    }

    for require in requires {
        pkg = pkg.requires(rpm::Dependency::any(require));
    }

    if !output.is_dir() {
        fs::create_dir_all(output)?;
    }

    let pkg = pkg.build()?;
    pkg.write_file(&output_path)?;
    Ok(())
}
