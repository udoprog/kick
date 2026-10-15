use core::fmt;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use relative_path::RelativePath;
use reqwest::{StatusCode, Url};
use semver::{Op, Version, VersionReq};
use tokio::sync::OnceCell;

use crate::cargo::{
    BUILD_DEPENDENCIES, DEPENDENCIES, DEPS, DEV_DEPENDENCIES, Dependencies, DependencyItem,
    Manifest,
};
use crate::cli::WithRepos;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::workspace::{self, Crates};

#[derive(Debug, Parser)]
pub(crate) struct Opts {
    /// The crate to look for, optionally followed by a semver selector such as
    /// `serde <1.0.200`, `sqll <= 0.14.7` or `tokio@^0.2`.
    ///
    /// The selector may be part of the same argument or given as further
    /// arguments, which are joined with spaces, and may be preceded by `@`.
    ///
    /// A declaration matches the selector when the lowest version its
    /// requirement admits does, so `1.2` is tested as `1.2.0`. Path and git
    /// dependencies without a version only match when no selector is given.
    ///
    /// Optional with `--outdated` and `--patched`, which then report every
    /// dependency.
    #[arg(
        value_name = "CRATE [SELECTOR]",
        required_unless_present_any = ["outdated", "patched"],
        num_args = 1..
    )]
    spec: Vec<String>,
    /// Only report versioned registry dependencies whose requirement does not
    /// admit the newest version published on the index.
    #[arg(long)]
    outdated: bool,
    /// Report crates overridden through `[patch.<source>]` tables or legacy
    /// `[replace]` entries instead of dependency declarations.
    ///
    /// A selector matches the `version` of a patch entry, or the version of a
    /// `[replace]` spec such as `foo:1.2.3`.
    #[arg(long, conflicts_with = "outdated")]
    patched: bool,
    /// The index to compare against with `--outdated`: a sparse index URL
    /// (`sparse+https://...`), a `file://` URL or a local directory with the
    /// same layout.
    // Clap waives `requires` for an arg that conflicts with a present one, so
    // the conflict with `patched` is spelled out.
    #[arg(
        long,
        value_name = "URL",
        requires = "outdated",
        conflicts_with = "patched",
        default_value = CRATES_IO
    )]
    index: String,
}

const CRATES_IO: &str = "sparse+https://index.crates.io/";

#[derive(Debug, Clone)]
struct Spec {
    name: String,
    selector: Option<VersionReq>,
}

fn parse_spec(input: &str) -> Result<Spec, String> {
    let input = input.trim();

    let split = input
        .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_')))
        .unwrap_or(input.len());

    let (name, rest) = input.split_at(split);

    if name.is_empty() {
        return Err(format!("missing crate name in `{input}`"));
    }

    let rest = rest.trim_start();

    let selector = match rest.strip_prefix('@') {
        Some(selector) => Some(selector.trim()),
        None if rest.is_empty() => None,
        None => Some(rest),
    };

    let selector = match selector {
        Some(selector) => Some(
            VersionReq::parse(selector)
                .map_err(|error| format!("invalid selector `{selector}`: {error}"))?,
        ),
        None => None,
    };

    Ok(Spec {
        name: name.to_owned(),
        selector,
    })
}

pub(crate) async fn entry(with_repos: &mut WithRepos<'_>, opts: &Opts) -> Result<()> {
    if opts.outdated {
        return outdated_entry(with_repos, opts).await;
    }

    if opts.patched {
        return patched_entry(with_repos, opts);
    }

    let spec = parse_spec(&opts.spec.join(" ")).map_err(anyhow::Error::msg)?;
    let matches = Cell::new(0usize);
    let repos = Cell::new(0usize);

    with_repos.run(
        "find dependents",
        format_args!("deps: {spec:?}"),
        |cx, repo| {
            let count = deps(cx, repo, &spec)?;

            if count > 0 {
                matches.set(matches.get() + count);
                repos.set(repos.get() + 1);
            }

            Ok(())
        },
    )?;

    let (matches, repos) = (matches.get(), repos.get());
    let s = |n: usize| if n == 1 { "" } else { "s" };
    println!(
        "{matches} declaration{} in {repos} repo{}",
        s(matches),
        s(repos)
    );
    Ok(())
}

fn deps(cx: &Ctxt<'_>, repo: &Repo, spec: &Spec) -> Result<usize> {
    let Some(crates) = workspace::open(cx, repo)? else {
        return Ok(0);
    };

    let found = find(&crates, Some(spec))?;

    for found in &found {
        let manifest = found
            .manifest
            .strip_prefix(repo.path())
            .unwrap_or(found.manifest);

        println!("{}: {manifest} {found}", repo.path());
    }

    Ok(found.len())
}

fn optional_spec(opts: &Opts) -> Result<Option<Spec>> {
    if opts.spec.is_empty() {
        return Ok(None);
    }

    Ok(Some(
        parse_spec(&opts.spec.join(" ")).map_err(anyhow::Error::msg)?,
    ))
}

