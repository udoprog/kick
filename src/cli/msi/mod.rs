use self::version::{Release, Version};
mod version;

use anyhow::{bail, Result};
use chrono::{NaiveDate, Utc};
use clap::Parser;

use std::env;
use std::str::FromStr;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::repo_sets::RepoSet;
use crate::wix;
use crate::workspace;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// Define a release channel.
    ///
    /// Valid channels are: nightly which will use the current date, or a valid
    /// naive date like `2023-12-11`.
    #[clap(long, value_name = "channel")]
    channel: Option<String>,
    /// Define a release version.
    #[clap(long, value_name = "version")]
    version: Option<String>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    let mut release = None;

    if let Some(channel) = &opts.channel {
        release = match (channel.as_str(), NaiveDate::from_str(channel.as_str())) {
            (_, Ok(date)) => Some(Release::Date(date)),
            ("nightly", _) => Some(Release::Nightly(Utc::now().naive_utc())),
            _ => None,
        };
    }

    if let Some(version) = &opts.version {
        release = Some(Release::Version(Version::parse(version.as_str())?));
    }

    let release = release.unwrap_or_else(github_release);
    let file_version = release.file_version()?;

    let mut good = RepoSet::default();
    let mut bad = RepoSet::default();

    for repo in cx.repos() {
        if let Err(error) = msi(cx, repo, &release, &file_version) {
            tracing::error!("Failed to build MSI: {error}");
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
    let wsx_file = root.join("wix").join("main.wxs");

    if !wsx_file.is_file() {
        bail!("Missing: {}", wsx_file.display());
    }

    let builder = wix::Builder::new(&wix_dir, name, release)?;
    builder.build(wsx_file, file_version)?;
    builder.link()?;
    Ok(())
}

/// Get the github release to build.
fn github_release() -> Release {
    match github_ref_version() {
        Err(error) => {
            tracing::warn!("Assuming nightly release since we couldn't determine tag: {error}");
            Release::Nightly(Utc::now().naive_local())
        }
        Ok(version) => Release::Version(version),
    }
}

/// Get the version from GITHUB_REF.
fn github_ref_version() -> Result<Version> {
    let version = match env::var("GITHUB_REF") {
        Ok(version) => version,
        _ => bail!("Missing: GITHUB_REF"),
    };

    let mut it = version.split('/');

    let version = match (it.next(), it.next(), it.next()) {
        (Some("refs"), Some("tags"), Some(version)) => Version::parse(version)?,
        _ => bail!("Expected GITHUB_REF: refs/tags/*"),
    };

    Ok(version)
}
