//! The `kick cat` command, which prints the effective name, configuration and
//! templating variables of each repo.

use std::io::{self, Write};
use std::path::Path;

use anyhow::Result;
use clap::Parser;

use crate::cli::WithRepos;
use crate::config::Config;
use crate::ctxt::Ctxt;
use crate::model::{Repo, RepoRef};

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {}

pub(crate) fn entry(with_repos: &mut WithRepos<'_>, _: &Opts) -> Result<()> {
    let mut first = true;

    with_repos.run("cat", "cat", |cx, repo| {
        let variables = variables(cx, repo)?;
        let document = document(cx.config, cx.paths.root, repo, variables);
        let document = toml::to_string(&document)?;

        let mut o = io::stdout().lock();

        if !std::mem::take(&mut first) {
            writeln!(o)?;
        }

        writeln!(o, "# {}", cx.display(repo.path()))?;
        write!(o, "{document}")?;
        Ok(())
    })
}

/// The variables templates are rendered with, which for a repo without a
/// Cargo package are only those configured.
fn variables(cx: &Ctxt<'_>, repo: &Repo) -> Result<toml::Table> {
    let Some(crates) = repo.try_workspace(cx)? else {
        return Ok(cx.config.variables(repo));
    };

    let package = crates.primary_package()?.ensure_package()?;
    let params = cx.repo_params(package, repo)?;
    Ok(toml::Table::try_from(&params)?)
}

fn document(
    config: &Config<'_>,
    root: &Path,
    repo: &RepoRef,
    variables: toml::Table,
) -> toml::Table {
    let mut document = toml::Table::new();

    if let Some(name) = config.selection_name(root, repo.path()) {
        document.insert("name".into(), toml::Value::String(name.into_owned()));
    }

    document.insert("config".into(), toml::Value::Table(config.effective(repo)));
    document.insert("variables".into(), toml::Value::Table(variables));
    document
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::config;
    use crate::ctxt::Paths;
    use crate::model::{Repo, RepoSource};
    use crate::templates::Templating;

    #[test]
    fn layers_are_merged() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        fs::create_dir_all(root.join("repos/a")).unwrap();

        fs::write(
            root.join("Kick.toml"),
            "license = \"MIT\"\nauthors = [\"one\"]\n\
             [variables]\nbase = 1\nshared = \"base\"\n\
             [repo.\"repos/a\"]\nurl = \"https://example.com/a\"\nalias = \"short\"\n\
             [repo.\"repos/a\".variables]\nshared = \"repo\"\n",
        )
        .unwrap();

        fs::write(
            root.join("repos/a/Kick.toml"),
            "license = \"Apache-2.0\"\nauthors = [\"two\"]\n",
        )
        .unwrap();

        let templating = Templating::new().unwrap();
        let defaults = config::defaults();

        let paths = Paths {
            root: &root,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let loaded = config::load_all(paths, &templating, &defaults);
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);

        let repo = Repo::new(
            [RepoSource::Git],
            "repos/a".into(),
            url::Url::parse("https://example.com/a").unwrap(),
        );

        let variables = loaded.config.variables(&repo);
        let document = super::document(&loaded.config, &root, &repo, variables);

        let expected: toml::Table = toml::from_str(
            "license = \"Apache-2.0\"\nauthors = [\"one\", \"two\"]\n\
             url = \"https://example.com/a\"\nalias = \"short\"\n\
             [variables]\nbase = 1\nshared = \"repo\"\n",
        )
        .unwrap();

        assert_eq!(document["name"].as_str(), Some("short"));
        assert_eq!(document["config"].as_table(), Some(&expected));

        let variables = document["variables"].as_table().unwrap();
        assert_eq!(variables["base"].as_integer(), Some(1));
        assert_eq!(variables["shared"].as_str(), Some("repo"));
    }
}