fn patched_entry(with_repos: &mut WithRepos<'_>, opts: &Opts) -> Result<()> {
    let spec = optional_spec(opts)?;
    let matches = Cell::new(0usize);
    let repos = Cell::new(0usize);

    with_repos.run(
        "find patched dependencies",
        format_args!("deps --patched: {spec:?}"),
        |cx, repo| {
            let Some(crates) = workspace::open(cx, repo)? else {
                return Ok(());
            };

            let found = find_patched(&crates, spec.as_ref())?;

            for found in &found {
                let manifest = found
                    .manifest
                    .strip_prefix(repo.path())
                    .unwrap_or(found.manifest);

                println!("{}: {manifest} {found}", repo.path());
            }

            if !found.is_empty() {
                matches.set(matches.get() + found.len());
                repos.set(repos.get() + 1);
            }

            Ok(())
        },
    )?;

    let (matches, repos) = (matches.get(), repos.get());
    let s = |n: usize| if n == 1 { "" } else { "s" };
    println!(
        "{matches} patched declaration{} in {repos} repo{}",
        s(matches),
        s(repos)
    );
    Ok(())
}

async fn outdated_entry(with_repos: &mut WithRepos<'_>, opts: &Opts) -> Result<()> {
    let spec = optional_spec(opts)?;
    let index = Index::open(&opts.index)?;
    let (mut matches, mut repos) = (0usize, 0usize);

    with_repos
        .run_async(
            "find outdated dependencies",
            format_args!("deps --outdated: {spec:?}"),
            async |cx, repo| {
                let Some(crates) = workspace::open(cx, repo)? else {
                    return Ok(Vec::new());
                };

                let found = outdated(&crates, spec.as_ref(), &index).await?;
                let mut lines = Vec::with_capacity(found.len());

                for (found, latest) in &found {
                    let manifest = found
                        .manifest
                        .strip_prefix(repo.path())
                        .unwrap_or(found.manifest);

                    lines.push(format!(
                        "{}: {manifest} {found} (latest {latest})",
                        repo.path()
                    ));
                }

                Ok(lines)
            },
            |lines| {
                if !lines.is_empty() {
                    matches += lines.len();
                    repos += 1;
                }

                for line in lines {
                    println!("{line}");
                }

                Ok(())
            },
        )
        .await?;

    let s = |n: usize| if n == 1 { "" } else { "s" };
    println!(
        "{matches} outdated declaration{} in {repos} repo{}",
        s(matches),
        s(repos)
    );
    Ok(())
}

/// Find every versioned declaration in a workspace whose requirement does not
/// admit the latest version on the index, together with that version.
async fn outdated<'a>(
    crates: &'a Crates,
    spec: Option<&Spec>,
    index: &Index,
) -> Result<Vec<(Found<'a>, Version)>> {
    let mut output = Vec::new();

    for found in find(crates, spec)? {
        let Some(version) = found.version else {
            continue;
        };

        let Ok(requirement) = VersionReq::parse(version) else {
            tracing::warn!(
                "{}: Ignoring unparseable requirement `{version}` for `{}`",
                found.manifest,
                found.package
            );
            continue;
        };

        let Some(releases) = index.releases(found.package).await? else {
            continue;
        };

        let Some(latest) = latest(&releases, &requirement) else {
            continue;
        };

        if !requirement.matches(latest) {
            let latest = latest.clone();
            output.push((found, latest));
        }
    }

    Ok(output)
}

/// A version published on an index.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Release {
    version: Version,
    yanked: bool,
}

/// The newest non-yanked release, ignoring pre-releases unless the
/// requirement names one.
fn latest<'a>(releases: &'a [Release], requirement: &VersionReq) -> Option<&'a Version> {
    let pre = requirement.comparators.iter().any(|c| !c.pre.is_empty());

    releases
        .iter()
        .filter(|r| !r.yanked && (pre || r.version.pre.is_empty()))
        .map(|r| &r.version)
        .max()
}

/// The path of a crate's file in an index, relative to its root.
fn index_path(name: &str) -> String {
    let name = name.to_ascii_lowercase();

    match name.len() {
        1 => format!("1/{name}"),
        2 => format!("2/{name}"),
        3 => format!("3/{}/{name}", &name[..1]),
        _ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
    }
}

/// Parse the JSON lines of an index file.
fn parse_releases(input: &str) -> Result<Vec<Release>> {
    #[derive(serde::Deserialize)]
    struct Line {
        vers: String,
        #[serde(default)]
        yanked: bool,
    }

    let mut releases = Vec::new();

    for line in input.lines() {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        let line: Line = serde_json::from_str(line)?;

        let Ok(version) = Version::parse(&line.vers) else {
            tracing::warn!("Ignoring unparseable version `{}` on index", line.vers);
            continue;
        };

        releases.push(Release {
            version,
            yanked: line.yanked,
        });
    }

    Ok(releases)
}

enum Source {
    Sparse { client: reqwest::Client, base: Url },
    Local(PathBuf),
}

type Cached = Rc<OnceCell<Option<Rc<[Release]>>>>;

/// A registry index, looking up each crate at most once.
struct Index {
    source: Source,
    cache: RefCell<HashMap<String, Cached>>,
}

