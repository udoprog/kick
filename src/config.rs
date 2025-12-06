use core::cell::RefCell;
use core::mem;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::hash::Hash;
use std::io::{self, BufRead, BufReader, Write};
use std::iter;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::str::{self, FromStr};

use anyhow::{Context, Error, Result, anyhow, bail};
use musli::{Decode, Encode};
use relative_path::{RelativePath, RelativePathBuf};
use semver::Version;
use tempfile::NamedTempFile;
use url::Url;

use crate::KICK_TOML;
use crate::ctxt::Paths;
use crate::glob::Glob;
use crate::keys::Keys;
use crate::model::{Repo, RepoInfo, RepoParams, RepoRef, RepoSource};
use crate::shell::Shell;
use crate::templates::{Template, Templating};

/// Default job name.
const DEFAULT_CI_NAME: &str = "CI";
/// Default weekly name.
const DEFAULT_WEEKLY_NAME: &str = "Weekly";
/// Default license to use in configuration.
const DEFAULT_LICENSE: &str = "MIT/Apache-2.0";

struct ErrorMarker;

/// Set up variable defaults.
pub(crate) fn defaults() -> toml::Table {
    let mut defaults = toml::Table::new();
    defaults.insert(
        String::from("ci_name"),
        toml::Value::String(String::from(DEFAULT_CI_NAME)),
    );
    defaults.insert(
        String::from("weekly_name"),
        toml::Value::String(String::from(DEFAULT_WEEKLY_NAME)),
    );
    defaults
}

#[derive(Clone, Encode, Decode)]
pub(crate) struct Replaced {
    path: PathBuf,
    content: Vec<u8>,
    replacement: Box<str>,
    ranges: Vec<Range<usize>>,
}

impl Replaced {
    /// Get the path to replace.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Get replacement string.
    pub(crate) fn replacement(&self) -> &str {
        &self.replacement
    }

    fn write_ranges<O>(&self, mut out: O) -> io::Result<()>
    where
        O: Write,
    {
        let mut last = 0;

        for range in &self.ranges {
            out.write_all(&self.content[last..range.start])?;
            out.write_all(self.replacement.as_bytes())?;
            last = range.end;
        }

        out.write_all(&self.content[last..])?;
        Ok(())
    }

