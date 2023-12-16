use std::ffi::OsStr;
use std::fmt;

use anyhow::{bail, Context, Result};
use chrono::{Datelike, NaiveDate, NaiveDateTime, Timelike};

pub(super) enum Release {
    Version(Version),
    Nightly(NaiveDateTime),
    Date(NaiveDate),
}

impl Release {
    pub(crate) fn file_version(&self) -> Result<String> {
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
