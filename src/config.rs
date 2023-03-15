use core::fmt;
use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use relative_path::{RelativePath, RelativePathBuf};
use serde::Serialize;

use crate::ctxt::Ctxt;
use crate::model::CrateParams;
use crate::rust_version::{self, RustVersion};
use crate::templates::{Template, Templating};

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

#[derive(Serialize)]
pub(crate) struct PerCrateRender<'a, T: 'a> {
    #[serde(rename = "crate")]
    pub(crate) crate_params: T,
    /// Current job name.
    job_name: &'a str,
    /// Globally known rust versions in use.
    rust_versions: RenderRustVersions,
    #[serde(flatten)]
    extra: &'a toml::Value,
}

pub(crate) struct Repo {
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
}

pub(crate) struct Config {
    default_workflow: Option<Template>,
    pub(crate) job_name: Option<String>,
    pub(crate) license: Option<String>,
    pub(crate) authors: Vec<String>,
    pub(crate) extra: toml::Value,
    pub(crate) documentation: Option<Template>,
    pub(crate) badges: Vec<ConfigBadge>,
    pub(crate) repos: HashMap<String, Repo>,
}

impl Config {
    /// Generate a default workflow.
    pub(crate) fn default_workflow<T>(
        &self,
        cx: &Ctxt<'_>,
        crate_params: T,
    ) -> Result<Option<String>>
    where
        T: Serialize,
    {
        let Some(template) = &self.default_workflow  else {
            return Ok(None);
        };

        Ok(Some(
            template.render(&self.per_crate_render(cx, crate_params))?,
        ))
    }

    /// Set up render parameters.
    pub(crate) fn per_crate_render<'a, T: 'a>(
        &'a self,
        cx: &Ctxt<'_>,
        crate_params: T,
    ) -> PerCrateRender<'a, T> {
        PerCrateRender {
            crate_params,
            job_name: self.job_name(),
            extra: &self.extra,
            rust_versions: RenderRustVersions {
                rustc: cx.rustc_version,
                edition_2018: rust_version::EDITION_2018,
                edition_2021: rust_version::EDITION_2021,
            },
        }
    }

    pub(crate) fn job_name(&self) -> &str {
        self.job_name.as_deref().unwrap_or(DEFAULT_JOB_NAME)
    }

    pub(crate) fn license(&self) -> &str {
        self.license.as_deref().unwrap_or(DEFAULT_LICENSE)
    }

    /// Iterator over badges for the given repo.
    pub(crate) fn badges(&self, repo: &str) -> impl Iterator<Item = &'_ ConfigBadge> {
        let repos = self
            .repos
            .get(repo)
            .into_iter()
            .flat_map(|repo| repo.badges.iter());
        self.badges.iter().chain(repos)
    }

    /// Get the header for the given repo.
    pub(crate) fn header(&self, repo: &str) -> Option<&Template> {
        self.repos.get(repo)?.header.as_ref()
    }

    /// Get crate for the given repo.
    pub(crate) fn crate_for<'a>(&'a self, name: &str) -> Option<&'a str> {
        self.repos.get(name)?.krate.as_deref()
    }

    /// Get Cargo.toml path for the given module.
    pub(crate) fn cargo_toml<'a>(&'a self, name: &str) -> Option<&'a RelativePath> {
        self.repos.get(name)?.cargo_toml.as_deref()
    }

    /// Get Cargo.toml path for the given module.
    pub(crate) fn is_enabled<'a>(&'a self, name: &str, feature: &str) -> bool {
        let Some(repo) = self.repos.get(name) else {
            return true;
        };

        !repo.disabled.contains(feature)
    }
}

pub(crate) struct ConfigBadge {
    markdown: Option<Template>,
    html: Option<Template>,
}

impl ConfigBadge {
    pub(crate) fn markdown(
        &self,
        cx: &Ctxt<'_>,
        params: &CrateParams<'_>,
    ) -> Result<Option<String>> {
        let data = cx.config.per_crate_render(cx, params);

        let Some(template) = self.markdown.as_ref() else {
            return Ok(None);
        };

        Ok(Some(template.render(&data)?))
    }

    pub(crate) fn html(&self, cx: &Ctxt<'_>, params: &CrateParams<'_>) -> Result<Option<String>> {
        let data = cx.config.per_crate_render(cx, params);

        let Some(template) = self.html.as_ref() else {
            return Ok(None);
        };

        Ok(Some(template.render(&data)?))
    }
}

