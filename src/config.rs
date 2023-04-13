use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs;
use std::hash::Hash;
use std::io::{self, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Error, Result};
use relative_path::{RelativePath, RelativePathBuf};
use tempfile::NamedTempFile;

use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::model::{CrateParams, Module, ModuleParams, RenderRustVersions};
use crate::rust_version::{self};
use crate::templates::{Template, Templating};
use crate::KICK_TOML;

/// Default job name.
const DEFAULT_JOB_NAME: &str = "CI";
/// Default license to use in configuration.
const DEFAULT_LICENSE: &str = "MIT/Apache-2.0";

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
        let (mut file, path) = file.keep()?;
        file.flush()?;
        drop(file);
        fs::rename(path, &self.path)?;
        Ok(())
    }
}

#[derive(Default, Clone)]
pub(crate) struct Upgrade {
    /// Packages to exclude during an upgrade.
    pub(crate) exclude: BTreeSet<String>,
}

impl Upgrade {
    fn merge_with(&mut self, other: Self) {
        self.exclude.extend(other.exclude);
    }
}

#[derive(Clone)]
pub(crate) struct Replacement {
    /// Replacements to perform in a given crate.
    pub(crate) crate_name: Option<String>,
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
                let output_path = crate::utils::to_path(&path, root);

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

#[derive(Default)]
pub(crate) struct Repo {
    pub(crate) workflow: Option<Template>,
    /// The name of the actions job.
    pub(crate) job_name: Option<String>,
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
    /// Override crate to use.
    pub(crate) krate: Option<String>,
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
}

impl Repo {
    /// Merge this config with another.
    pub(crate) fn merge_with(&mut self, mut other: Self) {
        self.workflow = other.workflow.or(self.workflow.take());
        self.job_name = other.job_name.or(self.job_name.take());
        self.license = other.license.or(self.license.take());
        self.authors.append(&mut other.authors);
        self.documentation = other.documentation.or(self.documentation.take());
        self.lib = other.lib.or(self.lib.take());
        self.badges.append(&mut other.badges);
        self.krate = other.krate.or(self.krate.take());
        self.cargo_toml = other.cargo_toml.or(self.cargo_toml.take());
        self.disabled.extend(other.disabled);
        self.lib_badges.merge_with(other.lib_badges);
        self.readme_badges.merge_with(other.readme_badges);
        self.version.extend(other.version);
        self.upgrade.merge_with(other.upgrade);
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
        if !enabled {
            self.enabled.contains(id)
        } else {
            !self.disabled.contains(id)
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

#[derive(Default)]
pub(crate) struct Config {
    pub(crate) base: Repo,
    pub(crate) repos: HashMap<RelativePathBuf, Repo>,
}

impl Config {
    /// Generate a default workflow.
    pub(crate) fn workflow(
        &self,
        module: &Module,
        params: ModuleParams<'_>,
    ) -> Result<Option<String>> {
        let Some(template) = &self.repos.get(module.path()).and_then(|r|r.workflow.as_ref()).or(self.base.workflow.as_ref())  else {
            return Ok(None);
        };

        Ok(Some(template.render(&params)?))
    }

    /// Set up render parameters.
    pub(crate) fn module_params<'a>(
        &'a self,
        cx: &Ctxt<'_>,
        module: &Module,
        crate_params: CrateParams<'a>,
        variables: toml::Table,
    ) -> ModuleParams<'a> {
        ModuleParams {
            crate_params,
            job_name: self.job_name(module),
            rust_versions: RenderRustVersions {
                rustc: cx.rustc_version,
                edition_2018: rust_version::EDITION_2018,
                edition_2021: rust_version::EDITION_2021,
            },
            variables,
        }
    }

    /// Get the current job name.
    pub(crate) fn job_name(&self, module: &Module) -> &str {
        if let Some(name) = self
            .repos
            .get(module.path())
            .and_then(|r| r.job_name.as_deref())
        {
            return name;
        }

        self.base.job_name.as_deref().unwrap_or(DEFAULT_JOB_NAME)
    }

    /// Get the current documentation template.
    pub(crate) fn documentation(&self, module: &Module) -> Option<&Template> {
        if let Some(template) = self
            .repos
            .get(module.path())
            .and_then(|r| r.documentation.as_ref())
        {
            return Some(template);
        }

        self.base.documentation.as_ref()
    }

    /// Get the current license template.
    pub(crate) fn license(&self, module: &Module) -> &str {
        if let Some(template) = self
            .repos
            .get(module.path())
            .and_then(|r| r.license.as_deref())
        {
            return template;
        }

        self.base.license.as_deref().unwrap_or(DEFAULT_LICENSE)
    }

    /// Get the current license template.
    pub(crate) fn authors(&self, module: &Module) -> Vec<String> {
        let mut authors = Vec::new();

        for author in self
            .repos
            .get(module.path())
            .into_iter()
            .flat_map(|r| r.authors.iter())
        {
            authors.push(author.to_owned());
        }

        authors.extend(self.base.authors.iter().cloned());
        authors
    }

    /// Get the current license template.
    pub(crate) fn variables(&self, module: &Module) -> toml::Table {
        let mut variables = self.base.variables.clone();

        if let Some(source) = self.repos.get(module.path()).map(|r| &r.variables) {
            merge_map(&mut variables, source.clone());
        }

        variables
    }

    fn badges<F>(&self, path: &RelativePath, mut filter: F) -> impl Iterator<Item = &'_ ConfigBadge>
    where
        F: FnMut(&Repo, &ConfigBadge, bool) -> bool,
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

    /// Get readme template for the given module.
    pub(crate) fn readme(&self, path: &RelativePath) -> Option<&Template> {
        if let Some(readme) = self.repos.get(path).and_then(|r| r.readme.as_ref()) {
            return Some(readme);
        }

        self.base.readme.as_ref()
    }

    /// Get crate for the given repo.
    pub(crate) fn crate_for<'a>(&'a self, path: &RelativePath) -> Option<&'a str> {
        if let Some(krate) = self.repos.get(path).and_then(|r| r.krate.as_deref()) {
            return Some(krate);
        }

