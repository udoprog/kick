use std::process::Command;

use anyhow::{anyhow, Context, Error, Result};
use clap::Parser;

use crate::ctxt::Ctxt;
use crate::model::Module;
use crate::utils::CommandRepr;

#[derive(Default, Parser)]
pub(crate) struct Opts {
    /// Only run over dirty modules with changes that have not been staged in
    /// cache.
    #[arg(long)]
    dirty: bool,
    /// Only run over modules that have changes staged in cache.
    #[arg(long)]
    cached: bool,
    /// Only run over modules that only have changes staged in cached and
    /// nothing dirty.
    #[arg(long)]
    cached_only: bool,
    /// Only go over repos with unreleased changes, or ones which are on a
    /// commit that doesn't have a tag as determined by `git describe --tags`.
    #[arg(long)]
    unreleased: bool,
    /// Command to run.
    command: Vec<String>,
}

impl Opts {
    fn needs_git(&self) -> bool {
        self.dirty || self.cached || self.cached_only || self.unreleased
    }
}

pub(crate) fn entry(cx: &Ctxt<'_>, opts: &Opts) -> Result<()> {
    let Some((command, args)) = opts.command.split_first() else {
        return Err(anyhow!("missing command"));
    };

    for module in cx.modules() {
        foreach(cx, opts, module, command, args).with_context(|| module.path.clone())?;
    }

    Ok(())
}

#[tracing::instrument(skip(cx, opts, module, command, args), fields(path = module.path.as_str()))]
fn foreach(
    cx: &Ctxt<'_>,
    opts: &Opts,
    module: &Module,
    command: &str,
    args: &[String],
) -> Result<()> {
    let current_dir = module.path.to_path(cx.root);

    if opts.needs_git() {
        let git = cx.require_git()?;

        let cached = git.is_cached(&current_dir)?;
        let dirty = git.is_dirty(&current_dir)?;

        let span = tracing::trace_span!("git", ?cached, ?dirty);
        let _enter = span.enter();

        if opts.dirty && !dirty {
            tracing::trace!("directory is not dirty");
            return Ok(());
        }

        if opts.cached && !cached {
            tracing::trace!("directory has no cached changes");
            return Ok(());
        }

        if opts.cached_only && (!cached || dirty) {
            tracing::trace!("directory has no cached changes");
            return Ok(());
        }

        if opts.unreleased {
            let Some((tag, offset)) = git.describe_tags(&current_dir)? else {
                tracing::trace!("no tags to describe");
                return Ok(());
            };

            if offset.is_none() {
                tracing::trace!("no offset detected (tag: {tag})");
                return Ok(());
            }
        }
    }

    tracing::info!(
        path = module.path.as_str(),
        "{}",
        CommandRepr::new(&opts.command)
    );

    let status = Command::new(command)
        .args(args)
        .current_dir(&current_dir)
        .status()
        .with_context(|| Error::msg(CommandRepr::new(&opts.command).to_string()))?;

    tracing::trace!(?status);
    Ok(())
}
