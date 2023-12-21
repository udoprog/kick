use std::fmt;
use std::fs::OpenOptions;
use std::io;
use std::path::PathBuf;

use anyhow::{Context, Result};
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

#[derive(Default, Debug, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    release: ReleaseOpts,
    /// A location to write the output to.
    #[arg(long, value_name = "path")]
    output: Option<PathBuf>,
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
}

pub(crate) fn entry(opts: &Opts) -> Result<()> {
    let env = ReleaseEnv::new();
    let release = opts.release.make(&env)?;

    let mut output;
    let mut stdout;
    let o: &mut dyn io::Write;

    if let Some(path) = &opts.output {
        output = OpenOptions::new()
            .append(true)
            .create(true)
            .open(path)
            .with_context(|| path.display().to_string())?;

        o = &mut output;
    } else {
        stdout = io::stdout();

        o = &mut stdout;
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
