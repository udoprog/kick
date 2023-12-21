use std::env;
use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;
use std::{ffi::OsString, fmt};

use anyhow::{bail, Context, Result};
use clap::{Parser, ValueEnum};

use crate::release::{ReleaseEnv, ReleaseOpts};

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

#[derive(Default, Debug, Clone, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    release: ReleaseOpts,
    /// A location to write the output to.
    #[arg(long, value_name = "path")]
    output: Option<PathBuf>,
    /// If specified, the output will be written to the path specified by the
    /// given environment variable.
    ///
    /// For example, an argument of `--output-from-env GITHUB_OUTPUT` would
    /// cause the values to be written to the path specified by the
    /// `GITHUB_OUTPUT` environment variable.
    #[arg(long, value_name = "env")]
    output_from_env: Option<OsString>,
    /// The format to write the output in.
    ///
    /// Available formats are: text, json.
    #[arg(long, value_name = "format", default_value_t = Format::Text)]
    format: Format,
    /// If specified, the value will be written to the specified name.
    ///
    /// For example, an argument of `--value-to channel` would cause `channel=<release>\n` to be written
    #[arg(long, value_name = "name")]
    version_to: Option<String>,
    /// If specified, the a `yes` or a `no` will be written to the specified
    /// variable depending on if it's a prerelease or not.
    ///
    /// Pre-releases are versions which are anything beyond strictly a semantic
    /// version or dated release.
    ///
    /// For example, an argument of `--is-pre-to prerelease` would cause `prerelease=yes\n` to be written
    #[arg(long, value_name = "name")]
    is_pre_to: Option<String>,
    /// Sets the following options:
    ///
    /// - `--version-to version`
    /// - `--is-pre-to pre`
    /// - `--output-from-env GITHUB_OUTPUT`
    ///
    /// Causing a file to be written to the path specified by GITHUB_OUTPUT,
    /// containing the `version` and `pre` definitions.
    #[arg(long, verbatim_doc_comment)]
    github_action: bool,
}

pub(crate) fn entry(opts: &Opts) -> Result<()> {
    let env = ReleaseEnv::new();
    let release = opts.release.make(&env)?;

    let mut copy;

    let opts = if opts.github_action {
        copy = opts.clone();
        copy.output_from_env = Some("GITHUB_OUTPUT".into());
        copy.version_to = Some("version".into());
        copy.is_pre_to = Some("pre".into());
        &copy
    } else {
        opts
    };

    let mut output;
    let mut stdout;
    let o: &mut dyn io::Write;

    match (opts.output.as_deref(), opts.output_from_env.as_deref()) {
        (Some(_), Some(_)) => {
            bail!("--output and --output-from-env cannot be used together")
        }
        (Some(path), None) => {
            output = OpenOptions::new()
                .append(true)
                .create(true)
                .open(path)
                .with_context(|| path.display().to_string())?;

            o = &mut output;
        }
        (None, Some(env)) => {
            let Some(path) = env::var_os(env).map(PathBuf::from) else {
                bail!(
                    "Environment variable `{}` is not set",
                    env.to_string_lossy()
                );
            };

            output = OpenOptions::new()
                .append(true)
                .create(true)
                .open(&path)
                .with_context(|| path.display().to_string())?;

            o = &mut output;
        }
        _ => {
            stdout = io::stdout();
            o = &mut stdout;
        }
    }

    match opts.format {
        Format::Text => {
            if let Some(key) = &opts.version_to {
                writeln!(o, "{key}={release}")?;
            }

            if let Some(key) = &opts.is_pre_to {
                let is_pre = release.is_pre();
                writeln!(o, "{key}={}", if is_pre { "yes" } else { "no" })?;
            }
        }
        Format::Json => {
            let mut payload = serde_json::Map::new();

            if let Some(key) = &opts.version_to {
                payload.insert(key.clone(), serde_json::to_value(&release)?);
            }

            if let Some(key) = &opts.is_pre_to {
                let is_pre = release.is_pre();
                payload.insert(key.clone(), serde_json::Value::Bool(is_pre));
            }

            serde_json::to_writer(&mut *o, &payload)?;
            writeln!(o)?;
        }
    };

    Ok(())
}