    /// Perform the given write.
    pub(crate) fn save(&self) -> Result<()> {
        let Some(parent) = self.path.parent() else {
            bail!("{}: missing parent directory", self.path.display());
        };

        let mut file = NamedTempFile::new_in(parent)?;
        self.write_ranges(&mut file)
            .with_context(|| self.path.display().to_string())?;
        file.persist(&self.path)?;
        Ok(())
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct Upgrade {
    /// Packages to exclude during an upgrade.
    pub(crate) exclude: BTreeSet<String>,
}

impl Upgrade {
    fn merge_with(&mut self, other: Self) {
        self.exclude.extend(other.exclude);
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct RpmPackage {
    /// Requirements to add to an rpm package.
    pub(crate) requires: Vec<RpmRequire>,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct DebPackage {
    /// Dependencies to add to a debian package.
    pub(crate) depends: Vec<DebDependency>,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct Package {
    /// Packages to include in an rpm package.
    pub(crate) files: Vec<PackageFile>,
    /// Options specific to rpm packages.
    pub(crate) rpm: RpmPackage,
    /// Options specific to deb packages.
    pub(crate) deb: DebPackage,
}

impl Package {
    fn merge_with(&mut self, other: Self) {
        self.files.extend(other.files);
        self.rpm.requires.extend(other.rpm.requires);
        self.deb.depends.extend(other.deb.depends);
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct DenyAction {
    /// The name of a denied action.
    pub(crate) name: String,
    /// The reason an action is denied.
    pub(crate) reason: Option<String>,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct LatestAction {
    /// The name of an action.
    pub(crate) name: String,
    /// The latest version available of the given action.
    pub(crate) version: String,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct Actions {
    /// Packages to include in an rpm package.
    pub(crate) deny: Vec<DenyAction>,
    /// Latest versions of available actions.
    pub(crate) latest: Vec<LatestAction>,
}

impl Actions {
    fn merge_with(&mut self, other: Self) {
        self.deny.extend(other.deny);
        self.latest.extend(other.latest);
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct PackageFile {
    /// The source of an rpm file.
    pub(crate) source: RelativePathBuf,
    /// Destination of an rpm file.
    pub(crate) dest: RelativePathBuf,
    /// The mode of a file.
    pub(crate) mode: Option<u16>,
}

#[derive(Clone, Debug, Copy)]
pub(crate) enum VersionConstraint {
    /// Greater than.
    Gt,
    /// Greater than or equal to.
    Ge,
    /// Less than.
    Lt,
    /// Less than or equal to.
    Le,
    /// Equal to.
    Eq,
}

impl fmt::Display for VersionConstraint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VersionConstraint::Gt => write!(f, ">"),
            VersionConstraint::Ge => write!(f, ">="),
            VersionConstraint::Lt => write!(f, "<"),
            VersionConstraint::Le => write!(f, "<="),
            VersionConstraint::Eq => write!(f, "="),
        }
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) enum VersionRequirement {
    #[default]
    Any,
    Constraint(VersionConstraint, Version),
}

impl FromStr for VersionRequirement {
    type Err = Error;

    #[inline]
    fn from_str(string: &str) -> Result<Self, Self::Err> {
        if string == "*" {
            return Ok(VersionRequirement::Any);
        }

        let Some((op, version)) = string.split_once(' ') else {
            return Err(anyhow!("illegal version specification: {string}"));
        };

        let op = match op {
            "<" => VersionConstraint::Lt,
            "<=" => VersionConstraint::Le,
            "=" => VersionConstraint::Eq,
            ">=" => VersionConstraint::Ge,
            ">" => VersionConstraint::Gt,
            version => return Err(anyhow!("illegal version constraint: {version}")),
        };

        Ok(VersionRequirement::Constraint(op, Version::parse(version)?))
    }
}

#[derive(Default, Debug, Clone)]
pub(crate) struct RpmRequire {
    /// The package being required.
    pub(crate) package: String,
    /// The version being required.
    pub(crate) version: VersionRequirement,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct DebDependency {
    /// The package being required.
    pub(crate) package: String,
    /// The version being required.
    pub(crate) version: VersionRequirement,
}

#[derive(Debug, Clone)]
pub(crate) struct Replacement {
    /// Replacements to perform in a given package.
    pub(crate) package_name: Option<String>,
    /// Replacement path.
    pub(crate) paths: Vec<RelativePathBuf>,
    /// A regular expression pattern to replace.
    pub(crate) pattern: regex::bytes::Regex,
}

impl Replacement {
    /// Find and perform replacements in the given root path.
    pub(crate) fn replace_in(
        &self,
        root: &Path,
        group: &str,
        replacement: &str,
    ) -> Result<Vec<Replaced>> {
        let mut output = Vec::new();

        for path in &self.paths {
            let glob = Glob::new(root, path);

            for path in glob.matcher() {
                let path = path?;
                let output_path = path.to_path(root);

                let content = match fs::read(&output_path) {
                    Ok(content) => content,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        tracing::warn!("{path}: failed to read");
                        continue;
                    }
                    Err(e) => return Err(Error::from(e)).context(path),
                };

                let mut ranges = Vec::new();

                for cap in self.pattern.captures_iter(&content) {
                    if let Some(m) = cap.name(group)
                        && m.as_bytes() != replacement.as_bytes()
                    {
                        ranges.push(m.range());
                    }
                }

                if !ranges.is_empty() {
                    output.push(Replaced {
                        path: output_path,
                        content,
                        replacement: replacement.into(),
                        ranges,
                    });
                }
            }
        }

        Ok(output)
    }
}

/// Which operating system we are on.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Os {
    Windows,
    Linux,
    Mac,
    Other(String),
}

impl Os {
    /// Get the shell for the given operating system.
    pub(crate) fn shell(&self) -> Shell {
        match self {
            Os::Windows => Shell::Powershell,
            _ => Shell::Bash,
        }
    }

    /// Get the corresponding tree value.
    pub(crate) fn as_tree_value(&self) -> &str {
        match self {
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::Mac => "macOS",
            Self::Other(other) => other,
        }
    }
}

impl fmt::Display for Os {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Os::Windows => write!(f, "Windows"),
            Os::Linux => write!(f, "Linux"),
            Os::Mac => write!(f, "Mac"),
            Os::Other(other) => other.fmt(f),
        }
    }
}

/// Which distribution we are on.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Distribution {
    #[default]
    Other,
    Ubuntu,
    Debian,
    Fedora,
}

impl Distribution {
    /// Get the distribution from a string.
    pub(crate) fn from_string_ignore_case(string: impl AsRef<str>) -> Self {
        let string = string.as_ref();

        if string.eq_ignore_ascii_case("ubuntu") {
            return Distribution::Ubuntu;
        }

        if string.eq_ignore_ascii_case("debian") {
            return Distribution::Debian;
        }

        if string.eq_ignore_ascii_case("fedora") {
            return Distribution::Fedora;
        }

        Distribution::Other
    }

    /// Get the linux distribution we are currently on.
    pub(crate) fn linux_distribution() -> Option<Self> {
        let Ok(f) = File::open("/etc/os-release") else {
            tracing::trace!("No such file: /etc/os-release");
            return None;
        };

        let mut f = BufReader::new(f);
        let mut line = String::new();

        loop {
            line.clear();

            if f.read_line(&mut line).ok()? == 0 {
                break;
            }

            let Some((key, value)) = line.trim().split_once('=') else {
                continue;
            };

            if key.trim().eq_ignore_ascii_case("id") {
                return Some(Distribution::from_string_ignore_case(value.trim()));
            }
        }

        None
    }
}

impl fmt::Display for Distribution {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Distribution::Other => write!(f, "Other"),
            Distribution::Ubuntu => write!(f, "Ubuntu"),
            Distribution::Debian => write!(f, "Debian"),
            Distribution::Fedora => write!(f, "Fedora"),
        }
    }
}

#[derive(Default, Debug)]
pub(crate) struct RepoConfig {
    /// Sources for this repo.
    pub(crate) sources: BTreeSet<RepoSource>,
    /// Override crate to use.
    pub(crate) name: Option<String>,
    /// URLs to use.
    pub(crate) urls: BTreeSet<Url>,
    /// Supported operating system.
    pub(crate) os: BTreeSet<Os>,
    /// Name of the repo branch.
    pub(crate) branch: Option<String>,
    /// Filesystem workflows that have been found.
    pub(crate) filesystem_workflows: HashSet<String>,
    /// Workflows to incorporate.
    pub(crate) workflows: HashMap<String, PartialWorkflowConfig>,
    /// License of the project.
    pub(crate) license: Option<String>,
    /// Authors of the project.
    pub(crate) authors: Vec<String>,
    /// Documentation link of the project.
    pub(crate) documentation: Option<Template>,
    /// Custom lib template.
    pub(crate) lib: Option<Template>,
    /// Custom readme template.
    pub(crate) readme: Option<Template>,
    /// Custom badges for a specific project.
    pub(crate) badges: Vec<ConfigBadge>,
    /// Path to Cargo.toml to build.
    pub(crate) cargo_toml: Option<RelativePathBuf>,
    /// Disabled modules.
    pub(crate) disabled: BTreeSet<String>,
    /// Badges used in lib file.
    pub(crate) lib_badges: IdSet,
    /// Badges used in readmes.
    pub(crate) readme_badges: IdSet,
    /// Variables that can be used verbatim in templates.
    pub(crate) variables: toml::Table,
    /// Files to look for in replacements.
    pub(crate) version: Vec<Replacement>,
    /// Upgrade configuration.
    pub(crate) upgrade: Upgrade,
    /// RPM configuration.
    pub(crate) package: Package,
    /// Actions configuration.
    pub(crate) actions: Actions,
}

impl RepoConfig {
    /// Merge this config with another.
    pub(crate) fn merge_with(&mut self, mut other: Self) {
        self.name = other.name.or(self.name.take());
        self.os.extend(other.os);
        self.branch = other.branch.or(self.branch.take());

        for id in other.filesystem_workflows {
            self.filesystem_workflows.insert(id);
        }

        for (id, workflow) in other.workflows {
            self.workflows.entry(id).or_default().merge_with(workflow);
        }

        self.license = other.license.or(self.license.take());
        self.authors.append(&mut other.authors);
        self.documentation = other.documentation.or(self.documentation.take());
        self.lib = other.lib.or(self.lib.take());
        self.badges.append(&mut other.badges);
        self.cargo_toml = other.cargo_toml.or(self.cargo_toml.take());
        self.disabled.extend(other.disabled);
        self.lib_badges.merge_with(other.lib_badges);
        self.readme_badges.merge_with(other.readme_badges);
        self.version.extend(other.version);
        self.upgrade.merge_with(other.upgrade);
        self.package.merge_with(other.package);
        self.actions.merge_with(other.actions);

        merge_map(&mut self.variables, other.variables);
    }

    /// Test if this repo wants the specified readme badge.
    pub(crate) fn wants_lib_badge(&self, b: &ConfigBadge, enabled: bool) -> bool {
        let Some(id) = &b.id else {
            return enabled;
        };

        self.lib_badges.is_enabled(id, enabled)
    }

    /// Test if this repo wants the specified lib badge.
    pub(crate) fn wants_readme_badge(&self, b: &ConfigBadge, enabled: bool) -> bool {
        let Some(id) = &b.id else {
            return enabled;
        };

        self.readme_badges.is_enabled(id, enabled)
    }
}

/// A workflow configuration.
#[derive(Default, Debug, Clone)]
pub struct PartialWorkflowConfig {
    /// Workflow template.
    pub(crate) template: Option<Template>,
    /// The expected name of the workflow.
    pub(crate) name: Option<String>,
    /// Eanbled workflow features.
    pub(crate) features: HashSet<WorkflowFeature>,
    /// Branch that the workflow should trigger on.
    pub(crate) branch: Option<String>,
    /// If the workflow config is disabled.
    pub(crate) disable: Option<bool>,
}

impl PartialWorkflowConfig {
    fn merge_with(&mut self, other: Self) {
        self.template = other.template.or(self.template.take());
        self.name = other.name.or(self.name.take());
        self.features.extend(other.features);
        self.branch = other.branch.or(self.branch.take());
        self.disable = other.disable.or(self.disable.take());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WorkflowFeature {
    ScheduleRandomWeekly,
}

impl FromStr for WorkflowFeature {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "schedule-random-weekly" => Ok(WorkflowFeature::ScheduleRandomWeekly),
            other => bail!("Unknown workflow feature: {other}"),
        }
    }
}

/// A workflow configuration.
#[derive(Default, Debug, Clone)]
pub struct WorkflowConfig {
    /// The expected name of the workflow.
    pub(crate) name: Option<String>,
    /// Features enabled in workflow configuration.
    pub(crate) features: HashSet<WorkflowFeature>,
    /// Branch that the workflow should trigger on.
    pub(crate) branch: Option<String>,
    /// If the workflow configuration is disabled.
    pub(crate) disable: bool,
}

/// A badge configuration.
pub(crate) enum Id {
    /// An enabled badge.
    Enabled(String),
    /// A disabled badge.
    Disabled(String),
}

impl FromStr for Id {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut chars = s.chars();

        match (chars.next(), chars.as_str()) {
            (Some('-'), rest) if !rest.is_empty() => Ok(Id::Disabled(rest.to_owned())),
            (Some('+'), rest) if !rest.is_empty() => Ok(Id::Enabled(rest.to_owned())),
            (_, "") => Err(anyhow!("expected id after `+` and `-`")),
            _ => Err(anyhow!("expected `+` and `-` in id, but got `{s}`")),
        }
    }
}

/// Set of identifers.
#[derive(Debug, Default)]
pub(crate) struct IdSet {
    /// Explicit allowlist for badges to enabled which are already disabled.
    enabled: HashSet<String>,
    /// Explicit blocklist for badges to enabled.
    disabled: HashSet<String>,
}

impl IdSet {
    /// Merge with another set.
    pub(crate) fn merge_with(&mut self, other: Self) {
        self.enabled.extend(other.enabled);
        self.disabled.extend(other.disabled);
    }

    /// Test if an id is enabled.
    pub(crate) fn is_enabled(&self, id: &str, enabled: bool) -> bool {
        if enabled {
            !self.disabled.contains(id)
        } else {
            self.enabled.contains(id)
        }
    }
}

impl FromIterator<Id> for IdSet {
    #[inline]
    fn from_iter<T: IntoIterator<Item = Id>>(iter: T) -> Self {
        let mut enabled = HashSet::new();
        let mut disabled = HashSet::new();

        for badge in iter {
            match badge {
                Id::Enabled(badge) => {
                    enabled.insert(badge);
                }
                Id::Disabled(badge) => {
                    disabled.insert(badge);
                }
            }
        }

        Self { enabled, disabled }
    }
}

pub(crate) struct Config<'a> {
    pub(crate) base: RepoConfig,
    pub(crate) repos: BTreeMap<RelativePathBuf, RepoConfig>,
    pub(crate) defaults: &'a toml::Table,
}

impl Config<'_> {
    /// A workflow configuration.
    pub(crate) fn workflows(&self, repo: &RepoRef) -> Result<BTreeMap<String, WorkflowConfig>> {
        let ids = self.get_all(repo, |c| {
            c.filesystem_workflows.iter().chain(c.workflows.keys())
        });

        let mut out = BTreeMap::new();

        for id in ids {
            if out.contains_key(id) {
                continue;
            }

            let mut config = PartialWorkflowConfig::default();

            for repo in self.repos(repo) {
                if let Some(branch) = &repo.branch {
                    config.branch = Some(branch.clone());
                }

                if let Some(c) = repo.workflows.get(id) {
                    config.merge_with(c.clone());
                }
            }

            out.insert(
                id.to_owned(),
                WorkflowConfig {
                    name: config.name,
                    features: config.features,
                    branch: config.branch,
                    disable: config.disable.unwrap_or_default(),
                },
            );
        }

        Ok(out)
    }

    /// Generate a default workflow.
    pub(crate) fn workflow(
        &self,
        repo: &RepoRef,
        id: &str,
        params: RepoParams<'_>,
    ) -> Result<Option<String>> {
        for repo in self.repos(repo).rev() {
            if let Some(template) = repo.workflows.get(id).and_then(|r| r.template.as_ref()) {
                return Ok(Some(template.render(&params)?));
            }
        }

        Ok(None)
    }

    /// Get the current documentation template.
    pub(crate) fn documentation(&self, repo: &Repo) -> Option<&Template> {
        self.repos(repo)
            .rev()
            .flat_map(|r| r.documentation.as_ref())
            .next()
    }

    /// Get the current license template.
    pub(crate) fn license(&self, repo: &Repo) -> &str {
        self.repos(repo)
            .rev()
            .flat_map(|r| r.license.as_deref())
            .next()
            .unwrap_or(DEFAULT_LICENSE)
    }

    /// Get supported operating systems.
    pub(crate) fn os(&self, repo: &Repo) -> BTreeSet<&Os> {
        self.repos(repo).flat_map(|r| &r.os).collect()
    }

    /// Get the current license template.
    pub(crate) fn authors(&self, repo: &Repo) -> Vec<String> {
        self.repos(repo).flat_map(|r| &r.authors).cloned().collect()
    }

    /// Get the current license template.
    pub(crate) fn variables(&self, repo: &RepoRef) -> toml::Table {
        let mut variables = self.defaults.clone();

        for repo in self.repos(repo) {
            let mut current = repo.variables.clone();

            if let Some(branch) = &repo.branch {
                current.insert(
                    String::from("branch"),
                    toml::Value::String(branch.to_owned()),
                );
            }

            merge_map(&mut variables, current);
        }

        variables
    }

    /// Get all rpm files.
    pub(crate) fn package_files(&self, repo: &RepoRef) -> Vec<&PackageFile> {
        self.repos(repo).flat_map(|r| &r.package.files).collect()
    }

    /// Get all denied actions.
    pub(crate) fn action_deny(&self, repo: &RepoRef) -> Vec<&DenyAction> {
        self.repos(repo).flat_map(|r| &r.actions.deny).collect()
    }

    /// Get all latest actions.
    pub(crate) fn action_latest(&self, repo: &RepoRef) -> Vec<&LatestAction> {
        self.repos(repo).flat_map(|r| &r.actions.latest).collect()
    }

    /// Get all elements corresponding to the given field.
    pub(crate) fn get_all<'a, O: 'a, I>(
        &'a self,
        repo: &RepoRef,
        mut get: impl FnMut(&'a RepoConfig) -> I,
    ) -> impl Iterator<Item = &'a O>
    where
        I: IntoIterator<Item = &'a O>,
    {
        let a = get(&self.base).into_iter();
        a.chain(self.repos.get(repo.path()).into_iter().flat_map(get))
    }

    fn badges<F>(&self, path: &RelativePath, mut filter: F) -> impl Iterator<Item = &'_ ConfigBadge>
    where
        F: FnMut(&RepoConfig, &ConfigBadge, bool) -> bool,
    {
        let repo = self.repos.get(path);
        let repos = repo.into_iter().flat_map(|repo| repo.badges.iter());

        self.base.badges.iter().chain(repos).filter(move |b| {
            let enabled = filter(&self.base, b, b.enabled);
            repo.map(|repo| filter(repo, b, enabled)).unwrap_or(enabled)
        })
    }

    /// Iterator over lib badges for the given repo.
    pub(crate) fn lib_badges(&self, path: &RelativePath) -> impl Iterator<Item = &'_ ConfigBadge> {
        self.badges(path, |repo, b, enabled| repo.wants_lib_badge(b, enabled))
    }

    /// Iterator over readme badges for the given repo.
    pub(crate) fn readme_badges(
        &self,
        path: &RelativePath,
    ) -> impl Iterator<Item = &'_ ConfigBadge> {
        self.badges(path, |repo, b, enabled| repo.wants_readme_badge(b, enabled))
    }

