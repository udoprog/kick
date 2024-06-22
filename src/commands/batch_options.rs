use clap::{Parser, ValueEnum};

use crate::config::Distribution;

use super::RunOn;

#[derive(Default, Debug, Parser)]
pub(crate) struct BatchOptions {
    /// Run the command using the specified execution methods.
    #[arg(long, value_name = "run-on")]
    pub(super) run_on: Vec<RunOnOption>,
    /// Environment variables to pass to the command to run. Only specifying
    /// `<key>` means that the specified environment variable should be passed
    /// through.
    ///
    /// For WSL, this constructs the WSLENV environment variable, which dictates
    /// what environments are passed in.
    #[arg(long, short = 'E', value_name = "key[=value]")]
    pub(super) env: Vec<String>,
    /// Print verbose information.
    ///
    /// One level `-V` prints the environment of the command invoked. Two levels
    /// `-VV` prints the full command as run from the host operating system.
    #[arg(long, short = 'V', action = clap::ArgAction::Count)]
    pub(super) verbose: u8,
    /// When printing diagnostics output, exposed secrets.
    ///
    /// If this is not specified, secrets will be printed as `***`.
    #[arg(long)]
    pub(super) exposed: bool,
    /// Don't actually run any commands, just print what would be done.
    #[arg(long)]
    pub(super) dry_run: bool,
    /// If there are any system remediations that have to be performed before
    /// running commands, apply them automatically.
    #[arg(long)]
    pub(crate) fix: bool,
}

/// A run on configuration.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub(super) enum RunOnOption {
    /// Run on the same system (default).
    Same,
    /// Run over WSL with the specified distribution.
    Wsl,
}

impl RunOnOption {
    /// Coerce into a [`RunOn`].
    pub(super) fn to_run_on(self) -> RunOn {
        match self {
            Self::Same => RunOn::Same,
            Self::Wsl => RunOn::Wsl(Distribution::Ubuntu),
        }
    }
}
