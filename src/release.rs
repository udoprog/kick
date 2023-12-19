use std::ffi::OsStr;
use std::fmt;
use std::{env, str::FromStr};

use anyhow::{bail, ensure, Context, Result};
use chrono::{Datelike, Utc};
use clap::Parser;

/// The base year. Cannot perform releases prior to this year.
const BASE_YEAR: u32 = 2000;
const LAST_YEAR: u32 = 2255;

/// A valid year-month-day combination.
#[derive(Debug, Clone, Copy)]
pub(super) struct Date {
    year: u32,
    month: u32,
    day: u32,
}

impl Date {
    fn new(year: i32, month: u32, day: u32) -> Result<Self> {
        if year < 0 {
            bail!("Year must be positive: {}", year);
        };

        let year = year as u32;

        ensure!(
            (BASE_YEAR..LAST_YEAR).contains(&year),
            "Year must be within {BASE_YEAR}..{LAST_YEAR}, but was {}",
            year
        );

        Ok(Self { year, month, day })
    }

    fn today() -> Result<Self> {
        let now = Utc::now().naive_local().date();
        Self::new(now.year(), now.month(), now.day())
    }
}

impl fmt::Display for Date {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.year, self.month, self.day)
    }
}

impl FromStr for Date {
    type Err = anyhow::Error;

    #[inline]
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut it = s.split('-');
        let year = it
            .next()
            .context("Missing year")?
            .parse()
            .context("Bad year")?;
        let month = it
            .next()
            .context("Missing month")?
            .parse()
            .context("Bad month")?;
        let day = it
            .next()
            .context("Missing day")?
            .parse()
            .context("Bad day")?;

        if it.next().is_some() {
            bail!("Too many components");
        }

        Self::new(year, month, day)
    }
}

#[derive(Default, Debug, Parser)]
pub(crate) struct ReleaseOpts {
    /// Define a release channel.
    ///
    /// This is a very broad definition and supports a number of formats, the
    /// idea would be that you can use a single input variable for most if not
    /// all of your release needs.
    ///
    /// The supported formats are:
    /// * A simple naive date, like `2023-12-11`.
    /// * A version number, like `1.2.3-pre1`.
    /// * A alphanumerical channel name, like `nightly` which will result in a
    ///   dated release with the current naive date and the given channel name.
    /// * A alphanumerical channel suffixed with a date, like
    ///   `nightly-2023-12-11` which will make a dated release with the given
    ///   channel name.
    #[clap(long, verbatim_doc_comment, value_name = "channel")]
    channel: Option<String>,
    /// Define a release version.
    #[clap(long, value_name = "version")]
    version: Option<String>,
    /// Define a revision release.
    ///
    /// This only applies to date-based releases, and can be used to perform
    /// multiple releases in a given day up to a maximum of 99.
    #[clap(long, value_name = "version", default_value_t)]
    revision: u32,
    /// Append additional components to the release string, separated by dots.
    ///
    /// A use-case for this is to specify the fedora release, like `fc39` which
    /// will then be appended verbatim to the version string.
    #[clap(long, value_name = "part")]
    append: Vec<String>,
}

impl ReleaseOpts {
    /// Construct a release from provided arguments.
    pub(crate) fn make(&self) -> Result<Release> {
        ensure!(
            self.revision < 100,
            "Revision must be less than 100: {}",
            self.revision
        );

        let mut channel = None;

        let kind = 'out: {
            match (&self.version, &self.channel) {
                (Some(version), string) => {
                    channel = string.as_deref().map(Box::from);
                    ReleaseKind::Versioned(Version::parse(version.as_str())?)
                }
                (None, Some(string)) => {
                    if let Ok(date) = Date::from_str(string.as_str()) {
                        break 'out ReleaseKind::Dated(date);
                    };

                    if let Ok(version) = Version::parse(string) {
                        break 'out ReleaseKind::Versioned(version);
                    }

                    let (string, date) = if let Some((string, date)) = string.split_once('-') {
                        (string, Date::from_str(date)?)
                    } else {
                        (
                            string.as_str(),
                            Date::today().context("Getting today's date")?,
                        )
                    };

                    if !is_valid_channel(string) {
                        bail!("Invalid channel: {string}");
                    }

                    channel = Some(Box::from(string));
                    ReleaseKind::Dated(date)
                }
                _ => github_release_kind()?,
            }
        };

        Ok(Release {
            kind,
            channel,
            revision: self.revision,
            append: self.append.clone(),
        })
    }
}

pub(super) enum ReleaseKind {
    Versioned(Version),
    Dated(Date),
}

pub(super) struct Release {
    kind: ReleaseKind,
    channel: Option<Box<str>>,
    revision: u32,
    append: Vec<String>,
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

        fn from_date_revision(ymd: Date, revision: u32) -> Result<String> {
            Ok(format!(
                "{}.{}.{}",
                ymd.year - BASE_YEAR,
                ymd.month,
                ymd.day * 100 + revision
            ))
        }

        match &self.kind {
            ReleaseKind::Versioned(version) => from_version(version),
            ReleaseKind::Dated(date) => from_date_revision(*date, self.revision),
        }
    }
}

impl fmt::Display for Release {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ReleaseKind::Versioned(version) => {
                version.fmt(f)?;
            }
            ReleaseKind::Dated(date) => {
                date.fmt(f)?;
            }
        }

        if let Some(name) = self.channel.as_deref() {
            write!(f, "-{name}")?;

            if self.revision != 0 {
                self.revision.fmt(f)?;
            }
        } else if self.revision != 0 {
            write!(f, "-r{}", self.revision)?;
        }

        for additional in &self.append {
            write!(f, ".{}", additional)?;
        }

        Ok(())
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
fn github_release_kind() -> Result<ReleaseKind> {
    Ok(match github_ref_version() {
        Err(error) => {
            tracing::warn!("Assuming dated release since we couldn't determine tag: {error}");
            ReleaseKind::Dated(Date::today()?)
        }
        Ok(version) => ReleaseKind::Versioned(version),
    })
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

/// Test if the channel is valid.
fn is_valid_channel(string: &str) -> bool {
    let mut it = string.chars();

    let Some(c) = it.next() else {
        return false;
    };

    if !c.is_alphabetic() {
        return false;
    }

    it.all(|c| c.is_alphanumeric())
}
