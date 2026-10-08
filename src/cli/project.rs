//! The `kick project` command, which manages the projects registered in the
//! global configuration.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

use crate::config;
use crate::global::{self, Global, Projects};
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
    /// Directories inside of the home directory are stored with a `~/`
    /// prefix, and other directories by their absolute path. A relative key
    /// written by hand is resolved against the directory holding the global
    /// `Kick.toml`.
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
    /// Edit the settings of a project in the global configuration.
    ///
    /// The `[repo."<key>"]` table of the project is written to a temporary
    /// file and opened in `$VISUAL`, `$EDITOR`, or a default editor. Once the
    /// editor exits, the edited settings are validated and replace the
    /// table, leaving the rest of the configuration untouched. If they are
    /// invalid nothing is changed and the temporary file is kept.
    Edit {
        /// The directory of the project, or the key it is registered under.
        /// Defaults to the git checkout containing the current directory.
        path: Option<String>,
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
        Action::Edit { path } => {
            let (raw, dir) = match path {
                Some(path) => (Some(path.as_str()), resolve_arg(cx, path)),
                None => (None, checkout_root(cx.current_dir)),
            };

            edit(cx, raw, &dir)?;
        }
    }

    Ok(())
}

/// Edit the settings of the project registered under the key `raw` or at
/// `dir` in an editor.
fn edit(cx: &Cx<'_>, raw: Option<&str>, dir: &Path) -> Result<()> {
    let mut projects = Projects::open(cx.config, cx.home)?;
    let project = projects.get(raw, dir)?;
    let original = projects.settings(&project.key)?;

    let file = tempfile::Builder::new()
        .prefix("kick-project-")
        .suffix(".toml")
        .tempfile()
        .context("Creating a temporary file")?
        .into_temp_path();

    fs::write(&file, &original).with_context(|| format!("Writing {}", file.display()))?;

    run_editor(&file)?;

    let edited =
        fs::read_to_string(&file).with_context(|| format!("Reading {}", file.display()))?;

    if edited == original {
        println!("No changes made to {}", project.key);
        return Ok(());
    }

    let global = Global {
        path: cx.config.to_owned(),
        home: cx.home.to_owned(),
        provides_repos: false,
    };

    if let Err(error) = apply(&mut projects, &global, &project.key, &edited) {
        let kept = file
            .keep()
            .context("Keeping the temporary file with the edits")?;

        return Err(error.context(format!(
            "Edits to {} not saved, they are kept in {}",
            project.key,
            kept.display()
        )));
    }

    projects.save()?;

    println!(
        "Updated {} in {}",
        project.key,
        global::display(cx.home, cx.config)
    );

    if !projects
        .list()
        .iter()
        .any(|p| p.key == project.key && p.url.is_some())
    {
        println!(
            "Note: {} has no url, kick ignores projects without a url",
            project.key
        );
    }

    Ok(())
}

/// Validate the edited settings of the project registered under `key` and
/// replace its settings with them.
fn apply(projects: &mut Projects, global: &Global, key: &str, edited: &str) -> Result<()> {
    let table = toml::from_str::<toml::Table>(edited).context("Invalid TOML")?;
    let errors = config::validate_global_repo(global, table)?;

    if !errors.is_empty() {
        let mut message = String::from("Invalid settings:");

        for error in errors {
            message.push_str("\n  ");
            message.push_str(&error);
        }

        bail!(message);
    }

    projects.set_settings(key, edited).context("Invalid TOML")?;
    Ok(())
}

/// Open `path` in the editor of the user, which is taken from `$VISUAL` or
/// `$EDITOR` and may include arguments, such as `code --wait`.
fn run_editor(path: &Path) -> Result<()> {
    let editor = ["VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(std::env::var_os)
        .find(|value| !value.is_empty());

    let (mut command, display) = match editor {
        Some(editor) => (
            editor_command(&editor),
            editor.to_string_lossy().into_owned(),
        ),
        None => {
            let editor = if cfg!(windows) { "notepad" } else { "vi" };
            (Command::new(editor), editor.to_owned())
        }
    };

    let status = command
        .arg(path)
        .status()
        .with_context(|| format!("Running editor `{display}`"))?;

    if !status.success() {
        bail!("Editor `{display}` exited with {status}, no changes made");
    }

    Ok(())
}

/// Build the command for an editor specified by the user, which may include
/// arguments.
///
/// Like git, the editor is run through the shell so that it may quote its
/// arguments, with the file to edit passed as an argument to it.
#[cfg(not(windows))]
fn editor_command(editor: &OsString) -> Command {
    let mut script = editor.clone();
    script.push(" \"$@\"");

    let mut command = Command::new("sh");
    command.arg("-c").arg(script).arg(editor);
    command
}

/// Build the command for an editor specified by the user, which may include
/// arguments separated by whitespace.
#[cfg(windows)]
fn editor_command(editor: &OsString) -> Command {
    let editor = editor.to_string_lossy();
    let mut parts = editor.split_whitespace();
    let mut command = Command::new(parts.next().unwrap_or("notepad"));
    command.args(parts);
    command
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
