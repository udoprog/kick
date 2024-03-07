use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::hash::Hash;
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{anyhow, bail, Context, Error, Result};
use relative_path::{RelativePath, RelativePathBuf};
use semver::Version;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::ctxt::Paths;
use crate::glob::Glob;
use crate::model::{Repo, RepoParams, RepoRef};
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

#[derive(Clone, Serialize, Deserialize)]
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
    pub(crate) source: String,
    /// Destination of an rpm file.
    pub(crate) dest: String,
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

#[derive(Default, Debug)]
pub(crate) struct RepoConfig {
    /// Override crate to use.
    pub(crate) name: Option<String>,
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

    /// Test if id is enabled.
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

            if let Some(branch) = &self.base.branch {
                config.branch = Some(branch.clone());
            }

            if let Some(c) = self.base.workflows.get(id) {
                config.merge_with(c.clone());
            }

            if let Some(repo) = self.repos.get(repo.path()) {
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
        if let Some(template) = self
            .repos
            .get(repo.path())
            .and_then(|r| r.workflows.get(id)?.template.as_ref())
        {
            return Ok(Some(template.render(&params)?));
        }

        if let Some(template) = self
            .base
            .workflows
            .get(id)
            .and_then(|r| r.template.as_ref())
        {
            return Ok(Some(template.render(&params)?));
        }

        Ok(None)
    }

    /// Get the current job name.
    pub(crate) fn variable(&self, repo: &RepoRef, key: &str) -> Result<&toml::Value> {
        if let Some(source) = self.repos.get(repo.path()).map(|r| &r.variables) {
            if let Some(value) = source.get(key) {
                return Ok(value);
            }
        }

        if let Some(value) = self.base.variables.get(key) {
            return Ok(value);
        }

        let Some(value) = self.defaults.get(key) else {
            bail!("Missing variable `{key}`");
        };

        Ok(value)
    }

    /// Get a string variable.
    #[allow(unused)]
    pub(crate) fn string_variable(&self, repo: &RepoRef, key: &str) -> Result<&str> {
        let value = match self.variable(repo, key)? {
            toml::Value::String(value) => value,
            other => bail!("Found variable `{key}` with invalid type {other:?}, expected string"),
        };

        Ok(value.as_str())
    }

    /// Get the current documentation template.
    pub(crate) fn documentation(&self, repo: &Repo) -> Option<&Template> {
        if let Some(template) = self
            .repos
            .get(repo.path())
            .and_then(|r| r.documentation.as_ref())
        {
            return Some(template);
        }

        self.base.documentation.as_ref()
    }

    /// Get the current license template.
    pub(crate) fn license(&self, repo: &Repo) -> &str {
        if let Some(template) = self
            .repos
            .get(repo.path())
            .and_then(|r| r.license.as_deref())
        {
            return template;
        }

        self.base.license.as_deref().unwrap_or(DEFAULT_LICENSE)
    }

    /// Get the current license template.
    pub(crate) fn authors(&self, repo: &Repo) -> Vec<String> {
        let mut authors = Vec::new();

        for author in self
            .repos
            .get(repo.path())
            .into_iter()
            .flat_map(|r| r.authors.iter())
        {
            authors.push(author.to_owned());
        }

        authors.extend(self.base.authors.iter().cloned());
        authors
    }

