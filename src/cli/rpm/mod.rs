mod find_requires;

use std::env::consts::{ARCH, EXE_EXTENSION};
use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;

use crate::config::RpmOp;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::release::Release;
use crate::repo_sets::RepoSet;
use crate::workspace;

use crate::release::ReleaseOpts;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    release: ReleaseOpts,
    /// Output directory to write to.
    #[clap(long, value_name = "output")]
    output: Option<PathBuf>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let release = opts.release.make()?;

    let mut good = RepoSet::default();
    let mut bad = RepoSet::default();

    for repo in cx.repos() {
        if let Err(error) = rpm(cx, repo, opts, &release) {
            tracing::error!("Failed to build rpm");

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
fn rpm(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts, release: &Release) -> Result<(), anyhow::Error> {
    let root = cx.to_path(repo.path());

    let Some(workspace) = workspace::open(cx, repo)? else {
        bail!("Not a workspace");
    };

    let package = workspace.primary_package()?;
    let name = package.name()?;
    let license = package.license().context("Missing license")?;
    let description = package.description().context("Missing description")?;

    let binary = root
        .join("target")
        .join("release")
        .join(name)
        .with_extension(EXE_EXTENSION);

    let requires = find_requires::find_requires(&binary)?;

    let version = release.to_string();

    let output = match &opts.output {
        Some(output) => output,
        None => &root,
    };

    let output_path = output.join(format!("{name}-{release}-{ARCH}.rpm"));

    tracing::info!("Writing: {}", output_path.display());

    let mut pkg = rpm::PackageBuilder::new(name, &version, license, ARCH, description)
        .compression(rpm::CompressionType::Gzip);

    pkg = pkg
        .with_file(
            &binary,
            rpm::FileOptions::new(format!("/usr/bin/{}", name))
                .mode(rpm::FileMode::Regular { permissions: 0o755 }),
        )
        .with_context(|| anyhow!("Adding binary: {}", binary.display()))?;

    for file in cx.config.rpm_files(repo) {
        let source = root.join(&file.source);
        tracing::info!("Adding file: {}", source.display());

        let mut options = rpm::FileOptions::new(&file.dest);

        if let Some(mode) = file.mode {
            options = options.mode(mode);
        }

        pkg = pkg
            .with_file(&source, options)
            .with_context(|| anyhow!("Adding file: {}", source.display()))?;
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
