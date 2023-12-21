use self::parser::Vars;
#[macro_use]
mod parser;

use std::ffi::OsStr;
use std::fmt;
use std::{collections::HashSet, env};

use anyhow::{bail, ensure, Result};
use chrono::{Datelike, Utc};
use clap::Parser;
use serde::Serialize;

/// The base year. Cannot perform releases prior to this year.
const BASE_YEAR: u32 = 2000;
const LAST_YEAR: u32 = 2255;

#[derive(Default, Debug, Clone, Parser)]
pub(crate) struct ReleaseOpts {
    /// Define a version.
    ///
    /// This supports a number of formats, the idea would be that you can use a
    /// single input variable for most if not all of your release needs.
    ///
    /// The supported formats are:
    /// * A version number potentially with a custom prerelease, like
    ///   `1.2.3-pre1`.
    /// * A simple naive date, like `2023-12-11`.
    /// * An alphabetical name, like `nightly` which will result in a dated
    ///   version number where version numbers are strictly required. A version
    ///   suffixed with a number like `nightly1` will be treated as a
    ///   pre-release.
    /// * A date follow by a custom suffix, like `2023-12-11-nightly`.
    /// * It is also possible to use a variable like `%date` to get the custom
    ///   date. For available variable see below.
    ///
    /// A version can also take a simple kind of expression, where each
    /// candidate is separated from left to right using double pipes ('||'). The
    /// first expression for which all variables are defined, and results in a
    /// non-empty expansion will be used.
    ///
    /// This means that with Github Actions, you can uses something like this:
    ///
    /// ```text
    /// --version "${{github.event.inputs.release}} || %date-nightly"
    /// ```
    ///
    /// In this example, the `release` input might be defined by a
    /// workflow_dispatch job, and if undefined the version will default to a
    /// "nightly" dated release.
    ///
    /// Available variables:
    ///
    /// * `%date` - The current date.
    /// * `%{github.tag}` - The tag name from GITHUB_REF if it matches
    ///   `refs/tags/*`.
    /// * `%{github.head}` - The branch name from GITHUB_REF if it matches
    ///   `refs/heads/*`.
    /// * `%{github.sha}` - The sha fetched from GITHUB_SHA.
    ///
    /// You can also define your own variables using `--define <key>=<value>`.
    /// If the value is empty, the variable will be considered undefined.
    #[clap(long, verbatim_doc_comment, value_name = "version")]
    version: Option<String>,
    /// Define a custom variable. See `--version` for more information.
    #[clap(long, value_name = "<key>=<value>")]
    define: Vec<String>,
    /// Never include a release prefix. Even if one is part of the input, it
    /// will be stripped.
    ///
    /// So for example a channel of `v1.0.0` will become `1.0.0` with this
    /// option enabled.
    #[clap(long)]
    no_prefix: bool,
    /// Always ensure that the full version is included in the release string.
    ///
    /// This will pad any version seen with zeros. So `1.0` will become `1.0.0`.
    #[clap(long)]
    full_version: bool,
}

impl ReleaseOpts {
    /// Construct a release from provided arguments.
    pub(crate) fn version<'a>(&'a self, env: &'a ReleaseEnv) -> Result<Version<'_>> {
        let mut version = self.version.as_deref().filter(|c| !c.is_empty());

        if version.is_none() {
            version = env.kick_version.as_deref();
        }

        let span = tracing::info_span! {
            "release",
            GITHUB_EVENT_NAME = env.github_event_name.as_deref(),
            GITHUB_REF = env.github_ref.as_deref(),
            version,
        };

        let _span = span.entered();

        let mut vars = Vars::new(Date::today()?);

        let mut prefixes = HashSet::new();
        prefixes.insert(String::from("v"));

        for define in &self.define {
            let Some((key, value)) = define.split_once('=') else {
                bail!("Bad --define argument `{define}`");
            };

            if value.chars().all(|c| matches!(c, ws!() | '-' | '.')) {
                continue;
            }

            vars.insert(key, value);
        }

        github_release(env, &mut vars);

        let Some(version) = version else {
            bail!("Could not determine version from --version or KICK_VERSION");
        };

        let Some(mut release) = self::parser::expr(version, &vars, &prefixes)? else {
            bail!("Could not determine release from version");
        };

        if self.no_prefix {
            release.prefix = None;
        }

        if self.full_version {
            if let VersionKind::SemanticVersion(version) = &mut release.kind {
                if version.patch.is_none() {
                    version.patch = Some(0);
                }
            }
        }

        Ok(release)
    }
}

pub(crate) struct ReleaseEnv {
    kick_version: Option<String>,
    github_event_name: Option<String>,
    github_ref: Option<String>,
    github_sha: Option<String>,
}