    /// Get the header for the given repo.
    pub(crate) fn lib(&self, repo: &RepoRef) -> Option<&Template> {
        self.repos(repo).rev().flat_map(|r| r.lib.as_ref()).next()
    }

    /// Get readme template for the given repo.
    pub(crate) fn readme(&self, repo: &RepoRef) -> Option<&Template> {
        self.repos(repo)
            .rev()
            .flat_map(|r| r.readme.as_ref())
            .next()
    }

    /// Get crate for the given repo.
    pub(crate) fn name<'a>(&'a self, repo: &RepoRef) -> Option<&'a str> {
        self.repos(repo)
            .rev()
            .flat_map(|r| r.name.as_deref())
            .next()
    }

    /// Get Cargo.toml path for the given repo.
    pub(crate) fn cargo_toml<'a>(&'a self, repo: &RepoRef) -> Option<&'a RelativePath> {
        self.repos(repo)
            .rev()
            .flat_map(|r| r.cargo_toml.as_deref())
            .next()
    }

    /// Get Cargo.toml path for the given repo.
    pub(crate) fn is_enabled(&self, repo: &RepoRef, feature: &str) -> bool {
        let base = !self.base.disabled.contains(feature);

        let Some(repo) = self.repos.get(repo.path()) else {
            return base;
        };

        base && !repo.disabled.contains(feature)
    }

    /// Get version replacements.
    pub(crate) fn version<'a>(&'a self, repo: &RepoRef) -> Vec<&'a Replacement> {
        self.repos(repo).flat_map(|r| &r.version).collect()
    }

    /// Get crate for the given repo.
    pub(crate) fn upgrade(&self, repo: &RepoRef) -> Upgrade {
        let mut upgrade = Upgrade::default();

        for u in self.repos(repo).rev().map(|r| &r.upgrade) {
            upgrade.merge_with(u.clone());
        }

        upgrade
    }

    fn repos<'a>(&'a self, repo: &RepoRef) -> impl DoubleEndedIterator<Item = &'a RepoConfig> {
        [&self.base].into_iter().chain(self.repos.get(repo.path()))
    }
}

