use std::collections::HashMap;
use std::env::consts::{self, EXE_EXTENSION};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Cursor, Write};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use time::OffsetDateTime;

use crate::cli::WithRepos;
use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::model::Repo;
use crate::release::{ReleaseOpts, Version};

use super::output::OutputOpts;

#[derive(Debug, Clone, Copy)]
pub(crate) enum Kind {
    /// Construct a .tar.gz file.
    Gzip,
    /// Construct a .zip file.
    Zip,
}

impl Kind {
    fn extension(&self) -> &'static str {
        match self {
            Kind::Gzip => "tar.gz",
            Kind::Zip => "zip",
        }
    }
}

#[derive(Debug, Parser)]
pub(crate) struct Opts {
    #[clap(flatten)]
    release: ReleaseOpts,
    /// The architecture to append to the archive.
    ///
    /// If not specified, defaults to `std::env::consts::ARCH`,
    #[arg(long, value_name = "os")]
    arch: Option<String>,
    /// The operating system to append to the archive.
    ///
    /// If not specified, defaults to `std::env::consts::OS`,
    #[arg(long, value_name = "os")]
    os: Option<String>,
    /// The name format to use for the archive
    ///
    /// If unspecified, the name will be `{project}-{release}-{arch}-{os}`.
    #[arg(long, value_name = "name")]
    name: Option<String>,
    /// Exclude the default bianries from the archive.
    #[arg(long)]
    no_bin: bool,
    /// Binaries to append to the archive as they are named in the workspace.
    ///
    /// By default, all binaries from the primary package will be included.
    #[arg(long, value_name = "bin")]
    bin: Vec<String>,
    #[clap(flatten)]
    output: OutputOpts,
    /// Append the given extra files to the archive.
    #[arg(value_name = "path")]
    path: Vec<String>,
}

pub(crate) fn entry<'repo>(with_repos: impl WithRepos<'repo>, ty: Kind, opts: &Opts) -> Result<()> {
    with_repos.run(
        format!("compress {}", ty.extension()),
        format_args!("compress: {opts:?}"),
        |cx, repo| compress(cx, ty, opts, repo),
    )?;

    Ok(())
}

#[tracing::instrument(skip_all)]
fn compress(cx: &Ctxt<'_>, ty: Kind, opts: &Opts, repo: &Repo) -> Result<()> {
    let workspace = repo.workspace(cx)?;

    let release = opts.release.version(cx, repo)?;
    let package = workspace.primary_package()?;
    let name = package.name()?;

    let os = &opts.os.as_deref().unwrap_or(consts::OS);
    let arch = opts.arch.as_deref().unwrap_or(consts::ARCH);

    let name_template = Template::parse(
        opts.name
            .as_deref()
            .unwrap_or("{project}-{release}-{arch}-{os}"),
    )?;
    let variables = variables(name, release, os, arch);
    let archive_name = name_template.render(&variables)?;

    let root = cx.to_path(repo.path());

    let mut zip_archive;
    let mut gzip_archive;

    let archive: &mut dyn Archive = match ty {
        Kind::Gzip => {
            gzip_archive = GzipArchive::create();
            &mut gzip_archive
        }
        Kind::Zip => {
            zip_archive = ZipArchive::create();
            &mut zip_archive
        }
    };

    let mut out = Vec::new();

    let release_dir = root.join("target").join("release");

    if opts.bin.is_empty() && !opts.no_bin {
        let binary_path = release_dir.join(name).with_extension(EXE_EXTENSION);
        out.push(binary_path);
    } else {
        for name in &opts.bin {
            let binary_path = release_dir.join(name).with_extension(EXE_EXTENSION);
            out.push(binary_path);
        }
    }

    for pattern in &opts.path {
        let glob = Glob::new(&root, &pattern);

        for path in glob.matcher() {
            let path = path.with_context(|| anyhow!("Glob failed: {}", pattern))?;
            out.push(path.to_path(&root));
        }
    }

    for path in out {
        tracing::info!("Appending: {}", path.display());
        append(archive, &path).with_context(|| anyhow!("Appending {}", path.display()))?;
    }

    let contents = archive.finish()?;
    let output = opts.output.make_directory(cx, repo, ty.extension());

    let mut f = output.create_file(format!("{archive_name}.{}", ty.extension()))?;

    f.write_all(&contents)
        .with_context(|| anyhow!("Writing contents to {}", f.path().display()))?;
    Ok(())
}

fn append(archive: &mut dyn Archive, path: &Path) -> Result<()> {
    let metadata = fs::metadata(path)?;

    if metadata.is_file() {
        let input = File::open(path)?;

        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("Missing file name")?;

        archive
            .append_file(&metadata, input, file_name)
            .with_context(|| anyhow!("Append file {}", path.display()))?;
    } else {
        tracing::warn!("Ignoring non-file: {}", path.display());
    }

    Ok(())
}

trait Archive {
    fn append_file(&mut self, metadata: &fs::Metadata, input: File, file_name: &str) -> Result<()>;

    fn finish(&mut self) -> Result<Vec<u8>>;
}

