use std::env::consts::{self, EXE_EXTENSION};
use std::fs::{self, File};
use std::io::{self, Cursor};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Parser, ValueEnum};
use semver::Version;
use time::OffsetDateTime;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::workspace;

#[derive(Debug, Clone, ValueEnum)]
enum Kind {
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
    /// The type of archive to build.
    #[arg(name = "type", value_name = "type")]
    ty: Kind,
    /// The operating system to append to the archive.
    ///
    /// If not specified, defaults to `std::env::consts::OS`,
    #[arg(long, value_name = "os")]
    os: Option<String>,
    /// Append the given extra files to the archive.
    #[arg(value_name = "append")]
    append: Vec<PathBuf>,
}

pub(crate) fn entry(cx: &mut Ctxt<'_>, opts: &Opts) -> Result<()> {
    for repo in cx.repos() {
        compress(cx, repo, opts).with_context(cx.context(repo))?;
    }

    Ok(())
}

fn compress(cx: &Ctxt<'_>, repo: &Repo, opts: &Opts) -> Result<()> {
    let Some(workspace) = workspace::open(cx, repo)? else {
        bail!("Not a workspace");
    };

    let package = workspace.primary_package()?;
    let name = package.name()?;

    let Some(version) = package.version() else {
        bail!("No version in primary package");
    };

    let version = Version::parse(version)?;
    let arch = consts::ARCH;

    let os = match &opts.os {
        Some(os) => os,
        None => consts::OS,
    };

    let root = cx.to_path(repo.path());

    let mut zip_archive;
    let mut gzip_archive;

    let archive: &mut dyn Archive = match &opts.ty {
        Kind::Gzip => {
            gzip_archive = GzipArchive::create();
            &mut gzip_archive
        }
        Kind::Zip => {
            zip_archive = ZipArchive::create();
            &mut zip_archive
        }
    };

    let output_path = root.join(format!(
        "{name}-{version}-{arch}-{os}.{}",
        opts.ty.extension()
    ));

    tracing::info!("Writing {}", output_path.display());

    let exe_path = root
        .join("target")
        .join("release")
        .join(name)
        .with_extension(EXE_EXTENSION);

    for path in [&exe_path].into_iter().chain(&opts.append) {
        tracing::info!("Appending: {}", path.display());

        archive
            .append(path)
            .with_context(|| anyhow!("Appending {}", path.display()))?;
    }

    let contents = archive.finish()?;

    fs::write(&output_path, contents).with_context(|| output_path.display().to_string())?;
    Ok(())
}

trait Archive {
    fn append(&mut self, path: &Path) -> Result<()> {
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .context("Missing file name")?;

        let metadata = fs::metadata(path)?;
        let input = File::open(path)?;

        if metadata.is_file() {
            self.append_file(&metadata, input, file_name)
                .with_context(|| anyhow!("Append file {}", path.display()))?;
        } else {
            bail!("Not supported for archive");
        }

        Ok(())
    }

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
    zip: zip::ZipWriter<Cursor<Vec<u8>>>,
}

impl ZipArchive {
    fn create() -> Self {
        Self {
            zip: zip::ZipWriter::new(Cursor::new(Vec::new())),
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

        let mut options = FileOptions::default().compression_method(CompressionMethod::Bzip2);

        #[cfg(unix)]
        {
            options = options.unix_permissions(metadata.mode());
        }

        let from = OffsetDateTime::from(metadata.modified()?);
        options = options.last_modified_time(DateTime::try_from(from)?);
        self.zip.start_file(file_name, options)?;
        io::copy(&mut input, &mut self.zip).context("Copying file")?;
        Ok(())
    }

    fn finish(&mut self) -> Result<Vec<u8>> {
        Ok(self.zip.finish()?.into_inner())
    }
}