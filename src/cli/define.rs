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
    /// If the name is specified, the variable will be written in
    /// `<name>=<value>\n` format.
    #[arg(long, value_name = "name")]
    name: Option<String>,
    /// A location to write the output to.
    #[arg(long, value_name = "path")]
    output: Option<PathBuf>,
    /// The format to write the output in.
    ///
    /// Available formats are: text, json.
    #[arg(long, value_name = "format", default_value_t = Format::Text)]
    format: Format,
    #[clap(flatten)]
    release: ReleaseOpts,
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

    let value = match opts.format {
        Format::Text => release.to_string(),
        Format::Json => serde_json::to_string(&release)?,
    };

    if let Some(name) = &opts.name {
        writeln!(o, "{name}={value}").context("Could not write output")?;
    } else {
        writeln!(o, "{value}").context("Could not write output")?;
    }

    Ok(())
}
