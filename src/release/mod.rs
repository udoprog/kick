use self::parser::Vars;
#[macro_use]
mod parser;

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt;

use anyhow::{Result, bail, ensure};
use chrono::{Datelike, Utc};
use clap::Parser;
use serde::Serialize;

use crate::Repo;
use crate::ctxt::Ctxt;
use crate::env::Env;

/// The base year. Cannot perform releases prior to this year.
const BASE_YEAR: u32 = 2000;
const LAST_YEAR: u32 = 2255;

#[derive(Default, Debug, Clone, Parser)]
pub(crate) struct ReleaseOpts {
    /// Define a version.
    ///
    /// Note that can also be defined through the `KICK_VERSION` environment
    /// variable. If a switch is specified at the same time the switch takes
    /// priority.
    ///
    /// This primarily supports plain versions, dates, or tags, such as `1.2.3`,
    /// `2021-01-01`, or `nightly1` and will be coerced as appropriate into a
    /// target version specification depending in which type of package is being
    /// built.
    ///
    /// This also supports simple expressions such as `$VALUE || %date` which
    /// are evaluated left-to-right and picks the first non-empty version
    /// defined.
    ///
    /// For a full specification of the supported format, see the wobbly version
    /// specification:
    /// https://github.com/udoprog/kick/blob/main/WOBBLY_VERSIONS.md
    #[clap(long, value_name = "version")]
    version: Option<String>,
    /// Append additional components to the version.
    ///
    /// For example, if we start with a version like `1.2.3-beta1`, appending
    /// `fc39` would result in `1.2.3-beta1.fc39`.
    ///
    /// A component must be a valid identifier, so it can only contain ascii
    /// characters and digits and must start with a character.
    ///
    /// Empty components will be ignored and invalid components will cause an
    /// error.
    #[clap(long, value_name = "component")]
    append: Vec<String>,
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
    pub(crate) fn version<'a>(&'a self, cx: &Ctxt<'a>, repo: &'a Repo) -> Result<Version<'a>> {
        let today = Date::today()?;

        if let Some(version) = self.try_env_argument(cx.env, today)? {
            return Ok(version);
        };

        let Some(version) = self.try_project(cx, repo, today)? else {
            bail!(
                "Could not determine version, this can be done through --version, KICK_VERSION, or a project configuration like Cargo.toml"
            );
        };

        Ok(version)
    }

    /// Try to construct a kick version.
    pub(crate) fn try_env_argument<'a>(
        &'a self,
        env: &'a Env,
        today: Date,
    ) -> Result<Option<Version<'a>>> {
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

        let Some(version) = version else {
            return Ok(None);
        };

        Ok(Some(self.parse_version(env, today, version)?))
    }

    /// Try to construct a kick version.
    pub(crate) fn try_project<'a>(
        &'a self,
        cx: &Ctxt<'a>,
        repo: &'a Repo,
        today: Date,
    ) -> Result<Option<Version<'a>>> {
        let Some(workspace) = repo.try_workspace(cx)? else {
            return Ok(None);
        };

        let package = workspace.primary_package()?.ensure_package()?;

        let Some(version) = package.version() else {
            return Ok(None);
        };

        let version = self.parse_version(cx.env, today, version)?;
        Ok(Some(version))
    }

    fn parse_version<'a>(
        &'a self,
        env: &'a Env,
        today: Date,
        version: &'a str,
    ) -> Result<Version<'a>, anyhow::Error> {
        let mut vars = Vars::new(today);

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

        let mut prefixes = HashSet::new();
        prefixes.insert(String::from("v"));

        let Some(mut version) = self::parser::expr(version, &vars, &prefixes)? else {
            bail!("Could not determine release from version");
        };

        if self.no_prefix {
            version.prefix = None;
        }

        if self.full_version
            && let VersionKind::SemanticVersion(version) = &mut version.kind
            && version.patch.is_none()
        {
            version.patch = Some(0);
        }

        for append in &self.append {
            let append = append.trim();

            if append.is_empty() {
                continue;
            }

            ensure!(
                append.chars().all(|c| matches!(c, ident_cont!())),
                "Illegal appended component '{}', must only contain ascii characters or digits",
                append
            );

            version.push(append);
        }

        Ok(version)
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

    pub(crate) fn today() -> Result<Self> {
        let now = Utc::now().naive_local().date();
        Self::new(now.year(), now.month(), now.day())
    }
}

impl fmt::Display for Date {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
enum VersionKind<'a> {
    SemanticVersion(SemanticVersion<'a>),
    Date(Date),
    Name(Tag<'a>),
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
enum Tail<'a> {
    Hash(&'a str),
    Number(u32),
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

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
struct Tag<'a> {
    name: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    tail: Option<Tail<'a>>,
}

impl Tag<'_> {
    #[inline]
    fn is_pre(&self) -> bool {
        matches!(&self.tail, Some(Tail::Number(..)))
    }
}

impl fmt::Display for Tag<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.name.fmt(f)?;

        if let Some(tail) = &self.tail {
            tail.fmt(f)?;
        }

        Ok(())
    }
}

impl<'a> AsRef<Tag<'a>> for Tag<'a> {
    #[inline]
    fn as_ref(&self) -> &Tag<'a> {
        self
    }
}

#[derive(Debug, PartialEq, Serialize)]
pub(super) struct Version<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    prefix: Option<&'a str>,
    #[serde(flatten)]
    kind: VersionKind<'a>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    names: Vec<Tag<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    append: Vec<&'a str>,
}