        self.base.krate.as_deref()
    }

    /// Get Cargo.toml path for the given module.
    pub(crate) fn cargo_toml<'a>(&'a self, path: &RelativePath) -> Option<&'a RelativePath> {
        if let Some(cargo_toml) = self.repos.get(path).and_then(|r| r.cargo_toml.as_deref()) {
            return Some(cargo_toml);
        }

        self.base.cargo_toml.as_deref()
    }

    /// Get Cargo.toml path for the given module.
    pub(crate) fn is_enabled(&self, path: &RelativePath, feature: &str) -> bool {
        let Some(repo) = self.repos.get(path) else {
            return true;
        };

        !repo.disabled.contains(feature)
    }

    /// Get version replacements.
    pub(crate) fn version<'a>(&'a self, module: &Module) -> Vec<&'a Replacement> {
        let mut replacements = Vec::new();

        for replacement in self
            .repos
            .get(module.path())
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

pub(crate) struct ConfigBadge {
    pub(crate) id: Option<String>,
    enabled: bool,
    markdown: Option<Template>,
    html: Option<Template>,
}

impl ConfigBadge {
    pub(crate) fn markdown(&self, params: &ModuleParams<'_>) -> Result<Option<String>> {
        let Some(template) = self.markdown.as_ref() else {
            return Ok(None);
        };

        Ok(Some(template.render(params)?))
    }

    pub(crate) fn html(&self, params: &ModuleParams<'_>) -> Result<Option<String>> {
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
    root: &'a Path,
    kick_path: PathBuf,
    parts: Vec<Part>,
    templating: &'a Templating,
}

impl<'a> ConfigCtxt<'a> {
    fn new(root: &'a Path, templating: &'a Templating) -> Self {
        Self {
            root,
            kick_path: root.join(KICK_TOML),
            parts: Vec::new(),
            templating,
        }
    }

