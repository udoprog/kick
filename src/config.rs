use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::hash::Hash;
use std::io::{self, Write};
use std::iter;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Error, Result};
use musli::{Decode, Encode};
use relative_path::{RelativePath, RelativePathBuf};
use semver::Version;
use tempfile::NamedTempFile;

use crate::ctxt::Paths;
use crate::glob::Glob;
use crate::keys::Keys;
use crate::model::{Repo, RepoParams, RepoRef};
use crate::shell::Shell;
use crate::templates::{Template, Templating};
use crate::KICK_TOML;

/// Default job name.
const DEFAULT_CI_NAME: &str = "CI";
/// Default weekly name.
const DEFAULT_WEEKLY_NAME: &str = "Weekly";
/// Default license to use in configuration.
const DEFAULT_LICENSE: &str = "MIT/Apache-2.0";

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
    type Err = anyhow::Error;

    #[inline]
    fn from_str(string: &str) -> Result<Self, Self::Err> {
        if string == "*" {
            return Ok(VersionRequirement::Any);
        }

        let Some((op, version)) = string.split_once(' ') else {
            return Err(anyhow!("Illegal version specification: {string}"));
        };

        let op = match op {
            "<" => VersionConstraint::Lt,
            "<=" => VersionConstraint::Le,
            "=" => VersionConstraint::Eq,
            ">=" => VersionConstraint::Ge,
            ">" => VersionConstraint::Gt,
            version => return Err(anyhow!("Illegal version constraint: {version}")),
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
                    if let Some(m) = cap.name(group) {
                        if m.as_bytes() != replacement.as_bytes() {
                            ranges.push(m.range());
                        }
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

#[derive(Default, Debug)]
pub(crate) struct RepoConfig {
    /// Override crate to use.
    pub(crate) name: Option<String>,
    /// Supported operating system.
    pub(crate) os: BTreeSet<Os>,
    /// Name of the repo branch.
    pub(crate) branch: Option<String>,
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

impl WorkflowFeature {
    fn parse(s: &str) -> Result<Self> {
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

impl Id {
    /// Parse a single badge.
    fn parse<S>(item: S) -> Result<Self>
    where
        S: AsRef<str>,
    {
        let item = item.as_ref();
        let mut chars = item.chars();

        match (chars.next(), chars.as_str()) {
            (Some('-'), rest) => Ok(Id::Disabled(rest.to_owned())),
            (Some('+'), rest) => Ok(Id::Enabled(rest.to_owned())),
            _ => Err(anyhow!("expected `+` and `-` in badge, but got `{item}`")),
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
    pub(crate) repos: HashMap<RelativePathBuf, RepoConfig>,
    pub(crate) defaults: &'a toml::Table,
}

impl Config<'_> {
    /// A workflow configuration.
    pub(crate) fn workflows(&self, repo: &RepoRef) -> Result<BTreeMap<String, WorkflowConfig>> {
        let mut ids = HashSet::new();

        for id in self.base.workflows.keys() {
            ids.insert(id.as_str());
        }

        if let Some(repo) = self.repos.get(repo.path()) {
            for id in repo.workflows.keys() {
                ids.insert(id.as_str());
            }
        }

        let mut out = BTreeMap::new();

        for id in ids {
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
    pub(crate) fn get_all<'a, G, O>(&'a self, repo: &RepoRef, get: G) -> Vec<&'a O>
    where
        G: Fn(&'a RepoConfig) -> &'a [O],
    {
        let mut requires = get(&self.base).iter().collect::<Vec<_>>();

        if let Some(values) = self.repos.get(repo.path()).map(get) {
            requires.extend(values);
        }

        requires
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
struct ConfigCtxt<'a> {
    paths: Paths<'a>,
    current: &'a RelativePath,
    kick_path: RelativePathBuf,
    keys: Keys,
    templating: &'a Templating,
}

impl<'a> ConfigCtxt<'a> {
    fn new(paths: Paths<'a>, current: &'a RelativePath, templating: &'a Templating) -> Self {
        Self {
            paths,
            current,
            kick_path: current.join_normalized(KICK_TOML),
            keys: Keys::default(),
            templating,
        }
    }

    /// Load the kick config.
    fn kick_config(&self) -> Result<Option<toml::Value>> {
        let Some(string) = self.paths.read_to_string(&self.kick_path)? else {
            return Ok(None);
        };

        let config = toml::from_str(&string).with_context(|| self.kick_path.clone())?;
        Ok(Some(config))
    }

    fn context<E>(&self, error: E) -> anyhow::Error
    where
        anyhow::Error: From<E>,
    {
        anyhow::Error::from(error).context(anyhow!(
            "In {path}: {}",
            self.keys,
            path = self.paths.to_path(&self.kick_path).display()
        ))
    }

    /// Visit the given key, extracting it from the specified table.
    fn in_key<F, O>(&mut self, config: &mut toml::Table, key: &str, f: F) -> Result<Option<O>>
    where
        F: FnOnce(&mut Self, toml::Value) -> Result<O>,
    {
        let Some(value) = config.remove(key) else {
            return Ok(None);
        };

        self.keys.field(key);
        let out = f(self, value)?;
        self.keys.pop();
        Ok(Some(out))
    }

    /// Compile a template from a path.
    fn compile_path<S>(&mut self, path: S) -> Result<Template>
    where
        S: AsRef<str>,
    {
        let path = self.current.join(path.as_ref()).to_path(self.paths.root);
        let template = fs::read_to_string(&path).with_context(|| path.display().to_string())?;
        self.compile(template)
    }

    /// Compile a template.
    fn compile<S>(&mut self, source: S) -> Result<Template>
    where
        S: AsRef<str>,
    {
        self.templating.compile(source.as_ref())
    }

    fn string(&mut self, value: toml::Value) -> Result<String> {
        match value {
            toml::Value::String(string) => Ok(string),
            other => Err(anyhow!("Expected string, got {other}")),
        }
    }

    fn relative_path(&mut self, value: toml::Value) -> Result<RelativePathBuf> {
        let string = self.string(value)?;
        let path = RelativePathBuf::from(string);

        if path.as_str().starts_with('/') {
            bail!("path must be relative, but got {path}");
        }

        Ok(path)
    }

    fn boolean(&mut self, value: toml::Value) -> Result<bool> {
        match value {
            toml::Value::Boolean(value) => Ok(value),
            other => Err(anyhow!("Expected boolean, got {other}")),
        }
    }

    fn array(&mut self, value: toml::Value, map: Option<(&str, &str)>) -> Result<Vec<toml::Value>> {
        match (value, map) {
            (toml::Value::Array(array), _) => Ok(array),
            (toml::Value::Table(table), Some((key, value))) => {
                let mut array = Vec::new();

                for (k, v) in table {
                    let mut table = toml::Table::new();
                    table.insert(key.to_owned(), toml::Value::String(k));
                    table.insert(value.to_owned(), v);
                    array.push(toml::Value::Table(table));
                }

                Ok(array)
            }
            (other, Some((key, value))) => Err(anyhow!(
                "Expected array or map {{{key} => {value}}}, got {other}"
            )),
            (other, None) => Err(anyhow!("Expected array, got {other}")),
        }
    }

    fn table(&mut self, value: toml::Value) -> Result<toml::Table> {
        match value {
            toml::Value::Table(table) => Ok(table),
            other => Err(anyhow!("Expected table, got {other}")),
        }
    }

    fn in_array<F, O, B>(
        &mut self,
        config: &mut toml::Table,
        key: &str,
        map: Option<(&str, &str)>,
        mut f: F,
    ) -> Result<B>
    where
        F: FnMut(&mut Self, toml::Value) -> Result<O>,
        B: FromIterator<O>,
    {
        let result = self.in_key(config, key, move |cx, value| {
            let array = cx.array(value, map)?;
            let mut it = array.into_iter().enumerate();

            let it = iter::from_fn(|| {
                let (index, item) = it.next()?;

                cx.keys.index(index);

                let value = match f(cx, item) {
                    Ok(value) => value,
                    Err(error) => {
                        return Some(Err(error));
                    }
                };

                cx.keys.pop();
                Some(Ok(value))
            });

            it.collect()
        })?;

        match result {
            Some(result) => Ok(result),
            None => Ok(B::from_iter(iter::empty())),
        }
    }

    fn in_table<F, K, V>(
        &mut self,
        config: &mut toml::Table,
        key: &str,
        mut f: F,
    ) -> Result<Option<HashMap<K, V>>>
    where
        K: Eq + Hash,
        F: FnMut(&mut Self, String, toml::Value) -> Result<(K, V)>,
    {
        self.in_key(config, key, move |cx, value| {
            let table = cx.table(value)?;
            let mut out = HashMap::with_capacity(table.len());

            for (key, item) in table {
                cx.keys.field(&key);
                let (key, value) = f(cx, key, item)?;
                out.insert(key, value);
                cx.keys.pop();
            }

            Ok(out)
        })
    }

    fn as_table<F, O>(&mut self, config: &mut toml::Table, key: &str, f: F) -> Result<Option<O>>
    where
        F: FnOnce(&mut Self, toml::Table) -> Result<O>,
    {
        self.in_key(config, key, move |cx, value| {
            let table = cx.table(value)?;
            f(cx, table)
        })
    }

    fn in_string<F, O>(&mut self, config: &mut toml::Table, key: &str, f: F) -> Result<Option<O>>
    where
        F: FnOnce(&mut Self, String) -> Result<O>,
    {
        self.in_key(config, key, move |cx, value| {
            let string = cx.string(value)?;
            f(cx, string)
        })
    }

    fn workflow_table(cx: &mut TableContext<'a, '_>) -> Result<PartialWorkflowConfig> {
        Ok(PartialWorkflowConfig {
            name: cx.as_string("name")?,
            template: cx.in_string("template", Self::compile_path)?,
            features: cx.in_array("features", None, |cx, value| {
                let value = cx.string(value)?;
                WorkflowFeature::parse(&value)
            })?,
            branch: cx.as_string("branch")?,
            disable: cx.as_boolean("disable")?,
        })
    }

    fn repo_table(cx: &mut TableContext<'a, '_>) -> Result<RepoConfig> {
        let badges = cx.in_array("badges", None, |cx, value| {
            cx.with_table(value, |cx| {
                let id = cx.as_string("id")?;
                let alt = cx.as_string("alt")?;
                let src = cx.as_string("src")?;
                let href = cx.as_string("href")?;
                let height = cx.as_string("height")?;
                let enabled = cx.as_boolean("enabled")?.unwrap_or(true);

                let alt = FormatOptional(alt.as_ref(), |f, alt| write!(f, " alt=\"{alt}\""));

                let (markdown, html) =
                    if let (Some(src), Some(href), Some(height)) = (src, href, height) {
                        let markdown = cx.cx.compile(format!(
                            "[<img{alt} src=\"{src}\" height=\"{height}\">]({href})"
                        ))?;
                        let html = cx.cx.compile(format!(
                            "<a href=\"{href}\"><img{alt} src=\"{src}\" height=\"{height}\"></a>"
                        ))?;
                        (Some(markdown), Some(html))
                    } else {
                        (None, None)
                    };

                Ok(ConfigBadge {
                    id,
                    enabled,
                    markdown,
                    html,
                })
            })
        })?;

        let lib_badges = cx.in_array("lib_badges", None, |cx, item| Id::parse(cx.string(item)?))?;
        let readme_badges = cx.in_array("readme_badges", None, |cx, item| {
            Id::parse(cx.string(item)?)
        })?;

        let variables = cx
            .as_table("variables", |_, table| Ok(table))?
            .unwrap_or_default();

        let version = cx.in_array("version", None, |cx, value| {
            cx.with_table(value, |cx| {
                let package_name = cx.as_string("crate")?;

                let paths = cx.in_array("paths", None, Self::relative_path)?;

                let pattern = cx
                    .in_string("pattern", |_, pattern| {
                        Ok(regex::bytes::Regex::new(&pattern)?)
                    })?
                    .context("Missing `pattern`")?;

                Ok(Replacement {
                    package_name,
                    paths,
                    pattern,
                })
            })
        })?;

        let os = cx.in_array("os", None, |cx, item| match cx.string(item)?.as_str() {
            "windows" => Ok(Os::Windows),
            "linux" => Ok(Os::Linux),
            "macos" => Ok(Os::Mac),
            other => Err(anyhow!("Unknown os: {other}")),
        })?;

        Ok(RepoConfig {
            name: cx.as_string("name")?,
            os,
            branch: cx.as_string("branch")?,
            workflows: cx
                .in_table("workflows", |cx, id, value| Ok((id, cx.workflow(value)?)))?
                .unwrap_or_default(),
            license: cx.in_string("license", |_, string| Ok(string))?,
            authors: cx.in_array("authors", None, Self::string)?,
            documentation: cx.in_string("documentation", Self::compile)?,
            lib: cx.in_string("lib", Self::compile_path)?,
            readme: cx.in_string("readme", Self::compile_path)?,
            badges,
            cargo_toml: cx.as_relative_path("cargo_toml")?,
            disabled: cx.in_array("disabled", None, Self::string)?,
            lib_badges,
            readme_badges,
            variables,
            version,
            upgrade: cx.in_key("upgrade", Self::upgrade)?.unwrap_or_default(),
            package: cx.in_key("package", Self::package)?.unwrap_or_default(),
            actions: cx.in_key("actions", Self::actions)?.unwrap_or_default(),
        })
    }

    fn with_table<F, O>(&mut self, config: toml::Value, f: F) -> Result<O>
    where
        F: FnOnce(&mut TableContext<'a, '_>) -> Result<O>,
    {
        let mut config = self.table(config)?;

        let mut cx = TableContext {
            cx: self,
            config: &mut config,
        };

        let out = f(&mut cx)?;

        if !config.is_empty() {
            let keys = config.into_iter().map(|(key, _)| key).collect::<Vec<_>>();
            let keys = keys.join(", ");
            bail!("{}: got unsupported keys `{keys}`", self.keys);
        }

        Ok(out)
    }

    fn repo(&mut self, config: toml::Value) -> Result<RepoConfig> {
        self.with_table(config, Self::repo_table)
    }

    fn workflow(&mut self, config: toml::Value) -> Result<PartialWorkflowConfig> {
        self.with_table(config, Self::workflow_table)
    }

    fn upgrade(&mut self, value: toml::Value) -> Result<Upgrade> {
        self.with_table(value, |cx| {
            Ok(Upgrade {
                exclude: cx.in_array("exclude", None, Self::string)?,
            })
        })
    }

    fn package_file(&mut self, value: toml::Value) -> Result<PackageFile> {
        self.with_table(value, |cx| {
            let Some(source) = cx.as_relative_path("source")? else {
                bail!("Missing source");
            };

            let Some(dest) = cx.as_relative_path("dest")? else {
                bail!("Missing dest");
            };

            Ok(PackageFile {
                source,
                dest,
                mode: cx.in_string("mode", |_, string| Ok(u16::from_str_radix(&string, 8)?))?,
            })
        })
    }

    fn version_requirement(&mut self, string: String) -> Result<VersionRequirement> {
        VersionRequirement::from_str(&string)
    }

    fn rpm_require(&mut self, value: toml::Value) -> Result<RpmRequire> {
        self.with_table(value, |cx| {
            let Some(package) = cx.as_string("package")? else {
                bail!("Missing package");
            };

            let version = cx
                .in_string("version", Self::version_requirement)?
                .unwrap_or_default();

            Ok(RpmRequire { package, version })
        })
    }

    fn deb_dependency(&mut self, value: toml::Value) -> Result<DebDependency> {
        self.with_table(value, |cx| {
            let Some(package) = cx.in_string("package", |_, string| Ok(string))? else {
                bail!("Missing package");
            };

            let version = cx
                .in_string("version", Self::version_requirement)?
                .unwrap_or_default();

            Ok(DebDependency { package, version })
        })
    }

    fn rpm(&mut self, value: toml::Value) -> Result<RpmPackage> {
        self.with_table(value, |cx| {
            Ok(RpmPackage {
                requires: cx.in_array(
                    "requires",
                    Some(("package", "version")),
                    Self::rpm_require,
                )?,
            })
        })
    }

    fn deb(&mut self, value: toml::Value) -> Result<DebPackage> {
        self.with_table(value, |cx| {
            Ok(DebPackage {
                depends: cx.in_array(
                    "depends",
                    Some(("package", "version")),
                    Self::deb_dependency,
                )?,
            })
        })
    }

    fn package(&mut self, value: toml::Value) -> Result<Package> {
        self.with_table(value, |cx| {
            Ok(Package {
                files: cx.in_array("files", None, Self::package_file)?,
                rpm: cx.in_key("rpm", Self::rpm)?.unwrap_or_default(),
                deb: cx.in_key("deb", Self::deb)?.unwrap_or_default(),
            })
        })
    }

    fn deny_action(&mut self, value: toml::Value) -> Result<DenyAction> {
        self.with_table(value, |cx| {
            let Some(name) = cx.as_string("name")? else {
                bail!("Missing name of action");
            };

            Ok(DenyAction {
                name,
                reason: cx.as_string("reason")?,
            })
        })
    }

    fn latest_action(&mut self, value: toml::Value) -> Result<LatestAction> {
        self.with_table(value, |cx| {
            let Some(name) = cx.as_string("name")? else {
                bail!("Missing name of action");
            };

            let Some(version) = cx.as_string("version")? else {
                bail!("Missing version of action");
            };

            Ok(LatestAction { name, version })
        })
    }

    fn actions(&mut self, value: toml::Value) -> Result<Actions> {
        self.with_table(value, |cx| {
            Ok(Actions {
                deny: cx.in_array("deny", Some(("name", "reason")), Self::deny_action)?,
                latest: cx.in_array("latest", Some(("name", "version")), Self::latest_action)?,
            })
        })
    }
}

struct TableContext<'a, 'b> {
    cx: &'b mut ConfigCtxt<'a>,
    config: &'b mut toml::Table,
}

impl<'a> TableContext<'a, '_> {
    fn as_relative_path(&mut self, key: &str) -> Result<Option<RelativePathBuf>> {
        self.cx.in_key(self.config, key, ConfigCtxt::relative_path)
    }

    fn as_string(&mut self, key: &str) -> Result<Option<String>> {
        self.cx.in_string(self.config, key, |_, string| Ok(string))
    }

    fn as_boolean(&mut self, key: &str) -> Result<Option<bool>> {
        self.cx.in_key(self.config, key, ConfigCtxt::boolean)
    }

    fn in_array<F, O, B>(&mut self, key: &str, map: Option<(&str, &str)>, f: F) -> Result<B>
    where
        F: FnMut(&mut ConfigCtxt<'a>, toml::Value) -> Result<O>,
        B: FromIterator<O>,
    {
        self.cx.in_array(self.config, key, map, f)
    }

    fn in_string<F, O>(&mut self, key: &str, f: F) -> Result<Option<O>>
    where
        F: FnOnce(&mut ConfigCtxt<'a>, String) -> Result<O>,
    {
        self.cx.in_string(self.config, key, f)
    }

    /// Visit the given key, extracting it from the specified table.
    fn in_key<F, O>(&mut self, key: &str, f: F) -> Result<Option<O>>
    where
        F: FnOnce(&mut ConfigCtxt<'a>, toml::Value) -> Result<O>,
    {
        self.cx.in_key(self.config, key, f)
    }

    fn in_table<F, K, V>(&mut self, key: &str, f: F) -> Result<Option<HashMap<K, V>>>
    where
        K: Eq + Hash,
        F: FnMut(&mut ConfigCtxt<'a>, String, toml::Value) -> Result<(K, V)>,
    {
        self.cx.in_table(self.config, key, f)
    }

    fn as_table<F, O>(&mut self, key: &str, f: F) -> Result<Option<O>>
    where
        F: FnOnce(&mut ConfigCtxt<'a>, toml::Table) -> Result<O>,
    {
        self.cx.as_table(self.config, key, f)
    }
}

/// Load a configuration from the given path.
pub(crate) fn load<'a>(
    paths: Paths<'a>,
    templating: &Templating,
    repos: &[Repo],
    defaults: &'a toml::Table,
) -> Result<Config<'a>> {
    let mut cx = ConfigCtxt::new(paths, RelativePath::new(""), templating);

    let Some(config) = cx.kick_config()? else {
        tracing::trace!(
            "{}: Missing configuration file",
            paths.to_path(cx.kick_path).display()
        );

        return Ok(Config {
            base: RepoConfig::default(),
            repos: HashMap::new(),
            defaults,
        });
    };

    load_merged(&mut cx, templating, repos, config, defaults)
}

/// Load merged configuration with base and repo-specific configurations loaded
/// recursively.
fn load_merged<'a>(
    cx: &mut ConfigCtxt<'_>,
    templating: &Templating,
    inputs: &[Repo],
    config: toml::Value,
    defaults: &'a toml::Table,
) -> Result<Config<'a>> {
    let (base, mut repos) = match load_base(cx, config) {
        Ok(output) => output,
        Err(error) => return Err(cx.context(error)),
    };

    for repo in inputs {
        let config = load_repo(cx.paths, cx.current, repo, templating)
            .with_context(|| anyhow!("In repo {}", cx.paths.to_path(repo.path()).display()))?;

        let Some(config) = config else {
            continue;
        };

        let original = repos.entry(RelativePathBuf::from(repo.path())).or_default();
        original.merge_with(config);
    }

    Ok(Config {
        base,
        repos,
        defaults,
    })
}

fn load_base(
    cx: &mut ConfigCtxt<'_>,
    config: toml::Value,
) -> Result<(RepoConfig, HashMap<RelativePathBuf, RepoConfig>)> {
    cx.with_table(config, |cx| {
        let base = ConfigCtxt::repo_table(cx)?;

        let repo_configs = cx
            .in_table("repo", |cx, id, value| {
                Ok((RelativePathBuf::from(id), cx.repo(value)?))
            })?
            .unwrap_or_default();

        Ok((base, repo_configs))
    })
}

fn load_repo(
    paths: Paths<'_>,
    current: &RelativePath,
    repo: &Repo,
    templating: &Templating,
) -> Result<Option<RepoConfig>> {
    let current = current.join(repo.path());
    let mut cx = ConfigCtxt::new(paths, &current, templating);

    let Some(config) = cx.kick_config()? else {
        return Ok(None);
    };

    match cx.repo(config) {
        Ok(repo) => Ok(Some(repo)),
        Err(error) => Err(cx.context(error)),
    }
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