impl<'a> Version<'a> {
    /// Check if release is a pre-release.
    pub(crate) fn is_pre(&self) -> bool {
        if let Some(first) = self.names.first()
            && first.is_pre()
        {
            return true;
        }

        matches!(&self.kind, VersionKind::Name(name) if name.is_pre())
    }

    /// Append a verbatim component to the version.
    pub(crate) fn push(&mut self, part: &'a str) {
        self.append.push(part);
    }

    /// Coerce into a debian version.
    pub(crate) fn debian_version(&self) -> Result<String> {
        match &self.kind {
            VersionKind::SemanticVersion(version) => {
                if let (Some(name), other) = find_pre(&self.names) {
                    return Ok(format!("{version}~{name}{}", dot_extend(&other)));
                }

                Ok(format!("{version}{}", dot_extend(&self.names)))
            }
            VersionKind::Date(date) => {
                if let (Some(name), other) = find_pre(&self.names) {
                    return Ok(format!(
                        "{}.{}.{}~{name}{}",
                        date.year,
                        date.month,
                        date.day,
                        dot_extend(&other)
                    ));
                }

                Ok(format!(
                    "{}.{}.{}{}",
                    date.year,
                    date.month,
                    date.day,
                    dot_extend(&self.names)
                ))
            }
            VersionKind::Name(name) => {
                let (Some(name), other) = find_pre([name].into_iter().chain(&self.names)) else {
                    bail!("Could not determine debian version");
                };

                Ok(format!("0.0.0~{name}{}", dot_extend(&other)))
            }
        }
    }

    /// Ensures that the version is a valid ProductVersion, suitable for use in
    /// an MSI installer.
    ///
    /// See:
    /// <https://learn.microsoft.com/en-us/windows/win32/msi/productversion>.
    pub(crate) fn msi_version(&self) -> Result<String> {
        /// Validate a pre-release.
        fn validate_pre(pre: u32) -> Result<u32> {
            ensure!(pre < 999, "Pre-release number must be less than 999: {pre}");
            Ok(pre)
        }

        /// Calculate an MSI-safe version number.
        /// Unfortunately this enforces some unfortunate constraints on the available
        /// version range.
        ///
        /// The computed patch component must fit within 65535
        fn from_version(version: &SemanticVersion, pre: Option<u32>) -> Result<String> {
            ensure!(
                version.major <= 255,
                "Major version must not be greater than 255: {}",
                version.major
            );

            ensure!(
                version.minor <= 255,
                "Minor version must not be greater than 255: {}",
                version.minor
            );

            let patch = version.patch.unwrap_or_default();

            ensure!(
                patch <= 64,
                "Patch version must not be greater than 64: {patch}"
            );

            let pre = if let Some(pre) = pre {
                validate_pre(pre)?
            } else {
                999
            };

            let last = patch * 1000 + pre;
            Ok(format!("{}.{}.{}", version.major, version.minor, last))
        }

        fn from_date_revision(ymd: Date, pre: Option<u32>) -> Result<String> {
            let pre = if let Some(pre) = pre {
                validate_pre(pre)?
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

        fn from_name(pre: Option<u32>) -> Result<String> {
            let pre = if let Some(pre) = pre {
                validate_pre(pre)?
            } else {
                999
            };

            Ok(format!("0.0.{pre}"))
        }

        match &self.kind {
            VersionKind::SemanticVersion(version) => {
                from_version(version, find_pre_only(&self.names))
            }
            VersionKind::Date(date) => from_date_revision(*date, find_pre_only(&self.names)),
            VersionKind::Name(name) => {
                from_name(find_pre_only([name].into_iter().chain(&self.names)))
            }
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
            write!(f, ".{additional}")?;
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
fn github_release<'a>(env: &'a Env, vars: &mut Vars<'a>) {
    if let Some(tag) = env.github_tag() {
        vars.insert("github.tag", tag);
    }

    if let Some(head) = env.github_head() {
        vars.insert("github.head", head);
    }

    if let Some(sha) = env.github_sha.as_deref() {
        vars.insert("github.sha", sha);
    }
}

/// Find the first plausible pre-release version in the version string.
fn find_pre_only<'a, I>(names: I) -> Option<u32>
where
    I: IntoIterator,
    I::Item: AsRef<Tag<'a>>,
{
    for name in names {
        if let Some(Tail::Number(number)) = name.as_ref().tail {
            return Some(number);
        }
    }

    None
}

/// Find pre-release including full name.
fn find_pre<'a, I>(names: I) -> (Option<Tag<'a>>, Vec<Tag<'a>>)
where
    I: IntoIterator,
    I::Item: AsRef<Tag<'a>>,
{
    let mut found = None;
    let mut other = Vec::new();

    for name in names {
        let name = *name.as_ref();

        if found.is_none() && matches!(name.tail, Some(Tail::Number(..))) {
            found = Some(name);
            continue;
        }

        other.push(name);
    }

    (found, other)
}

fn dot_extend<I>(iter: I) -> DotExtend<I> {
    DotExtend { iter }
}

struct DotExtend<I> {
    iter: I,
}

impl<I> fmt::Display for DotExtend<I>
where
    I: Copy + IntoIterator,
    I::Item: fmt::Display,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for name in self.iter {
            write!(f, ".{name}")?;
        }

        Ok(())
    }
}
