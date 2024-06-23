use anyhow::Result;
use clap::{Parser, ValueEnum};

use crate::config::{Distribution, Os};
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::shell::Shell;

use super::{BatchConfig, RunOn};

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
    pub(super) fix: bool,
    /// Keep work files around and do not remove them at the end of the run.
    #[arg(long)]
    pub(super) keep: bool,
    /// The default shell to use when printing command invocations.
    ///
    /// By default this is `bash` for unix-like environments and `powershell`
    /// for windows.
    #[arg(long, value_name = "shell")]
    pub(super) shell: Option<Shell>,
    /// Matrix values to ignore when running a Github workflows job.
    #[arg(long, value_name = "value")]
    pub(super) matrix_ignore: Vec<String>,
    /// Filter matrix values when running a Github workflows job, only allowing
    /// the values which matches one of the conditions specified.
    #[arg(long, value_name = "key=value")]
    pub(super) matrix_filter: Vec<String>,
}

impl BatchOptions {
    /// Build a batch configuration from a set of commandline options.
    pub(crate) fn build<'a, 'cx>(
        &self,
        cx: &'a Ctxt<'cx>,
        repo: &'a Repo,
    ) -> Result<BatchConfig<'a, 'cx>> {
        let repo_path = cx.to_path(repo.path());

        let shell = self.shell.unwrap_or_else(|| cx.current_os.shell());

        let mut c = BatchConfig::new(cx, repo_path, shell);

        for &run_on in &self.run_on {
            c.add_run_on(run_on.to_run_on(), run_on.to_os(&cx.current_os))?;
        }

        if self.exposed {
            c.exposed = true;
        }

        c.verbose = self.verbose;

        if self.dry_run {
            c.dry_run = true;
        }

        for env in &self.env {
            c.parse_env(env)?;
        }

        for variable in &self.matrix_ignore {
            c.matrix_ignore.insert(variable.clone());
        }

        for filter in &self.matrix_filter {
            if let Some((key, value)) = filter.split_once('=') {
                c.matrix_filter.push((key.to_owned(), value.to_owned()));
            }
        }

        if self.fix {
            c.fix = true;
        }

        if self.keep {
            c.keep = true;
        }

        Ok(c)
    }
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

    /// Coerce into a [`Os`].
    pub(super) fn to_os(self, current_os: &Os) -> Os {
        match self {
            Self::Same => current_os.clone(),
            Self::Wsl => Os::Linux,
        }
    }
}
