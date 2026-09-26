use std::io::Write as _;
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::Parser;
use relative_path::RelativePath;
use termcolor::{ColorChoice, StandardStream};

use crate::cli::WithRepos;
use crate::config::ConfigCommand;
use crate::ctxt::Ctxt;
use crate::model::Repo;

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    /// An extra command to run after the configured ones, can be used more
    /// than once.
    ///
    /// Each command is split on whitespace, and is added to whatever the
    /// `commands` option in the `[install]` section specifies.
    #[arg(long = "command", short = 'c', value_name = "COMMAND")]
    command: Vec<String>,
    /// Print the commands which would be run instead of running them.
    #[arg(long)]
    dry_run: bool,
}

pub(crate) fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    let mut o = StandardStream::stdout(ColorChoice::Auto);

    with_repos.run("install", format_args!("install: {opts:?}"), |cx, repo| {
        install(&mut o, cx, opts, repo)
    })?;

    Ok(())
}

#[tracing::instrument(skip_all)]
fn install(o: &mut StandardStream, cx: &Ctxt<'_>, opts: &Opts, repo: &Repo) -> Result<()> {
    let config = cx.config.install(repo);
    let root = cx.to_path(repo.path());

    let mut commands = config.commands;

    // NB: The default only stands in for missing configuration, so commands
    // passed through `--command` are still appended to it.
    if commands.is_empty()
        && let Some(command) = default_command(cx, repo)
    {
        commands.push(command);
    }

    commands.extend(opts.command.iter().filter_map(|c| ConfigCommand::split(c)));

    if commands.is_empty() {
        bail!(
            "No install commands configured, specify `commands` in the `[install]` section or pass `--command <command>`"
        );
    }

    for command in &commands {
        run(o, opts, command, &root)?;
    }

    Ok(())
}

/// The default install command, which is `cargo install --path <dir>` for a
/// Cargo project.
fn default_command(cx: &Ctxt<'_>, repo: &Repo) -> Option<ConfigCommand> {
    let manifest = cx
        .config
        .cargo_toml(repo)
        .unwrap_or(RelativePath::new("Cargo.toml"));

    if !cx.to_path(repo.path().join(manifest)).is_file() {
        return None;
    }

    let dir = match manifest.parent() {
        Some(dir) if !dir.as_str().is_empty() => dir.as_str(),
        _ => ".",
    };

    Some(ConfigCommand {
        command: "cargo".to_owned(),
        args: vec!["install".to_owned(), "--path".to_owned(), dir.to_owned()],
    })
}

/// Run a single install command in the given directory.
fn run(o: &mut StandardStream, opts: &Opts, command: &ConfigCommand, root: &Path) -> Result<()> {
    let mut command = command.to_command(root);
    let repr = command.display().to_string();

    if opts.dry_run {
        writeln!(o, "{repr}")?;
        return Ok(());
    }

    tracing::info!("{repr}");

    let status = command
        .status()
        .with_context(|| format!("Failed to run install command: {repr}"))?;

    if !status.success() {
        bail!("Install command failed with {status}: {repr}");
    }

    Ok(())
}