impl ReleaseEnv {
    pub(crate) fn new() -> Self {
        let kick_version = env::var("KICK_VERSION").ok().filter(|e| !e.is_empty());
        let github_event_name = env::var("GITHUB_EVENT_NAME").ok().filter(|e| !e.is_empty());
        let github_ref = env::var("GITHUB_REF").ok().filter(|e| !e.is_empty());
        let github_sha = env::var("GITHUB_SHA").ok().filter(|e| !e.is_empty());

        Self {
            kick_version,
            github_event_name,
            github_ref,
            github_sha,
        }
    }
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
#[serde(rename_all = "kebab-case")]
enum VersionKind<'a> {
    SemanticVersion(SemanticVersion<'a>),
    Date(Date),
    Name(Name<'a>),
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
enum Tail<'a> {
    Hash(&'a str),
    Number(u32),
}

impl Tail<'_> {
    fn as_number(&self) -> Option<u32> {
        match self {
            Tail::Number(number) => Some(*number),
            _ => None,
        }
    }
}

impl fmt::Display for Tail<'_> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Tail::Hash(tail) => tail.fmt(f),
            Tail::Number(tail) => tail.fmt(f),
        }
    }
}

#[derive(Debug, PartialEq, Serialize)]
struct Name<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tail: Option<Tail<'a>>,
}

impl Name<'_> {
    #[inline]
    fn is_pre(&self) -> bool {
        matches!(&self.tail, Some(Tail::Number(..)))
    }
}

impl fmt::Display for Name<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.name.fmt(f)?;

        if let Some(tail) = &self.tail {
            tail.fmt(f)?;
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct Version<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    prefix: Option<&'a str>,
    #[serde(flatten)]
    kind: VersionKind<'a>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    names: Vec<Name<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    append: Vec<&'a str>,
}

impl<'a> Version<'a> {
    /// Check if release is a pre-release.
    pub(crate) fn is_pre(&self) -> bool {
        if let Some(first) = self.names.first() {
            if first.is_pre() {
                return true;
            }
        }

        matches!(&self.kind, VersionKind::Name(name) if name.is_pre())
    }

    pub(crate) fn msi_version(&self) -> Result<String> {
        /// Calculate an MSI-safe version number.
        /// Unfortunately this enforces some unfortunate constraints on the available
        /// version range.
        ///
        /// The computed patch component must fit within 65535
        fn from_version(version: &SemanticVersion, pre: Option<&Name>) -> Result<String> {
            let patch = version.patch.unwrap_or_default();

            if patch > 64 {
                bail!("patch version must not be greater than 64: {}", patch);
            }

            let pre = if let Some(pre) = pre
                .and_then(|c| c.tail.as_ref())
                .and_then(|tail| tail.as_number())
            {
                if pre >= 999 {
                    bail!("pre version must not be greater than 999: {}", pre);
                }

                pre
            } else {
                999
            };

            let last = patch * 1000 + pre;
            Ok(format!("{}.{}.{}", version.major, version.minor, last))
        }

        fn from_date_revision(ymd: Date, pre: Option<&Name>) -> Result<String> {
            let pre = if let Some(pre) = pre
                .and_then(|c| c.tail.as_ref())
                .and_then(|tail| tail.as_number())
            {
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
            VersionKind::SemanticVersion(version) => from_version(version, self.names.first()),
            VersionKind::Date(date) => from_date_revision(*date, self.names.first()),
            VersionKind::Name(..) => bail!("Cannot compute MSI version from channel"),
        }
    }
}

impl fmt::Display for Version<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(prefix) = self.prefix {
            prefix.fmt(f)?;
        }

        match &self.kind {
            VersionKind::SemanticVersion(version) => {
                version.fmt(f)?;
            }
            VersionKind::Date(date) => {
                date.fmt(f)?;
            }
            VersionKind::Name(name) => {
                name.fmt(f)?;
            }
        }

        for name in &self.names {
            write!(f, "-{name}")?;
        }

        for additional in &self.append {
            write!(f, ".{}", additional)?;
        }

        Ok(())
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct SemanticVersion<'a> {
    #[serde(skip)]
    original: &'a str,
    major: u32,
    minor: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    patch: Option<u32>,
}

impl fmt::Display for SemanticVersion<'_> {
    #[inline]
    fn fmt(&self, fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.original.fmt(fmt)
    }
}

impl AsRef<[u8]> for SemanticVersion<'_> {
    #[inline]
    fn as_ref(&self) -> &[u8] {
        self.original.as_bytes()
    }
}

impl AsRef<OsStr> for SemanticVersion<'_> {
    #[inline]
    fn as_ref(&self) -> &OsStr {
        OsStr::new(self.original)
    }
}

/// Define a github release.
fn github_release<'a>(env: &'a ReleaseEnv, vars: &mut Vars<'a>) {
    if let Some(r#ref) = env.github_ref.as_deref() {
        if let Some(tag) = r#ref.strip_prefix("refs/tags/") {
            vars.insert("github.tag", tag);
        }

        if let Some(head) = r#ref.strip_prefix("refs/heads/") {
            vars.insert("github.head", head);
        }
    }

    if let Some(sha) = env.github_sha.as_deref() {
        vars.insert("github.sha", sha);
    }
}
