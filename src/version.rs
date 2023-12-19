use std::env;
use std::ffi::OsStr;
use std::fmt;
use std::str::FromStr;

use anyhow::{bail, Context, Result};
use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike, Utc};
use clap::Parser;

#[derive(Default, Debug, Parser)]
pub(crate) struct VersionOpts {
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

impl VersionOpts {
    /// Construct a release from provided arguments.
    pub(crate) fn release(&self) -> Result<Release> {
        let mut release = None;

        if let Some(channel) = &self.channel {
            release = match (channel.as_str(), NaiveDate::from_str(channel.as_str())) {
                (_, Ok(date)) => Some(Release::Date(date)),
                ("nightly", _) => Some(Release::Nightly(Utc::now().naive_utc())),
                _ => None,
            };
        }

        if let Some(version) = &self.version {
            release = Some(Release::Version(Version::parse(version.as_str())?));
        }

        Ok(release.unwrap_or_else(github_release))
    }
}

pub(super) enum Release {
    Version(Version),
    Nightly(NaiveDateTime),
    Date(NaiveDate),
}

impl Release {
    pub(crate) fn msi_version(&self) -> Result<String> {
        /// Calculate an MSI-safe version number.
        /// Unfortunately this enforces some unfortunate constraints on the available
        /// version range.
        ///
        /// The computed patch component must fit within 65535
        fn from_version(version: &Version) -> Result<String> {
            if version.patch > 64 {
                bail!(
                    "patch version must not be greater than 64: {}",
                    version.patch
                );
            }

            let mut last = 999;

            if let Some(pre) = version.pre {
                if pre >= 999 {
                    bail!(
                        "pre version must not be greater than 999: {}",
                        version.patch
                    );
                }

                last = pre;
            }

            last += version.patch * 1000;
            Ok(format!("{}.{}.{}", version.major, version.minor, last))
        }

        fn from_date_time(date_time: &NaiveDateTime) -> Result<String> {
            let date = date_time.date();

            Ok(format!(
                "{}.{}.{}",
                date.year() - 2023,
                date.month(),
                date.day() * 100 + date.day() + date_time.hour()
            ))
        }

        fn from_date(date: &NaiveDate) -> Result<String> {
            Ok(format!(
                "{}.{}.{}",
                date.year() - 2023,
                date.month(),
                date.day() * 100 + date.day()
            ))
        }

        match self {
            Release::Version(version) => from_version(version),
            Release::Nightly(date_time) => from_date_time(date_time),
            Release::Date(date) => from_date(date),
        }
    }
}

impl fmt::Display for Release {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Release::Version(version) => version.fmt(f),
            Release::Date(date) => date.fmt(f),
            Release::Nightly(date_time) => {
                let date = date_time.date();
                write!(
                    f,
                    "nightly-{}.{}.{}.{}",
                    date.year(),
                    date.month(),
                    date.day(),
                    date_time.hour()
                )
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct Version {
    base: String,
    major: u32,
    minor: u32,
    patch: u32,
    pre: Option<u32>,
}

impl Version {
    /// Open a version by matching it against the given string.
    pub(crate) fn parse(version: &str) -> Result<Version> {
        let (head, pre) = if let Some((version, pre)) = version.rsplit_once('-') {
            (version, Some(pre))
        } else {
            (version, None)
        };

        let mut it = head.split('.');

        let [Some(major), Some(minor), Some(patch), None] =
            [it.next(), it.next(), it.next(), it.next()]
        else {
            bail!("Bad version: {head}");
        };

        let major: u32 = major.parse().context("Bad major version")?;
        let minor: u32 = minor.parse().context("Bad minor version")?;
        let patch: u32 = patch.parse().context("Bad patch version")?;
        let pre: Option<u32> = pre.map(str::parse).transpose().context("Bad pre version")?;

        Ok(Self {
            base: version.to_string(),
            major,
            minor,
            patch,
            pre,
        })
    }
}

impl fmt::Display for Version {
    #[inline]
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.base.fmt(fmt)
    }
}

impl AsRef<[u8]> for Version {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.base.as_bytes()
    }
}

impl AsRef<OsStr> for Version {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        self.base.as_ref()
    }
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
pub(crate) fn github_ref_version() -> Result<Version> {
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