impl Index {
    fn open(index: &str) -> Result<Self> {
        let source = if let Some(url) = index.strip_prefix("sparse+") {
            let mut base = Url::parse(url).with_context(|| anyhow!("Invalid index `{index}`"))?;

            if !base.path().ends_with('/') {
                base.set_path(&format!("{}/", base.path()));
            }

            let client = reqwest::Client::builder()
                .user_agent(crate::USER_AGENT.clone())
                .build()?;

            Source::Sparse { client, base }
        } else if index.starts_with("file://") {
            let url = Url::parse(index).with_context(|| anyhow!("Invalid index `{index}`"))?;

            let path = url
                .to_file_path()
                .map_err(|()| anyhow!("Invalid index `{index}`: not a local path"))?;

            Source::Local(path)
        } else if index.contains("://") {
            bail!(
                "Unsupported index `{index}`, expected `sparse+<url>`, `file://<path>` or a directory"
            );
        } else {
            Source::Local(PathBuf::from(index))
        };

        Ok(Self {
            source,
            cache: RefCell::new(HashMap::new()),
        })
    }

    /// The releases of a crate, or `None` with a warning if the index does
    /// not have it.
    async fn releases(&self, name: &str) -> Result<Option<Rc<[Release]>>> {
        let cell = self
            .cache
            .borrow_mut()
            .entry(name.to_ascii_lowercase())
            .or_default()
            .clone();

        let releases = cell
            .get_or_try_init(async || {
                let releases = self.fetch(name).await?;

                if releases.is_none() {
                    tracing::warn!("Crate `{name}` was not found on the index");
                }

                Ok::<_, anyhow::Error>(releases.map(Rc::from))
            })
            .await?;

        Ok(releases.clone())
    }

    async fn fetch(&self, name: &str) -> Result<Option<Vec<Release>>> {
        let path = index_path(name);

        let input = match &self.source {
            Source::Sparse { client, base } => {
                let url = base.join(&path)?;

                let response = client
                    .get(url.clone())
                    .send()
                    .await
                    .with_context(|| anyhow!("Fetching {url}"))?;

                if matches!(
                    response.status(),
                    StatusCode::NOT_FOUND
                        | StatusCode::GONE
                        | StatusCode::UNAVAILABLE_FOR_LEGAL_REASONS
                ) {
                    return Ok(None);
                }

                let response = response
                    .error_for_status()
                    .with_context(|| anyhow!("Fetching {url}"))?;

                response
                    .text()
                    .await
                    .with_context(|| anyhow!("Fetching {url}"))?
            }
            Source::Local(root) => {
                let path = root.join(&path);

                match fs::read_to_string(&path) {
                    Ok(input) => input,
                    Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
                    Err(e) => {
                        return Err(e).with_context(|| anyhow!("Reading {}", path.display()));
                    }
                }
            }
        };

        let releases =
            parse_releases(&input).with_context(|| anyhow!("Parsing index entry for `{name}`"))?;
        Ok(Some(releases))
    }
}

/// A declaration of the searched for crate.
struct Found<'a> {
    manifest: &'a RelativePath,
    table: String,
    key: &'a str,
    package: &'a str,
    version: Option<&'a str>,
    source: Option<&'static str>,
    /// Source keys with their values, shown in place of `source` when set.
    details: Vec<(&'static str, &'a str)>,
    renamed: bool,
    workspace: bool,
}

impl fmt::Display for Found<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.table, self.key)?;

        if self.renamed {
            write!(f, " (package = {:?})", self.package)?;
        }

        match (self.version, self.source) {
            (Some(version), _) => write!(f, " = {version:?}")?,
            _ if !self.details.is_empty() => {}
            (None, Some(source)) => write!(f, " = {source}")?,
            (None, None) => write!(f, " = (no version)")?,
        }

        if !self.details.is_empty() {
            write!(f, " (")?;

            for (n, (key, value)) in self.details.iter().enumerate() {
                if n > 0 {
                    write!(f, ", ")?;
                }

                write!(f, "{key} = {value:?}")?;
            }

            write!(f, ")")?;
        }

        if self.workspace {
            write!(f, " (workspace)")?;
        }

        Ok(())
    }
}

/// Find every declaration of the crate in `spec` across the manifests of a
/// workspace which matches its selector, or every declaration without a spec.
fn find<'a>(crates: &'a Crates, spec: Option<&Spec>) -> Result<Vec<Found<'a>>> {
    let mut output = Vec::new();

    for manifest in crates.manifests() {
        for (table, dependencies) in tables(manifest, crates) {
            for dep in dependencies.iter() {
                let package = *dep.package()?;
                let version = dep.lookup(DependencyItem::version)?.map(|v| *v);

                if !spec.is_none_or(|spec| spec_matches(spec, package, version)) {
                    continue;
                }

                output.push(Found {
                    manifest: manifest.path(),
                    table: table.clone(),
                    key: dep.key(),
                    package,
                    version,
                    source: dep.lookup(DependencyItem::source)?.map(|v| *v),
                    details: Vec::new(),
                    renamed: dep.key() != package,
                    workspace: dep.is_workspace(),
                });
            }
        }
    }

    Ok(output)
}