    /// Get the current license template.
    pub(crate) fn variables(&self, repo: &RepoRef) -> toml::Table {
        let mut variables = self.defaults.clone();

        if let Some(branch) = &self.base.branch {
            variables.insert(
                String::from("branch"),
                toml::Value::String(branch.to_owned()),
            );
        }

        merge_map(&mut variables, self.base.variables.clone());

        if let Some(repo) = self.repos.get(repo.path()) {
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
        let mut files = self.base.package.files.iter().collect::<Vec<_>>();

        if let Some(values) = self.repos.get(repo.path()).map(|r| &r.package.files) {
            files.extend(values);
        }

        files
    }

    /// Get all denied actions.
    pub(crate) fn action_deny(&self, repo: &RepoRef) -> Vec<&DenyAction> {
        let mut files = self.base.actions.deny.iter().collect::<Vec<_>>();

        if let Some(values) = self.repos.get(repo.path()).map(|r| &r.actions.deny) {
            files.extend(values);
        }

        files
    }

    /// Get all latest actions.
    pub(crate) fn action_latest(&self, repo: &RepoRef) -> Vec<&LatestAction> {
        let mut files = self.base.actions.latest.iter().collect::<Vec<_>>();

        if let Some(values) = self.repos.get(repo.path()).map(|r| &r.actions.latest) {
            files.extend(values);
        }

        files
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
    pub(crate) fn lib(&self, path: &RelativePath) -> Option<&Template> {
        if let Some(lib) = self.repos.get(path).and_then(|r| r.lib.as_ref()) {
            return Some(lib);
        }

        self.base.lib.as_ref()
    }

    /// Get readme template for the given repo.
    pub(crate) fn readme(&self, path: &RelativePath) -> Option<&Template> {
        if let Some(readme) = self.repos.get(path).and_then(|r| r.readme.as_ref()) {
            return Some(readme);
        }

        self.base.readme.as_ref()
    }

    /// Get crate for the given repo.
    pub(crate) fn name<'a>(&'a self, path: &RelativePath) -> Option<&'a str> {
        if let Some(krate) = self.repos.get(path).and_then(|r| r.name.as_deref()) {
            return Some(krate);
        }

        self.base.name.as_deref()
    }

    /// Get Cargo.toml path for the given repo.
    pub(crate) fn cargo_toml<'a>(&'a self, path: &RelativePath) -> Option<&'a RelativePath> {
        if let Some(cargo_toml) = self.repos.get(path).and_then(|r| r.cargo_toml.as_deref()) {
            return Some(cargo_toml);
        }

        self.base.cargo_toml.as_deref()
    }

    /// Get Cargo.toml path for the given repo.
    pub(crate) fn is_enabled(&self, path: &RelativePath, feature: &str) -> bool {
        let Some(repo) = self.repos.get(path) else {
            return true;
        };

        !repo.disabled.contains(feature)
    }

    /// Get version replacements.
    pub(crate) fn version<'a>(&'a self, repo: &Repo) -> Vec<&'a Replacement> {
        let mut replacements = Vec::new();

        for replacement in self
            .repos
            .get(repo.path())
            .into_iter()
            .flat_map(|r| r.version.iter())
        {
            replacements.push(replacement);
        }

        replacements.extend(self.base.version.iter());
        replacements
    }

    /// Get crate for the given repo.
    pub(crate) fn upgrade(&self, path: &RelativePath) -> Upgrade {
        let mut upgrade = self
            .repos
            .get(path)
            .map(|r| r.upgrade.clone())
            .unwrap_or_default();

        upgrade.merge_with(self.base.upgrade.clone());
        upgrade
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

enum Part {
    Key(String),
    Index(usize),
}

impl fmt::Display for Part {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Part::Key(key) => {
                write!(f, "{key}")
            }
            Part::Index(index) => {
                write!(f, "[{index}]")
            }
        }
    }
}

/// Context used when parsing configuration.
struct ConfigCtxt<'a> {
    paths: Paths<'a>,
    current: &'a RelativePath,
    kick_path: RelativePathBuf,
    parts: Vec<Part>,
    templating: &'a Templating,
}