/// Load a configuration from the given path.
pub(crate) fn load<P>(path: P, templating: &Templating) -> Result<Config>
where
    P: AsRef<Path>,
{
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

    struct Ctxt<'a> {
        path: Vec<Part>,
        templating: &'a Templating,
    }

    impl<'a> Ctxt<'a> {
        fn new(templating: &'a Templating) -> Self {
            Self {
                path: Vec::new(),
                templating,
            }
        }

        fn key(&mut self, key: &str) {
            self.path.push(Part::Key(key.to_owned()));
        }

        fn format_path(&self) -> String {
            use std::fmt::Write;

            if self.path.is_empty() {
                return format!(".");
            }

            let mut out = String::new();
            let mut it = self.path.iter();

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
            let path = self.format_path();
            anyhow::Error::msg(format!("{path}: {args}"))
        }

        /// Ensure table is empty.
        fn ensure_empty(&self, table: toml::Table) -> Result<()> {
            if let Some((key, value)) = table.into_iter().next() {
                return Err(self.bail(format_args!("got unsupported key `{key}`: {value}")));
            }

            Ok(())
        }

        fn table(&mut self, config: toml::Value) -> Result<toml::Table> {
            match config {
                toml::Value::Table(table) => Ok(table),
                other => return Err(self.bail(format_args!("expected table, got {other}"))),
            }
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

        fn in_string<F, O>(
            &mut self,
            config: &mut toml::Table,
            key: &str,
            f: F,
        ) -> Result<Option<O>>
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

            self.path.pop();
            Ok(Some(out))
        }

        fn as_string(&mut self, config: &mut toml::Table, key: &str) -> Result<Option<String>> {
            self.in_string(config, key, |_, string| Ok(string))
        }

        fn in_boolean(&mut self, config: &mut toml::Table, key: &str) -> Result<Option<bool>> {
            let Some(value) = config.remove(key) else {
                return Ok(None);
            };

            self.key(key);
            let out = self.boolean(value)?;
            self.path.pop();
            Ok(Some(out))
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
                self.path.push(Part::Index(index));
                out.push(f(self, item)?);
                self.path.pop();
            }

            self.path.pop();
            Ok(Some(out))
        }

        fn badges(
            &mut self,
            config: &mut toml::Table,
        ) -> Result<Option<Vec<ConfigBadge>>, anyhow::Error> {
            let badges = self.in_array(config, "badges", |cx, value| {
                let mut value = cx.table(value)?;
                let alt = cx.as_string(&mut value, "alt")?;
                let src = cx.as_string(&mut value, "src")?;
                let href = cx.as_string(&mut value, "href")?;
                let height = cx.as_string(&mut value, "height")?;

                let alt = FormatOptional(alt.as_ref(), |f, alt| write!(f, " alt=\"{alt}\""));

                let (markdown, html) =
                    if let (Some(src), Some(href), Some(height)) = (src, href, height) {
                        let markdown = cx.templating.compile(&format!(
                            "[<img{alt} src=\"{src}\" height=\"{height}\">]({href})"
                        ))?;
                        let html = cx.templating.compile(&format!(
                            "<a href=\"{href}\"><img{alt} src=\"{src}\" height=\"{height}\"></a>"
                        ))?;
                        (Some(markdown), Some(html))
                    } else {
                        (None, None)
                    };

                Ok(ConfigBadge { markdown, html })
            })?;

            Ok(badges)
        }

        fn repo(&mut self, config: toml::Value) -> Result<Repo> {
            let mut config = self.table(config)?;

            let header = self.in_string(&mut config, "header", |cx, string| {
                cx.templating.compile(&string)
            })?;

            let badges = self.badges(&mut config)?.unwrap_or_default();
            let _ = self
                .in_boolean(&mut config, "center_badges")?
                .unwrap_or_default();
            let krate = self.as_string(&mut config, "crate")?;

            let cargo_toml = self.in_string(&mut config, "cargo_toml", |_, string| {
                Ok(RelativePathBuf::from(string))
            })?;

            let disabled = self.in_array(&mut config, "disabled", |cx, item| cx.string(item))?;

            let disabled = disabled.unwrap_or_default().into_iter().collect();
            self.ensure_empty(config)?;

            Ok(Repo {
                header,
                badges,
                krate,
                cargo_toml,
                disabled,
            })
        }
    }

    let path = path.as_ref();
    let mut cx = Ctxt::new(templating);

    let mut config: toml::Table = {
        let string = std::fs::read_to_string(path)?;
        let config = toml::from_str(&string)?;
        cx.table(config)?
    };

    let parent = path.parent().unwrap_or(path);

    let default_workflow = cx.in_string(&mut config, "default_workflow", |cx, string| {
        let path = RelativePathBuf::from(string).to_path(parent);
        let string =
            std::fs::read_to_string(&path).with_context(|| anyhow!("{}", path.display()))?;
        Ok(cx.templating.compile(&string)?)
    })?;

    let job_name = cx.in_string(&mut config, "job_name", |_, string| Ok(string))?;
    let license = cx.in_string(&mut config, "license", |_, string| Ok(string))?;

    let badges = cx.badges(&mut config)?.unwrap_or_default();

    let authors = cx
        .in_array(&mut config, "authors", |cx, item| cx.string(item))?
        .unwrap_or_default();

    let documentation = cx.in_string(&mut config, "documentation", |cx, string| {
        Ok(cx.templating.compile(&string)?)
    })?;

    let extra = config
        .remove("extra")
        .unwrap_or_else(|| toml::Value::Table(toml::map::Map::default()));

    let mut repos = HashMap::new();

    if let Some(config) = config.remove("repos") {
        cx.key("repos");

        for (id, value) in cx.table(config)? {
            cx.key(&id);
            repos.insert(id.to_owned(), cx.repo(value)?);
            cx.path.pop();
        }

        cx.path.pop();
    }

    cx.ensure_empty(config)?;

    Ok(Config {
        default_workflow,
        job_name,
        license,
        authors,
        extra,
        documentation,
        badges,
        repos,
    })
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