#[derive(Debug)]
pub(crate) struct ConfigBadge {
    pub(crate) id: Option<String>,
    enabled: bool,
    markdown: Option<Template>,
    html: Option<Template>,
}

impl ConfigBadge {
    pub(crate) fn markdown(&self, params: &RepoParams<'_>) -> Result<Option<String>> {
        let Some(template) = self.markdown.as_ref() else {
            return Ok(None);
        };

        Ok(Some(template.render(params)?))
    }

    pub(crate) fn html(&self, params: &RepoParams<'_>) -> Result<Option<String>> {
        let Some(template) = self.html.as_ref() else {
            return Ok(None);
        };

        Ok(Some(template.render(params)?))
    }
}

/// Context used when parsing configuration.
struct Cx<'a> {
    paths: Paths<'a>,
    current: RelativePathBuf,
    config_path: RelativePathBuf,
    keys: Keys,
    templating: &'a Templating,
    errors: RefCell<Vec<Error>>,
}

impl<'a> Cx<'a> {
    fn new(paths: Paths<'a>, current: &RelativePath, templating: &'a Templating) -> Self {
        Self {
            paths,
            current: current.to_owned(),
            config_path: current.join_normalized(KICK_TOML),
            keys: Keys::default(),
            templating,
            errors: RefCell::new(Vec::new()),
        }
    }

