//! The `kick project` command, which manages the projects registered in the
//! global configuration.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use crate::global::{self, Projects};
use crate::system::Git;

#[derive(Debug, Parser)]
pub(crate) struct Opts {
    #[command(subcommand)]
    action: Action,
}

#[derive(Debug, Subcommand)]
enum Action {
    /// List the projects registered in the global configuration.
    List,
    /// Register a project directory in the global configuration.
    ///
    /// Directories inside of the home directory are stored relative to it
    /// with a `~/` prefix.
    Add {
        /// The directory to register. Defaults to the git checkout containing
        /// the current directory, or the current directory itself.
        path: Option<PathBuf>,
        /// The url of the project. Defaults to the url of the `origin` remote
        /// of the git checkout.
        #[arg(long)]
        url: Option<String>,
    },
    /// Remove a project from the global configuration.
    Remove {
        /// The directory of the project, or the key it is registered under.
        path: String,
    },
}

/// What the `project` command needs to run.
pub(crate) struct Cx<'a> {
    /// The path of the global configuration.
    pub(crate) config: &'a Path,
    /// The home directory.
    pub(crate) home: &'a Path,
    /// The current directory, which relative paths are resolved against.
    pub(crate) current_dir: &'a Path,
    /// Git, used to detect the url of a project.
    pub(crate) git: Option<&'a Git>,
}

pub(crate) fn entry(cx: &Cx<'_>, opts: &Opts) -> Result<()> {
    match &opts.action {
        Action::List => {
            let projects = Projects::open(cx.config, cx.home)?;
            let list = projects.list();

            if list.is_empty() {
                println!("No projects registered in {}", cx.config.display());
                println!("Register one with `kick project add <path>`");
                return Ok(());
            }

            for project in list {
                match &project.path {
                    Ok(path) => {
                        let exists = if path.is_dir() { "" } else { " (missing)" };

                        println!("{}{exists}", project.key);

                        if project.key != global::display(cx.home, path) {
                            println!("  path: {}", path.display());
                        }
                    }
                    Err(error) => {
                        println!("{} (error)", project.key);
                        println!("  error: {error}");
                    }
                }

                match &project.url {
                    Some(url) => println!("  url: {url}"),
                    None => println!("  url: none, kick ignores projects without a url"),
                }
            }
        }
        Action::Add { path, url } => {
            let dir = match path {
                Some(path) => resolve_arg(cx, &path.to_string_lossy()),
                None => checkout_root(cx.current_dir),
            };

            let url = match url {
                Some(url) => url.clone(),
                None => detect_url(cx.git, &dir)?,
            };

            let mut projects = Projects::open(cx.config, cx.home)?;
            let key = projects.add(&dir, &url)?;
            projects.save()?;
            println!(
                "Registered {key} ({url}) in {}",
                global::display(cx.home, cx.config)
            );
        }
        Action::Remove { path } => {
            let dir = resolve_arg(cx, path);
            let mut projects = Projects::open(cx.config, cx.home)?;
            let removed = projects.remove(path, &dir)?;
            projects.save()?;
            println!(
                "Removed {} from {}",
                removed.key,
                global::display(cx.home, cx.config)
            );
        }
    }

    Ok(())
}

/// Resolve a path given on the command line, which is relative to the
/// current directory unless it starts with `~/` or is absolute.
fn resolve_arg(cx: &Cx<'_>, value: &str) -> PathBuf {
    global::resolve(cx.home, cx.current_dir, value)
}

/// Find the git checkout containing `dir`, or `dir` itself if there is none.
fn checkout_root(dir: &Path) -> PathBuf {
    let dir = global::normalize(dir);

    dir.ancestors()
        .find(|p| p.join(".git").exists())
        .map(Path::to_path_buf)
        .unwrap_or(dir)
}

fn detect_url(git: Option<&Git>, dir: &Path) -> Result<String> {
    let git = git.context("No url specified with --url, and git is not available to detect one")?;

    let url = git.get_url(dir, "origin").with_context(|| {
        format!(
            "Detecting the url of the `origin` remote in {}, specify one with --url",
            dir.display()
        )
    })?;

    Ok(url.to_string())
}