impl<'a> ConfigCtxt<'a> {
    fn new(paths: Paths<'a>, current: &'a RelativePath, templating: &'a Templating) -> Self {
        Self {
            paths,
            current,
            kick_path: current.join_normalized(KICK_TOML),
            parts: Vec::new(),
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

    fn key(&mut self, key: &str) {
        self.parts.push(Part::Key(key.to_owned()));
    }

    fn format_parts(&self) -> String {
        use std::fmt::Write;

        if self.parts.is_empty() {
            return ".".to_string();
        }

        let mut out = String::new();
        let mut it = self.parts.iter();

        if let Some(p) = it.next() {
            write!(out, "{p}").unwrap();
        }

        for p in it {
            if let Part::Key(..) = p {
                out.push('.');
            }

            write!(out, "{p}").unwrap();
        }

        out
    }

    fn context<E>(&self, error: E) -> anyhow::Error
    where
        anyhow::Error: From<E>,
    {
        let parts = self.format_parts();

        anyhow::Error::from(error).context(anyhow!(
            "In {path}: {parts}",
            path = self.paths.to_path(&self.kick_path).display()
        ))
    }

    /// Ensure table is empty.
    fn ensure_empty(&self, table: toml::Table) -> Result<()> {
        if let Some((key, value)) = table.into_iter().next() {
            bail!("got unsupported key `{key}`: {value}");
        }

        Ok(())
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

    fn in_array<F, O>(
        &mut self,
        config: &mut toml::Table,
        key: &str,
        map: Option<(&str, &str)>,
        mut f: F,
    ) -> Result<Option<Vec<O>>>
    where
        F: FnMut(&mut Self, toml::Value) -> Result<O>,
    {
        let Some(value) = config.remove(key) else {
            return Ok(None);
        };

        self.key(key);
        let array = self.array(value, map)?;
        let mut out = Vec::with_capacity(array.len());

        for (index, item) in array.into_iter().enumerate() {
            self.parts.push(Part::Index(index));
            out.push(f(self, item)?);
            self.parts.pop();
        }

        self.parts.pop();
        Ok(Some(out))
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
        let Some(value) = config.remove(key) else {
            return Ok(None);
        };

        self.key(key);
        let table = self.table(value)?;
        let mut out = HashMap::with_capacity(table.len());

        for (key, item) in table {
            self.parts.push(Part::Key(key.clone()));
            let (key, value) = f(self, key, item)?;
            out.insert(key, value);
            self.parts.pop();
        }

        self.parts.pop();
        Ok(Some(out))
    }

    fn as_table<F, O>(&mut self, config: &mut toml::Table, key: &str, f: F) -> Result<Option<O>>
    where
        F: FnOnce(&mut Self, toml::Table) -> Result<O>,
    {
        let Some(value) = config.remove(key) else {
            return Ok(None);
        };

        self.key(key);
        let table = self.table(value)?;
        let output = f(self, table)?;
        self.parts.pop();
        Ok(Some(output))
    }

    fn in_string<F, O>(&mut self, config: &mut toml::Table, key: &str, f: F) -> Result<Option<O>>
    where
        F: FnOnce(&mut Self, String) -> Result<O>,
    {
        let Some(value) = config.remove(key) else {
            return Ok(None);
        };

        self.key(key);
        let out = self.string(value)?;
        let out = f(self, out)?;
        self.parts.pop();
        Ok(Some(out))
    }

    fn as_string(&mut self, config: &mut toml::Table, key: &str) -> Result<Option<String>> {
        self.in_string(config, key, |_, string| Ok(string))
    }

    fn as_boolean(&mut self, config: &mut toml::Table, key: &str) -> Result<Option<bool>> {
        let Some(value) = config.remove(key) else {
            return Ok(None);
        };

        self.key(key);
        let out = self.boolean(value)?;
        self.parts.pop();
        Ok(Some(out))
    }

    fn badges(&mut self, config: &mut toml::Table) -> Result<Option<Vec<ConfigBadge>>> {
        let badges = self.in_array(config, "badges", None, |cx, value| {
            let mut value = cx.table(value)?;

            let id = cx.as_string(&mut value, "id")?;
            let alt = cx.as_string(&mut value, "alt")?;
            let src = cx.as_string(&mut value, "src")?;
            let href = cx.as_string(&mut value, "href")?;
            let height = cx.as_string(&mut value, "height")?;
            let enabled = cx.as_boolean(&mut value, "enabled")?.unwrap_or(true);

            let alt = FormatOptional(alt.as_ref(), |f, alt| write!(f, " alt=\"{alt}\""));

            let (markdown, html) =
                if let (Some(src), Some(href), Some(height)) = (src, href, height) {
                    let markdown = cx.compile(format!(
                        "[<img{alt} src=\"{src}\" height=\"{height}\">]({href})"
                    ))?;
                    let html = cx.compile(format!(
                        "<a href=\"{href}\"><img{alt} src=\"{src}\" height=\"{height}\"></a>"
                    ))?;
                    (Some(markdown), Some(html))
                } else {
                    (None, None)
                };

            cx.ensure_empty(value)?;

            Ok(ConfigBadge {
                id,
                enabled,
                markdown,
                html,
            })
        })?;

        Ok(badges)
    }

    fn workflow_table(&mut self, config: &mut toml::Table) -> Result<PartialWorkflowConfig> {
        let name = self.in_string(config, "name", |_, string| Ok(string))?;

        let template = self.in_string(config, "template", Self::compile_path)?;

        let features = self.in_array(config, "features", None, |cx, value| {
            let value = cx.string(value)?;
            WorkflowFeature::parse(&value)
        })?;

        let branch = self.as_string(config, "branch")?;
        let disable = self.as_boolean(config, "disable")?;

        Ok(PartialWorkflowConfig {
            name,
            template,
            features: features.unwrap_or_default().into_iter().collect(),
            branch,
            disable,
        })
    }

    fn repo_table(&mut self, config: &mut toml::Table) -> Result<RepoConfig> {
        let name = self.as_string(config, "name")?;
        let branch = self.as_string(config, "branch")?;

        let workflows = self.in_table(config, "workflows", |cx, id, value| {
            Ok((id, cx.workflow(value)?))
        })?;

        let license = self.in_string(config, "license", |_, string| Ok(string))?;

        let authors = self
            .in_array(config, "authors", None, |cx, item| cx.string(item))?
            .unwrap_or_default();

        let documentation = self.in_string(config, "documentation", Self::compile)?;

        let lib = self.in_string(config, "lib", Self::compile_path)?;

        let readme = self.in_string(config, "readme", Self::compile_path)?;

        let badges = self.badges(config)?.unwrap_or_default();
        let _ = self
            .as_boolean(config, "center_badges")?
            .unwrap_or_default();

        let cargo_toml = self.in_string(config, "cargo_toml", |_, string| {
            Ok(RelativePathBuf::from(string))
        })?;

        let disabled = self.in_array(config, "disabled", None, |cx, item| cx.string(item))?;
        let disabled = disabled.unwrap_or_default().into_iter().collect();

        let lib_badges = self.in_array(config, "lib_badges", None, |cx, item| {
            Id::parse(cx.string(item)?)
        })?;

        let lib_badges = lib_badges.unwrap_or_default().into_iter().collect();

        let readme_badges = self.in_array(config, "readme_badges", None, |cx, item| {
            Id::parse(cx.string(item)?)
        })?;

        let readme_badges = readme_badges.unwrap_or_default().into_iter().collect();

        let variables = self
            .as_table(config, "variables", |_, table| Ok(table))?
            .unwrap_or_default();

        let version = self.in_array(config, "version", None, |cx, item| {
            let mut config = cx.table(item)?;
            let package_name = cx.as_string(&mut config, "crate")?;

            let paths = cx
                .in_array(&mut config, "paths", None, |cx, string| {
                    Ok(RelativePathBuf::from(cx.string(string)?))
                })?
                .context("missing `paths`")?;

            let pattern = cx
                .in_string(&mut config, "pattern", |_, pattern| {
                    Ok(regex::bytes::Regex::new(&pattern)?)
                })?
                .context("missing `pattern`")?;

            cx.ensure_empty(config)?;

            Ok(Replacement {
                package_name,
                paths,
                pattern,
            })
        })?;

        let upgrade = self
            .as_table(config, "upgrade", Self::upgrade)?
            .unwrap_or_default();

        let package = self
            .as_table(config, "package", Self::package)?
            .unwrap_or_default();

        let actions = self
            .as_table(config, "actions", Self::actions)?
            .unwrap_or_default();

        Ok(RepoConfig {
            name,
            branch,
            workflows: workflows.unwrap_or_default(),
            license,
            authors,
            documentation,
            lib,
            readme,
            badges,
            cargo_toml,
            disabled,
            lib_badges,
            readme_badges,
            variables,
            version: version.unwrap_or_default(),
            upgrade,
            package,
            actions,
        })
    }

    fn repo(&mut self, config: toml::Value) -> Result<RepoConfig> {
        let mut config = self.table(config)?;
        let repo = self.repo_table(&mut config)?;
        self.ensure_empty(config)?;
        Ok(repo)
    }

    fn workflow(&mut self, config: toml::Value) -> Result<PartialWorkflowConfig> {
        let mut config = self.table(config)?;
        let repo = self.workflow_table(&mut config)?;
        self.ensure_empty(config)?;
        Ok(repo)
    }

    fn upgrade(&mut self, mut config: toml::Table) -> Result<Upgrade> {
        let exclude = self
            .in_array(&mut config, "exclude", None, |cx, item| cx.string(item))?
            .into_iter()
            .flatten()
            .collect();

        self.ensure_empty(config)?;

        Ok(Upgrade { exclude })
    }

    fn package_file(&mut self, value: toml::Value) -> Result<PackageFile> {
        let mut config = self.table(value)?;

        let Some(source) = self.in_string(&mut config, "source", |_, string| Ok(string))? else {
            bail!("Missing source");
        };

        let Some(dest) = self.in_string(&mut config, "dest", |_, string| Ok(string))? else {
            bail!("Missing dest");
        };

        let mode = self.in_string(&mut config, "mode", |_, string| {
            Ok(u16::from_str_radix(&string, 8)?)
        })?;

        self.ensure_empty(config)?;
        Ok(PackageFile { source, dest, mode })
    }

    fn version_requirement(&mut self, string: String) -> Result<VersionRequirement> {
        VersionRequirement::from_str(&string)
    }

    fn rpm_require(&mut self, value: toml::Value) -> Result<RpmRequire> {
        let mut config = self.table(value)?;

        let Some(package) = self.in_string(&mut config, "package", |_, string| Ok(string))? else {
            bail!("Missing package");
        };

        let version = self
            .in_string(&mut config, "version", Self::version_requirement)?
            .unwrap_or_default();

        self.ensure_empty(config)?;
        Ok(RpmRequire { package, version })
    }

    fn deb_dependency(&mut self, value: toml::Value) -> Result<DebDependency> {
        let mut config = self.table(value)?;

        let Some(package) = self.in_string(&mut config, "package", |_, string| Ok(string))? else {
            bail!("Missing package");
        };

        let version = self
            .in_string(&mut config, "version", Self::version_requirement)?
            .unwrap_or_default();

        self.ensure_empty(config)?;
        Ok(DebDependency { package, version })
    }

    fn rpm(&mut self, mut config: toml::Table) -> Result<RpmPackage> {
        let requires = self
            .in_array(&mut config, "requires", None, Self::rpm_require)?
            .into_iter()
            .flatten()
            .collect();

        self.ensure_empty(config)?;
        Ok(RpmPackage { requires })
    }

    fn deb(&mut self, mut config: toml::Table) -> Result<DebPackage> {
        let depends = self
            .in_array(&mut config, "depends", None, Self::deb_dependency)?
            .into_iter()
            .flatten()
            .collect();

        self.ensure_empty(config)?;
        Ok(DebPackage { depends })
    }

    fn package(&mut self, mut config: toml::Table) -> Result<Package> {
        let files = self
            .in_array(&mut config, "files", None, Self::package_file)?
            .into_iter()
            .flatten()
            .collect();

        let rpm = self
            .as_table(&mut config, "rpm", Self::rpm)?
            .unwrap_or_default();

        let deb = self
            .as_table(&mut config, "deb", Self::deb)?
            .unwrap_or_default();

        self.ensure_empty(config)?;
        Ok(Package { files, rpm, deb })
    }

    fn deny_action(&mut self, value: toml::Value) -> Result<DenyAction> {
        let mut config = self.table(value)?;

        let Some(name) = self.as_string(&mut config, "name")? else {
            bail!("Missing name of action");
        };

        let reason = self.as_string(&mut config, "reason")?;

        self.ensure_empty(config)?;
        Ok(DenyAction { name, reason })
    }

    fn latest_action(&mut self, value: toml::Value) -> Result<LatestAction> {
        let mut config = self.table(value)?;

        let Some(name) = self.as_string(&mut config, "name")? else {
            bail!("Missing name of action");
        };

        let Some(version) = self.as_string(&mut config, "version")? else {
            bail!("Missing version of action");
        };

        self.ensure_empty(config)?;
        Ok(LatestAction { name, version })
    }

    fn actions(&mut self, mut config: toml::Table) -> Result<Actions> {
        let deny = self
            .in_array(
                &mut config,
                "deny",
                Some(("name", "reason")),
                Self::deny_action,
            )?
            .unwrap_or_default();

        let latest = self
            .in_array(
                &mut config,
                "latest",
                Some(("name", "version")),
                Self::latest_action,
            )?
            .unwrap_or_default();

        self.ensure_empty(config)?;
        Ok(Actions { deny, latest })
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
    let mut config = cx.table(config)?;
    let base = cx.repo_table(&mut config)?;

    let repo_configs = cx
        .in_table(&mut config, "repo", |cx, id, value| {
            Ok((RelativePathBuf::from(id), cx.repo(value)?))
        })?
        .unwrap_or_default();

    cx.ensure_empty(config)?;
    Ok((base, repo_configs))
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