    /// Load the kick config.
    fn config(&self) -> Result<Option<toml::Value>, ErrorMarker> {
        let string = match self.paths.read_to_string(&self.config_path) {
            Ok(string) => string,
            Err(err) => return self.capture(err),
        };

        let Some(string) = string else {
            return Ok(None);
        };

        match toml::from_str(&string) {
            Ok(config) => Ok(Some(config)),
            Err(err) => self.capture(err),
        }
    }

    #[track_caller]
    fn report(&self, error: impl fmt::Display) {
        self.errors.borrow_mut().push(anyhow!(
            "{path}: {}: {error}",
            self.keys,
            path = self.paths.to_path(&self.config_path).display()
        ));
    }

    fn capture<O>(&self, error: impl fmt::Display) -> Result<O, ErrorMarker> {
        self.report(error);
        Err(ErrorMarker)
    }

    /// Visit the given key, extracting it from the specified table.
    fn require_in_key<O>(
        &self,
        table: &mut toml::Table,
        key: &str,
        f: impl FnOnce(&Self, toml::Value) -> Result<O, ErrorMarker>,
    ) -> Result<O, ErrorMarker> {
        let Some(value) = table.remove(key) else {
            return self.capture(format_args!("missing required key `{key}`"));
        };

        self.keys.field(key);
        let out = f(self, value);
        self.keys.pop();
        out
    }

    /// Visit the given key, extracting it from the specified table.
    fn in_key<O>(
        &self,
        table: &mut toml::Table,
        key: &str,
        f: impl FnOnce(&Self, toml::Value) -> Result<O, ErrorMarker>,
    ) -> Result<Option<O>, ErrorMarker> {
        let Some(value) = table.remove(key) else {
            return Ok(None);
        };

        self.keys.field(key);
        let out = f(self, value);
        self.keys.pop();
        Ok(Some(out?))
    }

    fn parse<O>(&self, value: toml::Value) -> Result<O, ErrorMarker>
    where
        O: FromStr<Err: fmt::Display>,
    {
        let value = self.string(value)?;

        match O::from_str(&value) {
            Ok(feature) => Ok(feature),
            Err(e) => self.capture(e),
        }
    }

    /// Compile a template from a path.
    fn compile_path(&self, value: toml::Value) -> Result<Template, ErrorMarker> {
        let path = self.relative_path(value)?;
        let path = self.current.join(path).to_path(self.paths.root);

        let template = match fs::read_to_string(&path).with_context(|| path.display().to_string()) {
            Ok(template) => template,
            Err(err) => return self.capture(err),
        };

        self.compile_str(template)
    }

    /// Compile a template.
    fn compile(&self, value: toml::Value) -> Result<Template, ErrorMarker> {
        let source = self.string(value)?;
        self.compile_str(source)
    }

    /// Compile a template from a string.
    fn compile_str(&self, source: impl AsRef<str>) -> Result<Template, ErrorMarker> {
        match self.templating.compile(source.as_ref()) {
            Ok(template) => Ok(template),
            Err(err) => self.capture(err),
        }
    }

    fn string(&self, value: toml::Value) -> Result<String, ErrorMarker> {
        match value {
            toml::Value::String(string) => Ok(string),
            other => self.capture(format_args!("expected string, got {}", other.type_str())),
        }
    }