    /// Load the kick config.
    fn kick_config(&self) -> Result<Option<toml::Value>> {
        let string = match std::fs::read_to_string(&self.kick_path) {
            Ok(string) => string,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| self.kick_path.display().to_string()),
        };

        let config =
            toml::from_str(&string).with_context(|| self.kick_path.display().to_string())?;
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

    fn bail(&self, args: impl fmt::Display) -> anyhow::Error {
        let parts = self.format_parts();
        anyhow::Error::msg(format!(
            "{path}: {parts}: {args}",
            path = self.kick_path.display()
        ))
    }

    /// Ensure table is empty.
    fn ensure_empty(&self, table: toml::Table) -> Result<()> {
        if let Some((key, value)) = table.into_iter().next() {
            return Err(self.bail(format_args!("got unsupported key `{key}`: {value}")));
        }

        Ok(())
    }

    /// Compile a template.
    fn compile(&mut self, source: &str) -> Result<Template> {
        self.templating.compile(source)
    }

    fn string(&mut self, value: toml::Value) -> Result<String> {
        match value {
            toml::Value::String(string) => Ok(string),
            other => Err(self.bail(format_args!("expected string, got {other}"))),
        }
    }

    fn boolean(&mut self, value: toml::Value) -> Result<bool> {
        match value {
            toml::Value::Boolean(value) => Ok(value),
            other => Err(self.bail(format_args!("expected boolean, got {other}"))),
        }
    }

    fn array(&mut self, value: toml::Value) -> Result<Vec<toml::Value>> {
        match value {
            toml::Value::Array(array) => Ok(array),
            other => Err(self.bail(format_args!("expected array, got {other}"))),
        }
    }

    fn table(&mut self, value: toml::Value) -> Result<toml::Table> {
        match value {
            toml::Value::Table(table) => Ok(table),
            other => return Err(self.bail(format_args!("expected table, got {other}"))),
        }
    }

