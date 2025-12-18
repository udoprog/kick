use std::env;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::OpenOptions;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};
use clap::{Parser, ValueEnum};

use crate::Repo;
use crate::cli::WithRepos;
use crate::ctxt::Ctxt;
use crate::release::ReleaseOpts;

#[derive(Default, Debug, Clone, Copy, ValueEnum)]
enum Format {
    #[default]
    Text,
    Json,
}

impl fmt::Display for Format {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Format::Text => write!(f, "text"),
            Format::Json => write!(f, "json"),
        }
    }
}

enum Name<'a> {
    Path(&'a Path),
    Env(&'a OsStr),
}

impl fmt::Display for Name<'_> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Name::Path(path) => path.display().fmt(f),
            Name::Env(env) => env.to_string_lossy().fmt(f),
        }
    }
}

#[derive(Default, Debug, Clone, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    release: ReleaseOpts,
    /// A location to write the output to.
    #[arg(long)]
    output: Option<PathBuf>,
    /// If specified, the output will be written to the path specified by the
    /// given environment variable.
    ///
    /// For example, an argument of --output-from-env GITHUB_OUTPUT would cause
    /// the values to be written to the path specified by the GITHUB_OUTPUT
    /// environment variable.
    #[arg(long)]
    output_from_env: Option<OsString>,
    /// The format to write the output in.
    ///
    /// Available formats are: text, json.
    #[arg(long, default_value_t = Format::Text)]
    format: Format,
    /// If specified, the version will be written to the specified name.
    ///
    /// For example, an argument of --value-to version would cause
    /// version=<release>\n to be written.
    #[arg(long)]
    version_to: Option<String>,
    /// If specified, the version in MSI format will be written to the specified
    /// name.
    ///
    /// For example, an argument of --msi-version-to msi_version would cause
    /// msi_version=<value>\n to be written.
    ///
    /// Note that the MSI version follows the ProductVersion specification.
    ///
    /// See: https://learn.microsoft.com/en-us/windows/win32/msi/productversion
    #[arg(long)]
    msi_version_to: Option<String>,
    /// If specified, the a yes or a no will be written to the specified
    /// variable depending on if it's a prerelease or not.
    ///
    /// Pre-releases are versions which are anything beyond strictly a semantic
    /// version or dated release.
    ///
    /// For example, an argument of --is-pre-to prerelease would cause
    /// prerelease=yes\n to be written.
    #[arg(long)]
    is_pre_to: Option<String>,
    /// Set default settings for defining variables inside of a github release.
    ///
    /// Sets --version-to version, --is-pre-to pre, and --output-from-env
    /// GITHUB_OUTPUT.
    ///
    /// Causing a file to be written to the path specified by GITHUB_OUTPUT,
    /// containing the version and pre definitions.
    #[arg(long)]
    github_action: bool,
}

pub(crate) fn entry<'repo>(with_repos: &mut WithRepos<'repo>, opts: &Opts) -> Result<()> {
    with_repos.run(
        "publish github release",
        format_args!("github-release: {opts:?}"),
        |cx, repo| define(cx, repo, opts),
    )?;

    Ok(())
}

fn define(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let version = opts.release.version(cx, repo)?;

    let output_from_env = 'out: {
        if let Some(key) = opts.output_from_env.as_deref() {
            break 'out Some(key);
        }

        if opts.github_action {
            break 'out Some(OsStr::new("GITHUB_OUTPUT"));
        }

        None
    };

    let env_path;

    let output = 'out: {
        if let Some(env) = output_from_env
            && let Some(path) = env::var_os(env).map(PathBuf::from)
        {
            ensure!(
                opts.output.is_none(),
                "Cannot use --output and --output-from-env together"
            );

            env_path = path;
            break 'out Some((Name::Env(env), env_path.as_path()));
        }

        opts.output.as_deref().map(|path| (Name::Path(path), path))
    };

    let mut output_file;
    let mut stdout;
    let o: &mut dyn io::Write;

    match output {
        Some((ref name, path)) => {
            output_file = OpenOptions::new()
                .append(true)
                .create(true)
                .open(path)
                .with_context(|| path.display().to_string())?;

            tracing::info!("Writing information on version `{version}` to {name}",);
            o = &mut output_file;
        }
        _ => {
            stdout = io::stdout();
            o = &mut stdout;
        }
    }

    tracing::trace! {
        output = output.as_ref().map(|(name, _)| name.to_string()),
        output_from_env = output_from_env.map(|s| s.to_string_lossy().into_owned()),
        format = opts.format.to_string(),
        version = opts.version_to.as_deref(),
        msi_version = opts.msi_version_to.as_deref(),
        pre = opts.is_pre_to.as_deref().map(|key| format!("{key}={}", if version.is_pre() { "yes" } else { "no" })),
        "Defining",
    };

    let version_key = opts.version_to.as_deref().unwrap_or("version");
    let is_pre_key = opts.is_pre_to.as_deref().unwrap_or("pre");
    let is_pre = version.is_pre();

    match opts.format {
        Format::Text => {
            writeln!(o, "{version_key}={version}")?;
            writeln!(o, "{is_pre_key}={}", if is_pre { "yes" } else { "no" })?;

            if let Some(key) = &opts.msi_version_to {
                let msi_version = version.msi_version().context("Calculating MSI version")?;
                writeln!(o, "{key}={msi_version}")?;
            }
        }
        Format::Json => {
            let mut payload = serde_json::Map::new();
            payload.insert(version_key.to_owned(), serde_json::to_value(&version)?);
            payload.insert(is_pre_key.to_owned(), serde_json::Value::Bool(is_pre));

            if let Some(key) = &opts.msi_version_to {
                let msi_version = version.msi_version().context("Calculating MSI version")?;
                payload.insert(key.clone(), serde_json::Value::String(msi_version));
            }

            serde_json::to_writer(&mut *o, &payload)?;
            writeln!(o)?;
        }
    };

    Ok(())
}