    fn relative_path(&self, value: toml::Value) -> Result<RelativePathBuf, ErrorMarker> {
        let path = RelativePathBuf::from(self.string(value)?);

        if path.as_str().starts_with('/') {
            return self.capture(format_args!("path must be relative, but got {path}"));
        }

        Ok(path)
    }

    fn boolean(&self, value: toml::Value) -> Result<bool, ErrorMarker> {
        match value {
            toml::Value::Boolean(value) => Ok(value),
            other => self.capture(format_args!("expected boolean, got {}", other.type_str())),
        }
    }

    fn table(&self, value: toml::Value) -> Result<toml::Table, ErrorMarker> {
        match value {
            toml::Value::Table(table) => Ok(table),
            other => self.capture(format_args!("expected table, got {}", other.type_str())),
        }
    }

    fn in_array<O, B>(
        &self,
        table: &mut toml::Table,
        key: &str,
        map: Option<(&str, &str)>,
        mut f: impl FnMut(&Self, toml::Value) -> Result<O, ErrorMarker>,
    ) -> B
    where
        B: FromIterator<O>,
    {
        let result = self.in_key(table, key, move |cx, value| match (value, map) {
            (toml::Value::Array(array), _) => {
                let it = array.into_iter().enumerate().flat_map(|(index, item)| {
                    cx.keys.index(index);
                    let out = f(cx, item).ok();
                    cx.keys.pop();
                    out
                });

                Ok(it.collect())
            }
            (toml::Value::Table(table), Some((key, value))) => {
                let it = table.into_iter().enumerate().flat_map(|(index, (k, v))| {
                    let mut table = toml::Table::with_capacity(2);
                    table.insert(key.to_owned(), toml::Value::String(k));
                    table.insert(value.to_owned(), v);

                    cx.keys.index(index);
                    let out = f(cx, toml::Value::Table(table)).ok();
                    cx.keys.pop();
                    out
                });

                Ok(it.collect())
            }
            (other, Some((key, value))) => self.capture(format_args!(
                "expected array or map {{{key} => {value}}}, got {}",
                other.type_str()
            )),
            (other, None) => self.capture(format_args!("expected array, got {}", other.type_str())),
        });

        match result {
            Ok(Some(result)) => result,
            _ => B::from_iter(iter::empty()),
        }
    }

    fn in_table<K, V, O>(
        &self,
        table: &mut toml::Table,
        key: &str,
        mut f: impl FnMut(&Self, String, toml::Value) -> Result<(K, V), ErrorMarker>,
    ) -> O
    where
        K: Eq + Hash,
        O: FromIterator<(K, V)>,
    {
        let out = self.in_key(table, key, move |cx, value| {
            let table = cx.table(value)?;

            let data = O::from_iter(table.into_iter().flat_map(|(key, item)| {
                cx.keys.field(&key);
                let out = f(cx, key, item).ok();
                cx.keys.pop();
                out
            }));

            Ok(data)
        });

        match out {
            Ok(Some(out)) => out,
            _ => O::from_iter(iter::empty()),
        }
    }

    fn as_table<F, O>(
        &self,
        table: &mut toml::Table,
        key: &str,
        f: F,
    ) -> Result<Option<O>, ErrorMarker>
    where
        F: FnOnce(&Self, toml::Table) -> Result<O, ErrorMarker>,
    {
        self.in_key(table, key, move |cx, value| {
            let table = cx.table(value)?;
            f(cx, table)
        })
    }

    fn require_key<O>(
        &self,
        table: &mut toml::Table,
        key: &str,
        f: impl FnOnce(&Self, toml::Value) -> Result<O, ErrorMarker>,
    ) -> Result<O, ErrorMarker> {
        let Some(value) = table.remove(key) else {
            return self.capture(format_args!("missing required key `{key}`"));
        };

        self.keys.field(key);
        let out = f(self, value);
        self.keys.pop();
        out
    }

    fn workflow_table(
        &self,
        table: &mut toml::Table,
    ) -> Result<PartialWorkflowConfig, ErrorMarker> {
        let name = self.in_key(table, "name", Self::string);
        let template = self.in_key(table, "template", Self::compile_path);
        let features = self.in_array(table, "features", None, Self::parse);
        let branch = self.in_key(table, "branch", Self::string);
        let disable = self.in_key(table, "disable", Self::boolean);

        Ok(PartialWorkflowConfig {
            name: name?,
            template: template?,
            features,
            branch: branch?,
            disable: disable?,
        })
    }