    fn in_array<F, O>(
        &mut self,
        config: &mut toml::Table,
        key: &str,
        mut f: F,
    ) -> Result<Option<Vec<O>>>
    where
        F: FnMut(&mut Self, toml::Value) -> Result<O>,
    {
        let Some(value) = config.remove(key) else {
            return Ok(None);
        };

        self.key(key);
        let array = self.array(value)?;
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

    fn as_table(&mut self, config: &mut toml::Table, key: &str) -> Result<Option<toml::Table>> {
        let Some(value) = config.remove(key) else {
            return Ok(None);
        };

        Ok(Some(self.table(value)?))
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

        let out = match f(self, out) {
            Ok(out) => out,
            Err(e) => {
                return Err(e.context(self.bail(format_args!("failed to process string"))));
            }
        };

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

    fn badges(
        &mut self,
        config: &mut toml::Table,
    ) -> Result<Option<Vec<ConfigBadge>>, anyhow::Error> {
        let badges = self.in_array(config, "badges", |cx, value| {
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
                    let markdown = cx.compile(&format!(
                        "[<img{alt} src=\"{src}\" height=\"{height}\">]({href})"
                    ))?;
                    let html = cx.compile(&format!(
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

    fn repo_table(&mut self, config: &mut toml::Table) -> Result<Repo> {
        let workflow = self.in_string(config, "workflow", |cx, string| {
            let path = cx.root.join(string);
            let template =
                std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
            cx.compile(&template)
        })?;

        let job_name = self.in_string(config, "job_name", |_, string| Ok(string))?;
        let license = self.in_string(config, "license", |_, string| Ok(string))?;

        let authors = self
            .in_array(config, "authors", |cx, item| cx.string(item))?
            .unwrap_or_default();

        let documentation =
            self.in_string(config, "documentation", |cx, source| cx.compile(&source))?;

        let lib = self.in_string(config, "lib", |cx, string| {
            let path = cx.root.join(string);
            let template =
                std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
            cx.compile(&template)
        })?;

        let readme = self.in_string(config, "readme", |cx, string| {
            let path = cx.root.join(string);
            let template =
                std::fs::read_to_string(&path).with_context(|| path.display().to_string())?;
            cx.compile(&template)
        })?;

        let badges = self.badges(config)?.unwrap_or_default();
        let _ = self
            .as_boolean(config, "center_badges")?
            .unwrap_or_default();
        let krate = self.as_string(config, "crate")?;

        let cargo_toml = self.in_string(config, "cargo_toml", |_, string| {
            Ok(RelativePathBuf::from(string))
        })?;

        let disabled = self.in_array(config, "disabled", |cx, item| cx.string(item))?;
        let disabled = disabled.unwrap_or_default().into_iter().collect();

        let lib_badges =
            self.in_array(config, "lib_badges", |cx, item| Id::parse(cx.string(item)?))?;

        let lib_badges = lib_badges.unwrap_or_default().into_iter().collect();

        let readme_badges = self.in_array(config, "readme_badges", |cx, item| {
            Id::parse(cx.string(item)?)
        })?;

        let readme_badges = readme_badges.unwrap_or_default().into_iter().collect();

        let variables = self.as_table(config, "variables")?.unwrap_or_default();

        let version = self.in_array(config, "version", |cx, item| {
            let mut config = cx.table(item)?;
            let crate_name = cx.as_string(&mut config, "crate")?;

            let paths = cx
                .in_array(&mut config, "paths", |cx, string| {
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
                crate_name,
                paths,
                pattern,
            })
        })?;

        let upgrade = self.as_table(config, "upgrade")?.unwrap_or_default();
        let upgrade = self.upgrade(upgrade)?;

        Ok(Repo {
            workflow,
            job_name,
            license,
            authors,
            documentation,
            lib,
            readme,
            badges,
            krate,
            cargo_toml,
            disabled,
            lib_badges,
            readme_badges,
            variables,
            version: version.unwrap_or_default(),
            upgrade,
        })
    }

    fn repo(&mut self, config: toml::Value) -> Result<Repo> {
        let mut config = self.table(config)?;
        let repo = self.repo_table(&mut config)?;
        self.ensure_empty(config)?;
        Ok(repo)
    }

    fn upgrade(&mut self, mut config: toml::Table) -> Result<Upgrade> {
        let exclude = self
            .in_array(&mut config, "exclude", |cx, item| cx.string(item))?
            .into_iter()
            .flatten()
            .collect();

        self.ensure_empty(config)?;

        Ok(Upgrade { exclude })
    }
}

/// Load a configuration from the given path.
pub(crate) fn load(root: &Path, templating: &Templating, modules: &[Module]) -> Result<Config> {
    let mut cx = ConfigCtxt::new(root, templating);

    let Some(config) = cx.kick_config()? else {
        tracing::trace!("{}: missing configuration file", cx.kick_path.display());
        return Ok(Config::default());
    };

    load_base(&mut cx, templating, modules, config)
        .with_context(|| cx.kick_path.display().to_string())
}

fn load_base(
    cx: &mut ConfigCtxt<'_>,
    templating: &Templating,
    modules: &[Module],
    config: toml::Value,
) -> Result<Config> {
    let mut config = cx.table(config)?;
    let base = cx.repo_table(&mut config)?;

    let mut repos = cx
        .in_table(&mut config, "repos", |cx, id, value| {
            Ok((RelativePathBuf::from(id), cx.repo(value)?))
        })?
        .unwrap_or_default();

    for module in modules {
        let Some(repo) = load_repo(cx.root, module, templating).with_context(|| module.path().to_owned())? else {
            continue;
        };

        let original = repos
            .entry(RelativePathBuf::from(module.path()))
            .or_default();

        original.merge_with(repo);
    }

    cx.ensure_empty(config)?;
    Ok(Config { base, repos })
}

fn load_repo(root: &Path, module: &Module, templating: &Templating) -> Result<Option<Repo>> {
    let root = crate::utils::to_path(module.path(), root);
    let mut cx = ConfigCtxt::new(&root, templating);

    let Some(config) = cx.kick_config()? else {
        return Ok(None);
    };

    let repo = cx.repo(config)?;
    Ok(Some(repo))
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
