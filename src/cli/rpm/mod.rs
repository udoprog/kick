mod find_requires;
mod find_requires_by_elf;

use std::collections::BTreeSet;
use std::env::consts::ARCH;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use relative_path::RelativePath;

use crate::config::{rpm_requires, PackageFile, VersionConstraint, VersionRequirement};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::packaging::InstallFile;
use crate::release::Version;

use crate::release::ReleaseOpts;

use super::output::OutputOpts;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    release: ReleaseOpts,
    #[clap(flatten)]
    output: OutputOpts,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let release = opts.release.version(cx.env)?;

    with_repos!(
        cx,
        "build .rpm",
        format_args!("rpm: {:?}", opts),
        |cx, repo| { rpm(cx, repo, opts, &release) }
    );

    Ok(())
}

#[tracing::instrument(skip_all)]
fn rpm(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts, release: &Version<'_>) -> Result<()> {
    let workspace = repo.workspace(cx)?;

    let package = workspace.primary_package()?;
    let name = package.name()?;
    let license = package.license().context("Missing license")?;
    let description = package.description().context("Missing description")?;

    let version = release.to_string();

    let mut pkg = rpm::PackageBuilder::new(name, &version, license, ARCH, description)
        .compression(rpm::CompressionType::Gzip);

    let mut requires = BTreeSet::new();

    for install_file in crate::packaging::install_files(cx, repo)? {
        match install_file {
            InstallFile::Binary(name, path) => {
                requires.extend(if find_requires::detect() {
                    find_requires::find(&path)?
                } else {
                    find_requires_by_elf::find(&path)?
                });

                pkg = pkg
                    .with_file(
                        &path,
                        rpm::FileOptions::new(format!("/usr/bin/{}", name))
                            .mode(rpm::FileMode::Regular { permissions: 0o755 }),
                    )
                    .with_context(|| anyhow!("Adding binary: {}", path.display()))?;
            }
            InstallFile::File(file, source, dest) => {
                pkg = add_file(pkg, file, &source, &dest)?;
            }
        }
    }

    for require in cx.config.get_all(repo, rpm_requires) {
        let dep = match &require.version {
            VersionRequirement::Any => rpm::Dependency::any(&require.package),
            VersionRequirement::Constraint(constraint, version) => match constraint {
                VersionConstraint::Gt => {
                    rpm::Dependency::greater(&require.package, version.to_string())
                }
                VersionConstraint::Ge => {
                    rpm::Dependency::greater_eq(&require.package, version.to_string())
                }
                VersionConstraint::Lt => {
                    rpm::Dependency::less(&require.package, version.to_string())
                }
                VersionConstraint::Le => {
                    rpm::Dependency::less_eq(&require.package, version.to_string())
                }
                VersionConstraint::Eq => rpm::Dependency::eq(&require.package, version.to_string()),
            },
        };

        pkg = pkg.requires(dep);
    }

    for require in requires {
        pkg = pkg.requires(rpm::Dependency::any(require));
    }

    let pkg = pkg.build()?;
    let output = opts.output.make_directory(cx, repo, "rpm");
    let output_path = output.make_path(format!("{name}-{release}-{ARCH}.rpm"))?;
    pkg.write_file(&output_path)
        .with_context(|| anyhow!("Writing rpm to {}", output_path.display()))?;
    Ok(())
}

fn add_file(
    pkg: rpm::PackageBuilder,
    file: &PackageFile,
    source: &Path,
    dest: &RelativePath,
) -> Result<rpm::PackageBuilder> {
    tracing::info!("Adding {} to {dest}", source.display());

    let mut options = rpm::FileOptions::new(format!("/{dest}"));

    if let Some(mode) = file.mode {
        options = options.mode(mode);
    }

    pkg.with_file(source, options)
        .with_context(|| anyhow!("Adding file: {}", source.display()))
}