    fn repo_table(&self, table: &mut toml::Table) -> Result<RepoConfig, ErrorMarker> {
        let name = self.in_key(table, "name", Self::string);
        let url = self.in_key(table, "url", Self::parse);

        let os = self.in_array(table, "os", None, |cx, item| {
            match cx.string(item)?.as_str() {
                "windows" => Ok(Os::Windows),
                "linux" => Ok(Os::Linux),
                "macos" => Ok(Os::Mac),
                other => cx.capture(format_args!("unknown os: {other}")),
            }
        });

        let branch = self.in_key(table, "branch", Self::string);
        let workflows = self.in_table(table, "workflows", |cx, id, value| {
            Ok((id, cx.workflow(value)?))
        });
        let license = self.in_key(table, "license", Self::string);
        let authors = self.in_array(table, "authors", None, Self::string);
        let documentation = self.in_key(table, "documentation", Self::compile);
        let lib = self.in_key(table, "lib", Self::compile_path);
        let readme = self.in_key(table, "readme", Self::compile_path);

        let badges = self.in_array(table, "badges", None, |cx, value| {
            cx.with_table(value, |cx, table| {
                let id = cx.in_key(table, "id", Self::string);
                let alt = cx.in_key(table, "alt", Self::string);
                let src = cx.in_key(table, "src", Self::string);
                let href = cx.in_key(table, "href", Self::string);
                let height = cx.in_key(table, "height", Self::string);
                let enabled = cx.in_key(table, "enabled", Self::boolean);

                let (markdown, html) =
                    if let (Ok(alt), Ok(Some(src)), Ok(Some(href)), Ok(Some(height))) =
                        (alt, src, href, height)
                    {
                        let alt =
                            FormatOptional(alt.as_ref(), |f, alt| write!(f, " alt=\"{alt}\""));

                        let markdown = cx.compile_str(format!(
                            "[<img{alt} src=\"{src}\" height=\"{height}\">]({href})"
                        ));

                        let html = cx.compile_str(format!(
                            "<a href=\"{href}\"><img{alt} src=\"{src}\" height=\"{height}\"></a>"
                        ));

                        (Some(markdown), Some(html))
                    } else {
                        (None, None)
                    };

                Ok(ConfigBadge {
                    id: id?,
                    enabled: enabled?.unwrap_or(true),
                    markdown: markdown.transpose()?,
                    html: html.transpose()?,
                })
            })
        });

        let lib_badges = self.in_array(table, "lib_badges", None, Self::parse);
        let readme_badges = self.in_array(table, "readme_badges", None, Self::parse);

        let variables = self.as_table(table, "variables", |_, table| Ok(table));

        let version = self.in_array(table, "version", None, |cx, value| {
            cx.with_table(value, |cx, table| {
                let package_name = cx.in_key(table, "crate", Self::string);
                let paths = cx.in_array(table, "paths", None, Self::relative_path);
                let pattern = cx.require_in_key(table, "pattern", Self::parse);

                Ok(Replacement {
                    package_name: package_name?,
                    paths,
                    pattern: pattern?,
                })
            })
        });

        let cargo_toml = self.in_key(table, "cargo_toml", Self::relative_path);

        let upgrade = self.in_key(table, "upgrade", Self::upgrade);

        let disabled = self.in_array(table, "disabled", None, Self::string);

        let package = self.in_key(table, "package", Self::package);

        let actions = self.in_key(table, "actions", Self::actions);

        Ok(RepoConfig {
            sources: BTreeSet::from_iter([RepoSource::Config(self.current.to_owned())]),
            name: name?,
            urls: BTreeSet::from_iter(url?),
            os,
            branch: branch?,
            filesystem_workflows: HashSet::new(),
            workflows,
            license: license?,
            authors,
            documentation: documentation?,
            lib: lib?,
            readme: readme?,
            badges,
            cargo_toml: cargo_toml?,
            disabled,
            lib_badges,
            readme_badges,
            variables: variables?.unwrap_or_default(),
            version,
            upgrade: upgrade?.unwrap_or_default(),
            package: package?.unwrap_or_default(),
            actions: actions?.unwrap_or_default(),
        })
    }

    fn with_table<F, O>(&self, value: toml::Value, f: F) -> Result<O, ErrorMarker>
    where
        F: FnOnce(&Self, &mut toml::Table) -> Result<O, ErrorMarker>,
    {
        let mut config = self.table(value)?;
        let out = f(self, &mut config);

        if !config.is_empty() {
            let keys = config.into_iter().map(|(key, _)| key).collect::<Vec<_>>();

            let what = match &keys[..] {
                [_] => "key",
                _ => "keys",
            };

            let keys = keys.join(", ");
            return self.capture(format_args!("got unsupported {what}: {keys}"));
        }

        out
    }

    fn repo(&self, value: toml::Value) -> Result<RepoConfig, ErrorMarker> {
        self.with_table(value, Self::repo_table)
    }

    fn workflow(&self, value: toml::Value) -> Result<PartialWorkflowConfig, ErrorMarker> {
        self.with_table(value, Self::workflow_table)
    }

    fn upgrade(&self, value: toml::Value) -> Result<Upgrade, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let exclude = cx.in_array(table, "exclude", None, Self::string);
            Ok(Upgrade { exclude })
        })
    }

    fn package_file(&self, value: toml::Value) -> Result<PackageFile, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let source = cx.require_key(table, "source", Self::relative_path);
            let dest = cx.require_key(table, "dest", Self::relative_path);

            let mode = cx.in_key(table, "mode", |cx, string| {
                let string = cx.string(string)?;

                match u16::from_str_radix(&string, 8) {
                    Ok(mode) => Ok(mode),
                    Err(err) => cx.capture(format_args!("invalid file mode `{string}`: {err}")),
                }
            });

            Ok(PackageFile {
                source: source?,
                dest: dest?,
                mode: mode?,
            })
        })
    }

    fn rpm_require(&self, value: toml::Value) -> Result<RpmRequire, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let package = cx.require_key(table, "package", Self::string);
            let version = cx.in_key(table, "version", Self::parse);

            Ok(RpmRequire {
                package: package?,
                version: version?.unwrap_or_default(),
            })
        })
    }

    fn deb_dependency(&self, value: toml::Value) -> Result<DebDependency, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let package = cx.require_key(table, "package", Self::string);
            let version = cx.in_key(table, "version", Self::parse);

            Ok(DebDependency {
                package: package?,
                version: version?.unwrap_or_default(),
            })
        })
    }

    fn rpm(&self, value: toml::Value) -> Result<RpmPackage, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let requires = cx.in_array(
                table,
                "requires",
                Some(("package", "version")),
                Self::rpm_require,
            );

            Ok(RpmPackage { requires })
        })
    }

    fn deb(&self, value: toml::Value) -> Result<DebPackage, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let depends = cx.in_array(
                table,
                "depends",
                Some(("package", "version")),
                Self::deb_dependency,
            );

            Ok(DebPackage { depends })
        })
    }

    fn package(&self, value: toml::Value) -> Result<Package, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let files = cx.in_array(table, "files", None, Self::package_file);
            let rpm = cx.in_key(table, "rpm", Self::rpm);
            let deb = cx.in_key(table, "deb", Self::deb);

            Ok(Package {
                files,
                rpm: rpm?.unwrap_or_default(),
                deb: deb?.unwrap_or_default(),
            })
        })
    }

    fn deny_action(&self, value: toml::Value) -> Result<DenyAction, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let name = cx.require_in_key(table, "name", Self::string);
            let reason = cx.in_key(table, "reason", Self::string);

            Ok(DenyAction {
                name: name?,
                reason: reason?,
            })
        })
    }

    fn latest_action(&self, value: toml::Value) -> Result<LatestAction, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let name = cx.require_in_key(table, "name", Self::string);
            let version = cx.require_in_key(table, "version", Self::string);

            Ok(LatestAction {
                name: name?,
                version: version?,
            })
        })
    }

    fn actions(&self, value: toml::Value) -> Result<Actions, ErrorMarker> {
        self.with_table(value, |cx, table| {
            let deny = cx.in_array(table, "deny", Some(("name", "reason")), Self::deny_action);

            let latest = cx.in_array(
                table,
                "latest",
                Some(("name", "version")),
                Self::latest_action,
            );

            Ok(Actions { deny, latest })
        })
    }
}

