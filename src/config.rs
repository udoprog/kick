use core::fmt;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::hash::Hash;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use relative_path::{RelativePath, RelativePathBuf};
use serde::Serialize;

use crate::ctxt::Ctxt;
use crate::model::{CrateParams, Module};
use crate::rust_version::{self, RustVersion};
use crate::templates::{Template, Templating};
use crate::KICK_TOML;

/// Default job name.
const DEFAULT_JOB_NAME: &str = "CI";
/// Default license to use in configuration.
const DEFAULT_LICENSE: &str = "MIT/Apache-2.0";

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct RenderRustVersions {
    rustc: Option<RustVersion>,
    edition_2018: RustVersion,
    edition_2021: RustVersion,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub(crate) struct PerCrateRender<'a> {
    #[serde(rename = "crate")]
    pub(crate) crate_params: CrateParams<'a>,
    /// Current job name.
    job_name: &'a str,
    /// Globally known rust versions in use.
    rust_versions: RenderRustVersions,
    #[serde(flatten)]
    extra: &'a HashMap<String, toml::Value>,
}

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
    /// Custom header template.
    pub(crate) header: Option<Template>,
    /// Custom badges for a specific project.
    pub(crate) badges: Vec<ConfigBadge>,
    /// Override crate to use.
    pub(crate) krate: Option<String>,
    /// Path to Cargo.toml to build.
    pub(crate) cargo_toml: Option<RelativePathBuf>,
    /// Disabled modules.
    pub(crate) disabled: BTreeSet<String>,
    /// Explicit allowlist for badges to enabled which are already disabled.
    pub(crate) enabled_badges: HashSet<String>,
    /// Explicit blocklist for badges to enabled.
    pub(crate) disabled_badges: HashSet<String>,
}

impl Repo {
    /// Test if this repo wants the specified badge.
    pub(crate) fn wants_badge(&self, b: &ConfigBadge) -> bool {
        let Some(id) = &b.id else {
            return b.enabled;
        };

        if b.enabled {
            !self.disabled_badges.contains(id)
        } else {
            self.enabled_badges.contains(id)
        }
    }
}

pub(crate) struct Config {
    pub(crate) base: Repo,
    pub(crate) repos: HashMap<RelativePathBuf, Repo>,
    pub(crate) extra: HashMap<String, toml::Value>,
}

impl Config {
    /// Generate a default workflow.
    pub(crate) fn workflow(
        &self,
        module: &Module,
        params: PerCrateRender<'_>,
    ) -> Result<Option<String>> {
        let Some(template) = &self.repos.get(module.path.as_ref()).and_then(|r|r.workflow.as_ref()).or(self.base.workflow.as_ref())  else {
            return Ok(None);
        };

        Ok(Some(template.render(&params)?))
    }

    /// Set up render parameters.
    pub(crate) fn per_crate_render<'a>(
        &'a self,
        cx: &Ctxt<'_>,
        module: &Module,
        crate_params: CrateParams<'a>,
    ) -> PerCrateRender<'a> {
        PerCrateRender {
            crate_params,
            job_name: self.job_name(module),
            rust_versions: RenderRustVersions {
                rustc: cx.rustc_version,
                edition_2018: rust_version::EDITION_2018,
                edition_2021: rust_version::EDITION_2021,
            },
            extra: &self.extra,
        }
    }

    /// Get the current job name.
    pub(crate) fn job_name(&self, module: &Module) -> &str {
        if let Some(name) = self
            .repos
            .get(module.path.as_ref())
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
            .get(module.path.as_ref())
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
            .get(module.path.as_ref())
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
            .get(module.path.as_ref())
            .into_iter()
            .flat_map(|r| r.authors.iter())
        {
            authors.push(author.to_owned());
        }

        authors.extend(self.base.authors.iter().cloned());
        authors
    }

    /// Iterator over badges for the given repo.
    pub(crate) fn badges(&self, path: &RelativePath) -> impl Iterator<Item = &'_ ConfigBadge> {
        let repo = self.repos.get(path);
        let repos = repo.into_iter().flat_map(|repo| repo.badges.iter());

        self.base
            .badges
            .iter()
            .chain(repos)
            .filter(move |b| match repo {
                Some(repo) => repo.wants_badge(b),
                None => b.enabled,
            })
    }

    /// Get the header for the given repo.
    pub(crate) fn header(&self, path: &RelativePath) -> Option<&Template> {
        if let Some(header) = self.repos.get(path).and_then(|r| r.header.as_ref()) {
            return Some(header);
        }

        self.base.header.as_ref()
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
}