struct GzipArchive {
    builder: Option<tar::Builder<flate2::write::GzEncoder<Vec<u8>>>>,
}

impl GzipArchive {
    fn create() -> Self {
        let encoder = flate2::GzBuilder::new().write(Vec::new(), flate2::Compression::default());
        let builder = tar::Builder::new(encoder);
        Self {
            builder: Some(builder),
        }
    }
}

impl Archive for GzipArchive {
    fn append_file(
        &mut self,
        metadata: &fs::Metadata,
        mut input: File,
        file_name: &str,
    ) -> Result<()> {
        let Some(builder) = &mut self.builder else {
            bail!("Archive already finished");
        };

        let mut header = tar::Header::new_gnu();
        header.set_size(metadata.len());

        #[cfg(unix)]
        {
            header.set_mode(metadata.mode());
            header.set_mtime(metadata.mtime() as u64);
        }

        builder
            .append_data(&mut header, file_name, &mut input)
            .context("Appending to archive")?;

        Ok(())
    }

    fn finish(&mut self) -> Result<Vec<u8>> {
        let Some(builder) = self.builder.take() else {
            bail!("Archive already finished");
        };

        let encoder = builder.into_inner().context("Finishing archive")?;
        let inner = encoder.finish().context("Finishing archive")?;
        Ok(inner)
    }
}

struct ZipArchive {
    zip: Option<zip::ZipWriter<Cursor<Vec<u8>>>>,
}

impl ZipArchive {
    fn create() -> Self {
        Self {
            zip: Some(zip::ZipWriter::new(Cursor::new(Vec::new()))),
        }
    }
}

impl Archive for ZipArchive {
    fn append_file(
        &mut self,
        metadata: &fs::Metadata,
        mut input: File,
        file_name: &str,
    ) -> Result<()> {
        use zip::write::FileOptions;
        use zip::{CompressionMethod, DateTime};

        let Some(zip) = &mut self.zip else {
            bail!("Archive already finished");
        };

        let mut options =
            FileOptions::<()>::default().compression_method(CompressionMethod::Deflated);

        #[cfg(unix)]
        {
            options = options.unix_permissions(metadata.mode());
        }

        let from = OffsetDateTime::from(metadata.modified()?);
        options = options.last_modified_time(DateTime::try_from(from)?);
        zip.start_file(file_name, options)?;
        io::copy(&mut input, zip).context("Copying file")?;
        Ok(())
    }

    fn finish(&mut self) -> Result<Vec<u8>> {
        let Some(zip) = self.zip.take() else {
            bail!("Archive already finished");
        };

        Ok(zip.finish()?.into_inner())
    }
}

enum Part<'a> {
    Literal(&'a str),
    Variable(&'a str),
}

enum Variable<'a> {
    Str(&'a str),
    Version(Version<'a>),
}

impl fmt::Display for Variable<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Variable::Str(s) => f.write_str(s),
            Variable::Version(v) => write!(f, "{v}"),
        }
    }
}

fn variables<'a>(
    project: &'a str,
    release: Version<'a>,
    os: &'a str,
    arch: &'a str,
) -> HashMap<&'a str, Variable<'a>> {
    let mut vars = HashMap::new();
    vars.insert("project", Variable::Str(project));
    vars.insert("release", Variable::Version(release));
    vars.insert("os", Variable::Str(os));
    vars.insert("arch", Variable::Str(arch));
    vars
}

struct Template<'a> {
    parts: Vec<Part<'a>>,
}

impl<'a> Template<'a> {
    /// Parse a template of `{part}` separated by literal components.
    fn parse(input: &'a str) -> Result<Self> {
        let mut parts = Vec::new();
        let mut remaining = input;

        while let Some(open) = remaining.find('{') {
            // Add literal part before the '{'
            if open > 0 {
                parts.push(Part::Literal(
                    remaining.get(..open).context("Invalid input")?,
                ));
            }

            // Advance past the '{'
            remaining = remaining.get(open + 1..).context("Invalid input")?;

            // Find closing brace
            let Some(close) = remaining.find('}') else {
                bail!(
                    "Unclosed variable at position {}",
                    input.len() - remaining.len()
                );
            };

            // Extract variable name
            let name = remaining.get(..close).context("Invalid input")?;

            if name.is_empty() {
                bail!(
                    "Empty variable name at position {}",
                    input.len() - remaining.len() - 1
                );
            }

            parts.push(Part::Variable(name));

            // Advance past the closing brace
            remaining = &remaining[close + 1..];
        }

        // Add remaining literal part if any
        if !remaining.is_empty() {
            parts.push(Part::Literal(remaining));
        }

        Ok(Self { parts })
    }

    fn render(&self, variables: &HashMap<&str, Variable<'_>>) -> Result<String> {
        use std::fmt::Write;

        let mut s = String::new();

        for part in &self.parts {
            match part {
                Part::Literal(value) => {
                    s.push_str(value);
                }
                Part::Variable(var) => {
                    let Some(value) = variables.get(var) else {
                        bail!("Missing variable: {var}");
                    };

                    write!(s, "{value}").context("Rendering template")?;
                }
            }
        }

        Ok(s)
    }
}