/// Find every `[patch.<source>]` entry and legacy `[replace]` entry across the
/// manifests of a workspace which overrides the crate in `spec` and matches
/// its selector, or every entry without a spec.
fn find_patched<'a>(crates: &'a Crates, spec: Option<&Spec>) -> Result<Vec<Found<'a>>> {
    let mut output = Vec::new();

    for manifest in crates.manifests() {
        let mut tables = manifest
            .patches(crates)
            .into_iter()
            .map(|(source, entries)| (format!("patch.{}", toml_key(source)), entries, false))
            .collect::<Vec<_>>();

        tables.extend(
            manifest
                .replace(crates)
                .map(|entries| ("replace".to_owned(), entries, true)),
        );

        for (table, entries, replace) in tables {
            for entry in entries.iter() {
                let version = entry.lookup(DependencyItem::version)?.map(|v| *v);

                let (package, selected) = if replace {
                    replace_spec(entry.key())
                } else {
                    (*entry.package()?, version)
                };

                if !spec.is_none_or(|spec| spec_matches(spec, package, selected)) {
                    continue;
                }

                let details = entry
                    .lookup(|item| Some(item.source_details()))?
                    .map(|v| v.into_value())
                    .unwrap_or_default();

                output.push(Found {
                    manifest: manifest.path(),
                    table: table.clone(),
                    key: entry.key(),
                    package,
                    version,
                    source: entry.lookup(DependencyItem::source)?.map(|v| *v),
                    details,
                    renamed: !replace && entry.key() != package,
                    workspace: false,
                });
            }
        }
    }

    Ok(output)
}

/// The package name and version of a `[replace]` package id spec such as
/// `foo:1.2.3` or `https://github.com/rust-lang/crates.io-index#foo:1.2.3`.
fn replace_spec(key: &str) -> (&str, Option<&str>) {
    let key = key.rsplit_once('#').map_or(key, |(_, key)| key);

    match key.split_once(['@', ':']) {
        Some((package, version)) => (package, Some(version)),
        None => (key, None),
    }
}

/// Test if a declaration of `package` with the given version requirement
/// matches `spec`. Declarations without a version only match without a
/// selector.
fn spec_matches(spec: &Spec, package: &str, version: Option<&str>) -> bool {
    if package != spec.name {
        return false;
    }

    match (&spec.selector, version) {
        (None, _) => true,
        (Some(selector), Some(version)) => matches(selector, version),
        (Some(_), None) => false,
    }
}

/// Every dependency table in a manifest, with its display name.
fn tables<'a>(manifest: &'a Manifest, crates: &'a Crates) -> Vec<(String, Dependencies<'a>)> {
    let mut output = Vec::new();

    let direct = [
        (DEPENDENCIES, manifest.dependencies(crates)),
        (DEV_DEPENDENCIES, manifest.dev_dependencies(crates)),
        (BUILD_DEPENDENCIES, manifest.build_dependencies(crates)),
    ];

    for (kind, dependencies) in direct {
        output.extend(dependencies.map(|d| (kind.to_owned(), d)));
    }

    for kind in DEPS {
        for (cfg, dependencies) in manifest.target_dependencies(crates, kind) {
            output.push((format!("target.{}.{kind}", toml_key(cfg)), dependencies));
        }
    }

    if let Some(dependencies) = manifest.workspace_dependencies(crates) {
        output.push(("workspace.dependencies".to_owned(), dependencies));
    }

    output
}

fn toml_key(key: &str) -> String {
    let bare = !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));

    if bare {
        key.to_owned()
    } else {
        format!("'{key}'")
    }
}

/// Test if the lowest version admitted by the declared `requirement` matches
/// `selector`. Requirements which cannot be parsed never match.
fn matches(selector: &VersionReq, requirement: &str) -> bool {
    let Ok(requirement) = VersionReq::parse(requirement) else {
        tracing::warn!("Ignoring unparseable requirement `{requirement}`");
        return false;
    };

    selector.matches(&lowest(&requirement))
}