pub(crate) struct ConfigBadge {
    id: Option<String>,
    enabled: bool,
    markdown: Option<Template>,
    html: Option<Template>,
}

impl ConfigBadge {
    pub(crate) fn markdown(&self, params: PerCrateRender<'_>) -> Result<Option<String>> {
        let Some(template) = self.markdown.as_ref() else {
            return Ok(None);
        };

        Ok(Some(template.render(&params)?))
    }

    pub(crate) fn html(&self, params: PerCrateRender<'_>) -> Result<Option<String>> {
        let Some(template) = self.html.as_ref() else {
            return Ok(None);
        };

        Ok(Some(template.render(&params)?))
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
    path: &'a RelativePath,
    kick_path: RelativePathBuf,
    parts: Vec<Part>,
    templating: &'a Templating,
}

impl<'a> ConfigCtxt<'a> {
    fn new(root: &'a Path, path: &'a RelativePath, templating: &'a Templating) -> Self {
        let kick_path = path.join(KICK_TOML);

        Self {
            root,
            path,
            kick_path,
            parts: Vec::new(),
            templating,
        }
    }

    /// Load the kick config.
    fn kick_config(&self) -> Result<Option<toml::Value>> {
        let string = match std::fs::read_to_string(self.kick_path.to_path(self.root)) {
            Ok(string) => string,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| self.kick_path.to_owned()),
        };

        let config = toml::from_str(&string).with_context(|| self.kick_path.to_owned())?;
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
        anyhow::Error::msg(format!("{path}: {parts}: {args}", path = self.kick_path))
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
            let path = cx.path.join(string);
            let template =
                std::fs::read_to_string(path.to_path(cx.root)).with_context(|| path.to_owned())?;
            cx.compile(&template)
        })?;

        let job_name = self.in_string(config, "job_name", |_, string| Ok(string))?;
        let license = self.in_string(config, "license", |_, string| Ok(string))?;

        let authors = self
            .in_array(config, "authors", |cx, item| cx.string(item))?
            .unwrap_or_default();

        let documentation =
            self.in_string(config, "documentation", |cx, source| cx.compile(&source))?;

        let header = self.in_string(config, "header", |cx, string| {
            let path = cx.path.join(string);
            let template =
                std::fs::read_to_string(path.to_path(cx.root)).with_context(|| path.to_owned())?;
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

        let disabled_badges =
            self.in_array(config, "disabled_badges", |cx, item| cx.string(item))?;
        let disabled_badges = disabled_badges.unwrap_or_default().into_iter().collect();

        let enabled_badges = self.in_array(config, "enabled_badges", |cx, item| cx.string(item))?;
        let enabled_badges = enabled_badges.unwrap_or_default().into_iter().collect();

        Ok(Repo {
            workflow,
            job_name,
            license,
            authors,
            documentation,
            header,
            badges,
            krate,
            cargo_toml,
            disabled,
            enabled_badges,
            disabled_badges,
        })
    }

    fn repo(&mut self, config: toml::Value) -> Result<Repo> {
        let mut config = self.table(config)?;
        let repo = self.repo_table(&mut config)?;
        self.ensure_empty(config)?;
        Ok(repo)
    }
}

/// Load a configuration from the given path.
pub(crate) fn load(
    root: &Path,
    path: &RelativePath,
    templating: &Templating,
    modules: &[Module],
) -> Result<Config> {
    let mut cx = ConfigCtxt::new(root, path, templating);

    let Some(config) = cx.kick_config()? else {
        return Err(anyhow!("{}: missing file", cx.kick_path));
    };

    let mut config = cx.table(config)?;
    let base = cx.repo_table(&mut config)?;

    let extra = cx
        .in_table(&mut config, "extra", |_, key, value| Ok((key, value)))?
        .unwrap_or_default();

    let mut repos = cx
        .in_table(&mut config, "repos", |cx, id, value| {
            Ok((path.join(id), cx.repo(value)?))
        })?
        .unwrap_or_default();

    for module in modules {
        let Some(repo) = load_repo(root, module, templating).with_context(|| module.path.clone())? else {
            continue;
        };

        repos.insert(RelativePathBuf::from(module.path.as_ref()), repo);
    }

    cx.ensure_empty(config)?;
    Ok(Config { base, repos, extra })
}

fn load_repo(root: &Path, module: &Module, templating: &Templating) -> Result<Option<Repo>> {
    let mut cx = ConfigCtxt::new(root, &module.path, templating);

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
