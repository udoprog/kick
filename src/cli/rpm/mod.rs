mod find_requires;
mod find_requires_by_elf;

use std::collections::BTreeSet;
use std::env::consts::ARCH;
use std::fs::File;
use std::io::{BufWriter, Write as _};
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use clap::Parser;
use relative_path::RelativePath;

use crate::cli::WithRepos;
use crate::config::{PackageFile, VersionConstraint, VersionRequirement, rpm_requires};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::packaging::{self, Mode, Packager};
use crate::release::ReleaseOpts;

use super::output::OutputOpts;

const DEFAULT_LICENSE: &str = "MIT OR Apache-2.0";

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Use RPM format version 4 for improve compatibility with older systems.
    #[clap(long)]
    v4: bool,
    #[clap(flatten)]
    release: ReleaseOpts,
    #[clap(flatten)]
    output: OutputOpts,
}

pub(crate) fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    with_repos.run("build .rpm", format_args!("rpm: {opts:?}"), |cx, repo| {
        rpm(cx, repo, opts)
    })?;

    Ok(())
}

#[tracing::instrument(skip_all)]
fn rpm(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let release = opts.release.version(cx, repo)?;
    let workspace = repo.workspace(cx)?;

    let package = workspace.primary_package()?.ensure_package()?;
    let name = package.name()?;

    let license = match package.license() {
        Some(lic) => lic,
        None => {
            tracing::warn!("{name} no package.license: using '{}'", DEFAULT_LICENSE);
            DEFAULT_LICENSE
        }
    };

    let default_description;

    let description = match package.description() {
        Some(desc) => desc,
        None => {
            default_description = format!("No description for Rust package '{name}'");
            tracing::warn!("{name} no package.description: using '{default_description}'",);
            default_description.as_str()
        }
    };

    let version = release.to_string();

    let build_config = if opts.v4 {
        rpm::BuildConfig::v4()
    } else {
        rpm::BuildConfig::default()
    };

    let build_config = build_config.compression(rpm::CompressionType::Gzip);

    let pkg = rpm::PackageBuilder::new(name, &version, license, ARCH, description)
        .using_config(build_config);

    let mut requires = BTreeSet::new();

    let mut packager = RpmPackager {
        pkg: Some(pkg),
        requires: &mut requires,
    };

    packaging::install_files(&mut packager, cx, repo)?;

    let mut pkg = packager.pkg.context("missing package")?;

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
    let output_path = output.make_path(format_args!("{name}-{release}-{ARCH}.rpm"))?;

    let mut out = BufWriter::new(File::create(&output_path)?);

    pkg.write(&mut out)
        .with_context(|| anyhow!("Writing rpm to {}", output_path.display()))?;

    let mut out = out.into_inner()?;
    out.flush()?;
    Ok(())
}

struct RpmPackager<'a> {
    pkg: Option<rpm::PackageBuilder>,
    requires: &'a mut BTreeSet<String>,
}

impl Packager for RpmPackager<'_> {
    fn add_binary(&mut self, name: &str, path: &Path) -> Result<()> {
        let options =
            rpm::FileOptions::new(format!("/usr/bin/{name}")).mode(rpm::FileMode::Regular {
                permissions: Mode::EXECUTABLE.regular_file(),
            });

        let pkg = self.pkg.take().context("missing package")?;
        self.pkg = Some(pkg.with_file(path, options)?);

        self.requires.extend(if find_requires::detect() {
            find_requires::find(path)?
        } else {
            find_requires_by_elf::find(path)?
        });

        Ok(())
    }

    fn add_file(&mut self, file: &PackageFile, path: &Path, dest: &RelativePath) -> Result<()> {
        let (mode, is_exe) = if let Some(mode) = file.mode {
            (mode, mode.is_executable())
        } else {
            let (mode, is_exe) = packaging::infer_mode(path)?;
            (mode, is_exe)
        };

        if is_exe {
            self.requires.extend(if find_requires::detect() {
                find_requires::find(path)?
            } else {
                find_requires_by_elf::find(path)?
            });
        }

        let options = rpm::FileOptions::new(format!("/{dest}")).mode(mode.regular_file());

        let pkg = self.pkg.take().context("missing package")?;
        self.pkg = Some(
            pkg.with_file(path, options)
                .context("Adding file to rpm package")?,
        );

        Ok(())
    }
}