/// The lowest version a requirement admits, with missing minor and patch
/// components taken as 0.
fn lowest(requirement: &VersionReq) -> Version {
    let mut lowest = Version::new(0, 0, 0);

    for c in &requirement.comparators {
        let (minor, patch) = (c.minor.unwrap_or(0), c.patch.unwrap_or(0));

        let version = match c.op {
            Op::Greater => match (c.minor, c.patch) {
                (Some(minor), Some(patch)) if c.pre.is_empty() => {
                    Version::new(c.major, minor, patch + 1)
                }
                (Some(_), Some(_)) => Version::new(c.major, minor, patch),
                (Some(minor), None) => Version::new(c.major, minor + 1, 0),
                (None, _) => Version::new(c.major + 1, 0, 0),
            },
            Op::Less | Op::LessEq => continue,
            _ => {
                let mut version = Version::new(c.major, minor, patch);
                version.pre = c.pre.clone();
                version
            }
        };

        if version > lowest {
            lowest = version;
        }
    }

    lowest
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crates(manifests: &[(&str, &str)]) -> Crates {
        let manifests = manifests
            .iter()
            .map(|(path, input)| Manifest::parse_for_test(path, input).unwrap())
            .collect();

        Crates::from_manifests_for_test(manifests)
    }

    fn search(crates: &Crates, spec: &str) -> Vec<String> {
        let spec = parse_spec(spec).unwrap();

        find(crates, Some(&spec))
            .unwrap()
            .into_iter()
            .map(|found| format!("{} {found}", found.manifest))
            .collect()
    }

    #[test]
    fn plain_and_tables() {
        let crates = crates(&[(
            "repo/Cargo.toml",
            r#"
            [package]
            name = "a"

            [dependencies]
            serde = "1.0.100"
            other = "1"

            [dev-dependencies]
            serde = { version = "1.0.150", features = ["derive"] }

            [build-dependencies]
            serde = "=1.0.200"
            "#,
        )]);

        assert_eq!(
            search(&crates, "serde"),
            [
                "repo/Cargo.toml [dependencies] serde = \"1.0.100\"",
                "repo/Cargo.toml [dev-dependencies] serde = \"1.0.150\"",
                "repo/Cargo.toml [build-dependencies] serde = \"=1.0.200\"",
            ]
        );
        assert!(search(&crates, "serde_json").is_empty());
    }

    #[test]
    fn renamed() {
        let crates = crates(&[(
            "Cargo.toml",
            r#"
            [package]
            name = "a"

            [dependencies]
            serde1 = { package = "serde", version = "1.0" }
            serde = { package = "not-serde", version = "1.0" }
            "#,
        )]);

        assert_eq!(
            search(&crates, "serde"),
            ["Cargo.toml [dependencies] serde1 (package = \"serde\") = \"1.0\""]
        );
    }

    #[test]
    fn workspace_inherited() {
        let crates = crates(&[
            (
                "Cargo.toml",
                r#"
                [workspace]
                members = ["a"]

                [workspace.dependencies]
                tokio = "0.2.5"
                sd = { package = "serde", version = "1.0.50" }
                "#,
            ),
            (
                "a/Cargo.toml",
                r#"
                [package]
                name = "a"

                [dependencies]
                tokio = { workspace = true, features = ["full"] }
                sd = { workspace = true }
                "#,
            ),
        ]);

        assert_eq!(
            search(&crates, "tokio@^0.2"),
            [
                "Cargo.toml [workspace.dependencies] tokio = \"0.2.5\"",
                "a/Cargo.toml [dependencies] tokio = \"0.2.5\" (workspace)",
            ]
        );
        assert!(search(&crates, "tokio@>=1").is_empty());
        assert_eq!(
            search(&crates, "serde@<1.0.200"),
            [
                "Cargo.toml [workspace.dependencies] sd (package = \"serde\") = \"1.0.50\"",
                "a/Cargo.toml [dependencies] sd (package = \"serde\") = \"1.0.50\" (workspace)",
            ]
        );
    }

    #[test]
    fn target_specific() {
        let crates = crates(&[(
            "Cargo.toml",
            r#"
            [package]
            name = "a"

            [target.'cfg(windows)'.dependencies]
            winapi = "0.3"

            [target.x86_64-pc-windows-msvc.dev-dependencies]
            winapi = "0.2"
            "#,
        )]);

        assert_eq!(
            search(&crates, "winapi@<0.3"),
            ["Cargo.toml [target.x86_64-pc-windows-msvc.dev-dependencies] winapi = \"0.2\""]
        );
        assert_eq!(
            search(&crates, "winapi"),
            [
                "Cargo.toml [target.'cfg(windows)'.dependencies] winapi = \"0.3\"",
                "Cargo.toml [target.x86_64-pc-windows-msvc.dev-dependencies] winapi = \"0.2\"",
            ]
        );
    }

    #[test]
    fn selector() {
        let crates = crates(&[(
            "Cargo.toml",
            r#"
            [package]
            name = "a"

            [dependencies]
            serde = "1.2"
            "#,
        )]);

        // `1.2` admits 1.2.0 at the lowest.
        assert_eq!(search(&crates, "serde@<1.2.1").len(), 1);
        assert_eq!(search(&crates, "serde@=1.2.0").len(), 1);
        assert!(search(&crates, "serde@<1.2.0").is_empty());
        assert!(search(&crates, "serde@>=2").is_empty());
    }

    #[test]
    fn path_dependency() {
        let crates = crates(&[(
            "Cargo.toml",
            r#"
            [package]
            name = "a"

            [dependencies]
            local = { path = "../local" }
            remote = { git = "https://example.com/remote" }
            both = { path = "../both", version = "0.1" }
            "#,
        )]);

        assert_eq!(
            search(&crates, "local"),
            ["Cargo.toml [dependencies] local = path"]
        );
        assert_eq!(
            search(&crates, "remote"),
            ["Cargo.toml [dependencies] remote = git"]
        );
        assert!(search(&crates, "local@*").is_empty());
        assert!(search(&crates, "remote@<1").is_empty());
        assert_eq!(
            search(&crates, "both@<1"),
            ["Cargo.toml [dependencies] both = \"0.1\""]
        );
    }

    #[test]
    fn lowest_versions() {
        let lowest = |req: &str| lowest(&VersionReq::parse(req).unwrap()).to_string();

        assert_eq!(lowest("1.2"), "1.2.0");
        assert_eq!(lowest("^0.4.3"), "0.4.3");
        assert_eq!(lowest("~1"), "1.0.0");
        assert_eq!(lowest("*"), "0.0.0");
        assert_eq!(lowest("1.*"), "1.0.0");
        assert_eq!(lowest(">=1.2, <2"), "1.2.0");
        assert_eq!(lowest("<2"), "0.0.0");
        assert_eq!(lowest(">1.2.3"), "1.2.4");
        assert_eq!(lowest(">1.2"), "1.3.0");
        assert_eq!(lowest(">1"), "2.0.0");
        assert_eq!(lowest("=1.0.0-beta.1"), "1.0.0-beta.1");
    }

    #[test]
    fn spec_forms() {
        let spec = |input: &str| {
            let spec = parse_spec(input).unwrap();
            (spec.name, spec.selector.map(|s| s.to_string()))
        };

        let sqll = || ("sqll".to_owned(), Some("<=0.14.7".to_owned()));

        assert_eq!(spec("sqll <= 0.14.7"), sqll());
        assert_eq!(spec("sqll<=0.14.7"), sqll());
        assert_eq!(spec("sqll@<=0.14.7"), sqll());
        assert_eq!(spec("sqll @ <= 0.14.7"), sqll());
        assert_eq!(spec(&["sqll", "<=0.14.7"].join(" ")), sqll());
        assert_eq!(spec(&["sqll", "<=", "0.14.7"].join(" ")), sqll());
        assert_eq!(spec("serde_json"), ("serde_json".to_owned(), None));
        assert_eq!(spec("tokio-util "), ("tokio-util".to_owned(), None));
        assert_eq!(
            spec("serde >=1, <2"),
            ("serde".to_owned(), Some(">=1, <2".to_owned()))
        );
    }

    #[test]
    fn invalid_selector() {
        let error = parse_spec("serde@not a version").unwrap_err();
        assert!(error.contains("`not a version`"), "{error}");
        let error = parse_spec("serde <= banana").unwrap_err();
        assert!(error.contains("`<= banana`"), "{error}");
        assert!(parse_spec("@1").is_err());
        assert!(parse_spec("<=1").is_err());
        assert!(parse_spec("").is_err());
        assert!(parse_spec("serde@").is_err());
    }

    #[test]
    fn opts() {
        let parse = |args: &[&str]| Opts::try_parse_from([&["deps"][..], args].concat());

        assert!(parse(&[]).is_err());
        assert!(parse(&["--index", "dir", "serde"]).is_err());

        let opts = parse(&["--outdated"]).unwrap();
        assert!(opts.spec.is_empty());
        assert_eq!(opts.index, CRATES_IO);

        let opts = parse(&["--outdated", "--index", "dir", "serde", "<2"]).unwrap();
        assert_eq!(opts.spec, ["serde", "<2"]);
        assert_eq!(opts.index, "dir");

        let opts = parse(&["--patched"]).unwrap();
        assert!(opts.patched && opts.spec.is_empty());
        let opts = parse(&["--patched", "serde", "<2"]).unwrap();
        assert_eq!(opts.spec, ["serde", "<2"]);
        assert!(parse(&["--patched", "--outdated"]).is_err());
        assert!(parse(&["--patched", "--index", "dir"]).is_err());
    }

    fn search_patched(crates: &Crates, spec: Option<&str>) -> Vec<String> {
        let spec = spec.map(|spec| parse_spec(spec).unwrap());

        find_patched(crates, spec.as_ref())
            .unwrap()
            .into_iter()
            .map(|found| format!("{} {found}", found.manifest))
            .collect()
    }

    #[test]
    fn patched() {
        let crates = crates(&[
            (
                "Cargo.toml",
                r#"
                [workspace]
                members = ["a"]

                [workspace.dependencies]
                serde = "1.0.100"

                [patch.crates-io]
                serde = { path = "../serde" }
                tokio = { git = "https://github.com/tokio-rs/tokio", branch = "fix", version = "1.40.0" }
                sd = { package = "serde_derive", git = "https://example.com/serde", rev = "abc123" }
                not-serde = { package = "other", version = "1.0.0", path = "../other" }

                [patch."https://github.com/example/repo"]
                foo = { path = "../foo", version = "0.2.1" }
                bar = { git = "https://example.com/bar.git", tag = "v1" }

                [replace]
                "serde:1.0.150" = { git = "https://github.com/serde-rs/serde", rev = "def456" }
                "https://github.com/rust-lang/crates.io-index#foo:0.1.0" = { path = "../foo" }
                "#,
            ),
            (
                "a/Cargo.toml",
                r#"
                [package]
                name = "a"

                [dependencies]
                serde = { workspace = true }

                [patch.crates-io]
                bytes = { path = "../bytes" }
                "#,
            ),
        ]);

        assert_eq!(
            search_patched(&crates, None),
            [
                "Cargo.toml [patch.crates-io] serde (path = \"../serde\")",
                "Cargo.toml [patch.crates-io] tokio = \"1.40.0\" (git = \"https://github.com/tokio-rs/tokio\", branch = \"fix\")",
                "Cargo.toml [patch.crates-io] sd (package = \"serde_derive\") (git = \"https://example.com/serde\", rev = \"abc123\")",
                "Cargo.toml [patch.crates-io] not-serde (package = \"other\") = \"1.0.0\" (path = \"../other\")",
                "Cargo.toml [patch.'https://github.com/example/repo'] foo = \"0.2.1\" (path = \"../foo\")",
                "Cargo.toml [patch.'https://github.com/example/repo'] bar (git = \"https://example.com/bar.git\", tag = \"v1\")",
                "Cargo.toml [replace] serde:1.0.150 (git = \"https://github.com/serde-rs/serde\", rev = \"def456\")",
                "Cargo.toml [replace] https://github.com/rust-lang/crates.io-index#foo:0.1.0 (path = \"../foo\")",
                "a/Cargo.toml [patch.crates-io] bytes (path = \"../bytes\")",
            ]
        );

        // Renamed entries match their package, not their key.
        assert_eq!(
            search_patched(&crates, Some("serde_derive")),
            [
                "Cargo.toml [patch.crates-io] sd (package = \"serde_derive\") (git = \"https://example.com/serde\", rev = \"abc123\")"
            ]
        );
        assert!(search_patched(&crates, Some("not-serde")).is_empty());
        assert!(search_patched(&crates, Some("sd")).is_empty());

        assert_eq!(
            search_patched(&crates, Some("serde")),
            [
                "Cargo.toml [patch.crates-io] serde (path = \"../serde\")",
                "Cargo.toml [replace] serde:1.0.150 (git = \"https://github.com/serde-rs/serde\", rev = \"def456\")",
            ]
        );

        // Entries without a version only match without a selector.
        assert_eq!(
            search_patched(&crates, Some("serde <1.0.200")),
            [
                "Cargo.toml [replace] serde:1.0.150 (git = \"https://github.com/serde-rs/serde\", rev = \"def456\")"
            ]
        );
        assert!(search_patched(&crates, Some("serde >=1.0.200")).is_empty());
        assert_eq!(search_patched(&crates, Some("tokio@^1.40")).len(), 1);
        assert!(search_patched(&crates, Some("tokio@<1.40")).is_empty());
        assert_eq!(
            search_patched(&crates, Some("foo <1")),
            [
                "Cargo.toml [patch.'https://github.com/example/repo'] foo = \"0.2.1\" (path = \"../foo\")",
                "Cargo.toml [replace] https://github.com/rust-lang/crates.io-index#foo:0.1.0 (path = \"../foo\")",
            ]
        );
        assert!(search_patched(&crates, Some("bar *")).is_empty());

        // Plain searches are unaffected by patches.
        assert_eq!(
            search(&crates, "serde"),
            [
                "Cargo.toml [workspace.dependencies] serde = \"1.0.100\"",
                "a/Cargo.toml [dependencies] serde = \"1.0.100\" (workspace)",
            ]
        );
    }

    #[test]
    fn replace_specs() {
        assert_eq!(replace_spec("foo:1.2.3"), ("foo", Some("1.2.3")));
        assert_eq!(replace_spec("foo@1.2.3"), ("foo", Some("1.2.3")));
        assert_eq!(replace_spec("foo"), ("foo", None));
        assert_eq!(
            replace_spec("https://github.com/rust-lang/crates.io-index#foo:0.1.0"),
            ("foo", Some("0.1.0"))
        );
    }

    #[test]
    fn index_paths() {
        assert_eq!(index_path("a"), "1/a");
        assert_eq!(index_path("xz"), "2/xz");
        assert_eq!(index_path("syn"), "3/s/syn");
        assert_eq!(index_path("serde"), "se/rd/serde");
        assert_eq!(index_path("Inflector"), "in/fl/inflector");
        assert_eq!(index_path("tokio-util"), "to/ki/tokio-util");
    }

    fn releases(input: &[(&str, bool)]) -> Vec<Release> {
        input
            .iter()
            .map(|&(version, yanked)| Release {
                version: Version::parse(version).unwrap(),
                yanked,
            })
            .collect()
    }

    #[test]
    fn latest_version() {
        let releases = releases(&[
            ("1.0.0", false),
            ("1.2.0", false),
            ("1.3.0", true),
            ("2.0.0-beta.1", false),
            ("1.10.0-rc.1", false),
        ]);

        let latest =
            |req: &str| latest(&releases, &VersionReq::parse(req).unwrap()).map(|v| v.to_string());

        assert_eq!(latest("1").as_deref(), Some("1.2.0"));
        assert_eq!(latest("=1.10.0-rc.1").as_deref(), Some("2.0.0-beta.1"));
        assert_eq!(latest("^2.0.0-beta.1").as_deref(), Some("2.0.0-beta.1"));

        let only_yanked = self::releases(&[("0.1.0", true), ("0.2.0-alpha", false)]);
        assert_eq!(
            super::latest(&only_yanked, &VersionReq::parse("0.1").unwrap()),
            None
        );
    }

    #[test]
    fn parse_index_lines() {
        let input = concat!(
            "{\"name\":\"a\",\"vers\":\"0.1.0\",\"deps\":[],\"cksum\":\"x\",\"features\":{},\"yanked\":false}\n",
            "\n",
            "{\"name\":\"a\",\"vers\":\"0.2.0\",\"yanked\":true,\"v\":2}\n",
            "{\"name\":\"a\",\"vers\":\"not-a-version\",\"yanked\":false}\n",
        );

        assert_eq!(
            parse_releases(input).unwrap(),
            releases(&[("0.1.0", false), ("0.2.0", true)])
        );
        assert!(parse_releases("{").is_err());
    }

    #[test]
    fn index_sources() {
        assert!(matches!(
            Index::open(CRATES_IO).unwrap().source,
            Source::Sparse { ref base, .. } if base.as_str() == "https://index.crates.io/"
        ));
        assert!(matches!(
            Index::open("sparse+http://localhost/index").unwrap().source,
            Source::Sparse { ref base, .. } if base.as_str() == "http://localhost/index/"
        ));
        assert!(matches!(
            Index::open("file:///srv/index").unwrap().source,
            Source::Local(ref path) if path == std::path::Path::new("/srv/index")
        ));
        assert!(matches!(
            Index::open("some/index").unwrap().source,
            Source::Local(ref path) if path == std::path::Path::new("some/index")
        ));
        assert!(Index::open("https://index.crates.io/").is_err());
    }

    fn write_index(root: &std::path::Path, name: &str, versions: &[(&str, bool)]) {
        let path = root.join(index_path(name));
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        let lines = versions
            .iter()
            .map(|(vers, yanked)| {
                format!(
                    "{{\"name\":\"{name}\",\"vers\":\"{vers}\",\"deps\":[],\"yanked\":{yanked}}}\n"
                )
            })
            .collect::<String>();

        fs::write(path, lines).unwrap();
    }

    async fn search_outdated(crates: &Crates, spec: Option<&str>, index: &Index) -> Vec<String> {
        let spec = spec.map(|spec| parse_spec(spec).unwrap());

        outdated(crates, spec.as_ref(), index)
            .await
            .unwrap()
            .into_iter()
            .map(|(found, latest)| format!("{} {found} (latest {latest})", found.manifest))
            .collect()
    }

    #[tokio::test]
    async fn outdated_local_index() {
        let dir = tempfile::tempdir().unwrap();

        write_index(
            dir.path(),
            "serde",
            &[("1.0.100", false), ("1.0.228", false), ("1.0.229", true)],
        );
        write_index(dir.path(), "syn", &[("1.0.109", false), ("2.0.100", false)]);
        write_index(
            dir.path(),
            "rand",
            &[("0.8.5", false), ("0.9.0-beta.1", false)],
        );
        write_index(dir.path(), "bytes", &[("1.10.1", false)]);

        let crates = crates(&[
            (
                "Cargo.toml",
                r#"
                [workspace]
                members = ["a"]

                [workspace.dependencies]
                syn = "1.0.50"
                "#,
            ),
            (
                "a/Cargo.toml",
                r#"
                [package]
                name = "a"

                [dependencies]
                serde = "1.0.100"
                pinned = { package = "serde", version = "=1.0.100" }
                syn = { workspace = true }
                rand = "0.8"
                bytes = "1"
                local = { path = "../local" }
                missing = "1"

                [target.'cfg(unix)'.dev-dependencies]
                syn = ">=1, <3"
                "#,
            ),
        ]);

        let index = Index::open(&format!("file://{}", dir.path().display())).unwrap();

        assert_eq!(
            search_outdated(&crates, None, &index).await,
            [
                "Cargo.toml [workspace.dependencies] syn = \"1.0.50\" (latest 2.0.100)",
                "a/Cargo.toml [dependencies] pinned (package = \"serde\") = \"=1.0.100\" (latest 1.0.228)",
                "a/Cargo.toml [dependencies] syn = \"1.0.50\" (workspace) (latest 2.0.100)",
            ]
        );

        assert_eq!(
            search_outdated(&crates, Some("serde"), &index).await,
            [
                "a/Cargo.toml [dependencies] pinned (package = \"serde\") = \"=1.0.100\" (latest 1.0.228)"
            ]
        );
        assert!(
            search_outdated(&crates, Some("serde >=1.0.200"), &index)
                .await
                .is_empty()
        );

        // Every crate was read once, unknown ones included.
        let cached = index.cache.borrow();
        let mut names = cached.keys().map(String::as_str).collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, ["bytes", "missing", "rand", "serde", "syn"]);
        assert!(cached["missing"].get().unwrap().is_none());
    }
}