/// Load a configuration from the given path.
pub(crate) fn load<'a>(
    paths: Paths<'a>,
    templating: &Templating,
    extra_repos: impl IntoIterator<Item = (RelativePathBuf, RepoInfo)>,
    defaults: &'a toml::Table,
) -> Result<Config<'a>> {
    fn from_config(c: &RepoConfig) -> RepoInfo {
        let mut repo = RepoInfo::default();
        repo.urls.extend(c.urls.clone());
        repo.sources.extend(c.sources.iter().cloned());
        repo
    }

    let mut cx = Cx::new(paths, RelativePath::new(""), templating);

    let (base, mut repos) = 'out: {
        let Ok(Some(config)) = cx.config() else {
            break 'out (RepoConfig::default(), BTreeMap::new());
        };

        let Ok((base, repos)) = load_base(&mut cx, config) else {
            break 'out (RepoConfig::default(), BTreeMap::new());
        };

        (base, repos)
    };

    let mut infos = BTreeMap::from_iter(
        repos
            .iter()
            .map(|(path, config)| (path.to_owned(), from_config(config))),
    );

    for (path, info) in extra_repos {
        let to = infos.entry(path).or_default();
        to.urls.extend(info.urls);
        to.sources.extend(info.sources);
    }

    for (path, info) in infos {
        let path = cx.current.join(&path);
        let updates = load_repo(&mut cx, path.clone());
        let config = repos.entry(path).or_default();

        if let Ok(updates) = updates {
            config.merge_with(updates);
        }

        config.sources.extend(info.sources);
        config.urls.extend(info.urls);
    }

    let errors = cx.errors.into_inner();

    if !errors.is_empty() {
        let count = errors.len();

        for error in errors {
            tracing::error!("Error: {error}");

            for e in error.chain().skip(1) {
                tracing::error!("  Caused by: {e}");
            }
        }

        let what = match count {
            1 => "error",
            _ => "errors",
        };

        return Err(anyhow!(
            "{}: Failed to load configuration due to {count} {what}",
            cx.config_path
        ));
    }

    Ok(Config {
        base,
        repos,
        defaults,
    })
}

fn load_base(
    cx: &mut Cx<'_>,
    table: toml::Value,
) -> Result<(RepoConfig, BTreeMap<RelativePathBuf, RepoConfig>), ErrorMarker> {
    cx.with_table(table, |cx, table| {
        let base = cx.repo_table(table);

        let repos = cx.in_table(table, "repo", |cx, id, value| {
            Ok((RelativePathBuf::from(id), cx.repo(value)?))
        });

        Ok((base?, repos))
    })
}

fn load_repo(cx: &mut Cx, current: RelativePathBuf) -> Result<RepoConfig, ErrorMarker> {
    let old_config_path = mem::replace(&mut cx.config_path, current.join(KICK_TOML));
    let old_current = mem::replace(&mut cx.current, current);

    let out = match cx.config() {
        Ok(config) => match config {
            Some(config) => cx.repo(config),
            None => Ok(RepoConfig::default()),
        },
        Err(error) => Err(error),
    };

    cx.config_path = old_config_path;
    cx.current = old_current;
    out
}

struct FormatOptional<T, F>(Option<T>, F)
where
    F: Fn(&mut fmt::Formatter<'_>, &T) -> fmt::Result;

impl<T, F> fmt::Display for FormatOptional<T, F>
where
    T: fmt::Display,
    F: Fn(&mut fmt::Formatter<'_>, &T) -> fmt::Result,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(value) = &self.0 {
            (self.1)(f, value)?;
        }

        Ok(())
    }
}

fn merge_map(target: &mut toml::Table, source: toml::Table) {
    for (key, value) in source {
        match target.entry(key) {
            toml::map::Entry::Vacant(e) => {
                e.insert(value);
            }
            toml::map::Entry::Occupied(e) => match (e.into_mut(), value) {
                (toml::Value::Table(target), toml::Value::Table(source)) => {
                    merge_map(target, source);
                }
                (toml::Value::Array(target), toml::Value::Array(source)) => {
                    target.extend(source);
                }
                (target, source) => {
                    *target = source;
                }
            },
        }
    }
}

/// Access `rpm.requies` through [`Config::get_all`].
pub(crate) fn rpm_requires(config: &RepoConfig) -> &[RpmRequire] {
    &config.package.rpm.requires
}

/// Access `deb.depends` through [`Config::get_all`].
pub(crate) fn deb_depends(config: &RepoConfig) -> &[DebDependency] {
    &config.package.deb.depends
}
