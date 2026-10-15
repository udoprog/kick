use core::fmt;
use std::cell::Cell;

use anyhow::Result;
use clap::Parser;
use relative_path::RelativePath;
use semver::{Op, Version, VersionReq};

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
    /// The crate to look for, optionally followed by `@` and a semver selector
    /// such as `serde@<1.0.200` or `tokio@^0.2`.
    ///
    /// A declaration matches the selector when the lowest version its
    /// requirement admits does, so `1.2` is tested as `1.2.0`. Path and git
    /// dependencies without a version only match when no selector is given.
    #[arg(value_name = "CRATE[@SELECTOR]", value_parser = parse_spec)]
    spec: Spec,
}

#[derive(Debug, Clone)]
struct Spec {
    name: String,
    selector: Option<VersionReq>,
}

fn parse_spec(input: &str) -> Result<Spec, String> {
    let (name, selector) = match input.split_once('@') {
        Some((name, selector)) => {
            let selector = VersionReq::parse(selector)
                .map_err(|error| format!("invalid selector `{selector}`: {error}"))?;
            (name, Some(selector))
        }
        None => (input, None),
    };

    if name.is_empty() {
        return Err("missing crate name".to_owned());
    }

    Ok(Spec {
        name: name.to_owned(),
        selector,
    })
}

pub(crate) fn entry(with_repos: &mut WithRepos<'_>, opts: &Opts) -> Result<()> {
    let matches = Cell::new(0usize);
    let repos = Cell::new(0usize);

    with_repos.run(
        "find dependents",
        format_args!("deps: {opts:?}"),
        |cx, repo| {
            let count = deps(cx, repo, &opts.spec)?;

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

    let found = find(&crates, spec)?;

    for found in &found {
        let manifest = found
            .manifest
            .strip_prefix(repo.path())
            .unwrap_or(found.manifest);

        println!("{}: {manifest} {found}", repo.path());
    }

    Ok(found.len())
}

/// A declaration of the searched for crate.
struct Found<'a> {
    manifest: &'a RelativePath,
    table: String,
    key: &'a str,
    package: &'a str,
    version: Option<&'a str>,
    source: Option<&'static str>,
    workspace: bool,
}

impl fmt::Display for Found<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}] {}", self.table, self.key)?;

        if self.key != self.package {
            write!(f, " (package = {:?})", self.package)?;
        }

        match (self.version, self.source) {
            (Some(version), _) => write!(f, " = {version:?}")?,
            (None, Some(source)) => write!(f, " = {source}")?,
            (None, None) => write!(f, " = (no version)")?,
        }

        if self.workspace {
            write!(f, " (workspace)")?;
        }

        Ok(())
    }
}

/// Find every declaration of the crate in `spec` across the manifests of a
/// workspace which matches its selector.
fn find<'a>(crates: &'a Crates, spec: &Spec) -> Result<Vec<Found<'a>>> {
    let mut output = Vec::new();

    for manifest in crates.manifests() {
        for (table, dependencies) in tables(manifest, crates) {
            for dep in dependencies.iter() {
                let package = *dep.package()?;

                if package != spec.name {
                    continue;
                }

                let version = dep.lookup(DependencyItem::version)?.map(|v| *v);

                if let Some(selector) = &spec.selector {
                    let Some(version) = version else {
                        continue;
                    };

                    if !matches(selector, version) {
                        continue;
                    }
                }

                output.push(Found {
                    manifest: manifest.path(),
                    table: table.clone(),
                    key: dep.key(),
                    package,
                    version,
                    source: dep.lookup(DependencyItem::source)?.map(|v| *v),
                    workspace: dep.is_workspace(),
                });
            }
        }
    }

    Ok(output)
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

        find(crates, &spec)
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
    fn invalid_selector() {
        assert!(parse_spec("serde@not a version").is_err());
        assert!(parse_spec("@1").is_err());
        assert!(parse_spec("serde@").is_err());
    }
}
