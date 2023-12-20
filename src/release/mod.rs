mod parser;

use std::env;
use std::ffi::OsStr;
use std::fmt;

use anyhow::{bail, ensure, Result};
use chrono::{Datelike, Utc};
use clap::Parser;
use serde::Serialize;

/// The base year. Cannot perform releases prior to this year.
const BASE_YEAR: u32 = 2000;
const LAST_YEAR: u32 = 2255;

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
    /// * A version number potentially with a custom suffix, like `1.2.3-pre1`.
    /// * A alphanumerical channel name, like `nightly` which will result in a
    ///   dated version number where version numbers are strictly required. A
    ///   channel suffixed with a number like `nightly1` will be treated as a
    ///   pre-release.
    /// * A date follow by a custom suffix, like `2023-12-11-nightly`.
    /// * `%date` will be replaced with the current naive date in expressions
    ///  like `%date` or `%date-nightly1`.
    ///
    /// Finally a channel can take a simple kind of expression, where each
    /// candidate is separated from left to right using `||`. This allows the
    /// use of variables which might evaluate to empty strings, like this:
    ///
    /// --channel "${{github.event.inputs.release}} || %date-nightly"
    ///
    /// In this instance, the `release` input might be defined by a
    /// workflow_dispatch job, and if undefined the channel will default to a
    /// nightly dated release.
    #[clap(long, verbatim_doc_comment, value_name = "channel")]
    channel: Option<String>,
    /// Append additional components to the release string, separated by dots.
    ///
    /// A use-case for this is to specify the fedora release, like `fc39` which
    /// will then be appended verbatim to the version string.
    #[clap(long, value_name = "part")]
    append: Vec<String>,
    /// Do not process run as a release based on github information, such as
    /// `GITHUB_REF`.
    #[clap(long)]
    github_release: bool,
}

impl ReleaseOpts {
    /// Construct a release from provided arguments.
    pub(crate) fn make<'a>(&'a self, env: &'a ReleaseEnv) -> Result<Release<'_>> {
        let channel = self.channel.as_deref().filter(|c| !c.is_empty());

        let span = tracing::info_span! {
            "release",
            GITHUB_EVENT_NAME = env.github_event_name.as_deref(),
            GITHUB_REF = env.github_ref.as_deref(),
            channel,
        };

        let _span = span.entered();

        let mut release = 'out: {
            if let Some(channel) = channel {
                if let Some(release) = channel_to_release(channel)? {
                    break 'out release;
                }
            }

            if self.github_release {
                if let Some(release) = github_release(env)? {
                    break 'out release;
                }
            }

            tracing::warn!("Assuming dated release since we couldn't determine other release kind");

            Release {
                prefix: None,
                kind: ReleaseKind::Date {
                    date: Date::today()?,
                    channel: None,
                },
                append: Vec::new(),
            }
        };

        for append in &self.append {
            release.append.push(append);
        }

        Ok(release)
    }
}

pub(crate) struct ReleaseEnv {
    github_event_name: Option<String>,
    github_ref: Option<String>,
}

impl ReleaseEnv {
    pub(crate) fn new() -> Self {
        let github_event_name = env::var("GITHUB_EVENT_NAME").ok().filter(|e| !e.is_empty());
        let github_ref = env::var("GITHUB_REF").ok().filter(|e| !e.is_empty());

        Self {
            github_event_name,
            github_ref,
        }
    }
}

fn channel_to_release(string: &str) -> Result<Option<Release<'_>>> {
    self::parser::expr(string)
}

/// A valid year-month-day combination.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub(super) struct Date {
    year: u32,
    month: u32,
    day: u32,
}

impl Date {
    fn new(year: i32, month: u32, day: u32) -> Result<Self> {
        if chrono::NaiveDate::from_ymd_opt(year, month, day).is_none() {
            bail!("Invalid date: {}.{}.{}", year, month, day);
        }

        if year < 0 {
            bail!("Year must be positive: {year}");
        }

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

#[derive(Debug, PartialEq, Serialize)]
#[serde(untagged)]
enum ReleaseKind<'a> {
    Version {
        version: Version<'a>,
        #[serde(skip_serializing_if = "Option::is_none")]
        channel: Option<Channel<'a>>,
    },
    Date {
        date: Date,
        #[serde(skip_serializing_if = "Option::is_none")]
        channel: Option<Channel<'a>>,
    },
    Name {
        channel: Channel<'a>,
    },
}

#[derive(Debug, PartialEq, Serialize)]
struct Channel<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pre: Option<u32>,
}

impl fmt::Display for Channel<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.name.fmt(f)?;

        if let Some(pre) = self.pre {
            pre.fmt(f)?;
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct Release<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    prefix: Option<&'a str>,
    #[serde(flatten)]
    kind: ReleaseKind<'a>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    append: Vec<&'a str>,
}

impl Release<'_> {
    pub(crate) fn msi_version(&self) -> Result<String> {
        /// Calculate an MSI-safe version number.
        /// Unfortunately this enforces some unfortunate constraints on the available
        /// version range.
        ///
        /// The computed patch component must fit within 65535
        fn from_version(version: &Version, channel: Option<&Channel>) -> Result<String> {
            if version.patch > 64 {
                bail!(
                    "patch version must not be greater than 64: {}",
                    version.patch
                );
            }

            let pre = if let Some(pre) = channel.and_then(|c| c.pre) {
                if pre >= 999 {
                    bail!(
                        "pre version must not be greater than 999: {}",
                        version.patch
                    );
                }

                pre
            } else {
                999
            };

            let last = version.patch * 1000 + pre;
            Ok(format!("{}.{}.{}", version.major, version.minor, last))
        }

        fn from_date_revision(ymd: Date, channel: Option<&Channel>) -> Result<String> {
            let pre = if let Some(pre) = channel.and_then(|c| c.pre) {
                if pre >= 999 {
                    bail!("pre version must not be greater than 999: {pre}");
                }

                pre
            } else {
                999
            };

            Ok(format!(
                "{}.{}.{}",
                ymd.year - BASE_YEAR,
                ymd.month,
                ymd.day * 1000 + pre
            ))
        }

        match &self.kind {
            ReleaseKind::Version {
                version, channel, ..
            } => from_version(version, channel.as_ref()),
            ReleaseKind::Date { date, channel } => from_date_revision(*date, channel.as_ref()),
            ReleaseKind::Name { .. } => bail!("Cannot compute MSI version from channel"),
        }
    }
}

impl fmt::Display for Release<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = self.prefix {
            prefix.fmt(f)?;
        }

        let (prefix, channel) = match &self.kind {
            ReleaseKind::Version { version, channel } => {
                version.fmt(f)?;
                ("-", channel.as_ref())
            }
            ReleaseKind::Date { date, channel } => {
                date.fmt(f)?;
                ("-", channel.as_ref())
            }
            ReleaseKind::Name { channel } => ("", Some(channel)),
        };

        if let Some(channel) = channel {
            write!(f, "{prefix}{channel}")?;
        }

        for additional in &self.append {
            write!(f, ".{}", additional)?;
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct Version<'a> {
    #[serde(skip)]
    original: &'a str,
    major: u32,
    minor: u32,
    patch: u32,
}

impl fmt::Display for Version<'_> {
    #[inline]
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.original.fmt(fmt)
    }
}

impl AsRef<[u8]> for Version<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.original.as_bytes()
    }
}

impl AsRef<OsStr> for Version<'_> {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        OsStr::new(self.original)
    }
}

/// Define a github release.
fn github_release(env: &ReleaseEnv) -> Result<Option<Release<'_>>> {
    match (env.github_event_name.as_deref(), env.github_ref.as_deref()) {
        (Some("push"), Some(r#ref)) => {
            if let Some(tag) = r#ref.strip_prefix("refs/tags/") {
                return Ok(channel_to_release(tag)?);
            }

            if let Some(channel) = r#ref.strip_prefix("refs/heads/") {
                return Ok(channel_to_release(channel)?);
            }

            tracing::warn!("Unsupported GITHUB_REF");
        }
        (Some("schedule" | "workflow_dispatch"), _) => {}
        (Some(value), _) => {
            bail!("Unsupported GITHUB_EVENT_NAME='{value}'");
        }
        (None, Some(value)) => {
            bail!("Specifying GITHUB_REF='{value}' without GITHUB_EVENT_NAME='push' does nothing");
        }
        _ => {}
    }

    Ok(None)
}
