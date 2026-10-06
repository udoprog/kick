use std::env::consts::{self, EXE_SUFFIX};
use std::fmt;
use std::fs::File;
use std::io::{self, Cursor, Write};
use std::path::Path;
use std::time::SystemTime;

use anyhow::{Context, Result, anyhow, bail};
use clap::Parser;
use relative_path::RelativePath;
use time::OffsetDateTime;

use crate::cli::WithRepos;
use crate::config::PackageFile;
use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::packaging::{self, Mode, Packager, infer};
use crate::release::ReleaseOpts;

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
    /// If not specified, defaults to the rust ARCH,
    #[arg(long)]
    arch: Option<String>,
    /// The operating system to append to the archive.
    ///
    /// If not specified, defaults to the rust OS when compiling kick,
    #[arg(long)]
    os: Option<String>,
    /// The name format to use for the archive
    ///
    /// If unspecified, the name will be {project}-{release}-{arch}-{os}. This is
    /// a jinja template whose variables use single braces, see the templates
    /// section of the configuration documentation.
    #[arg(long)]
    name: Option<String>,
    #[clap(flatten)]
    output: OutputOpts,
}

pub(crate) fn entry<'repo>(with_repos: &mut WithRepos<'repo>, ty: Kind, opts: &Opts) -> Result<()> {
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
    let package = workspace.primary_package()?.ensure_package()?;
    let name = package.name()?;

    let os = &opts.os.as_deref().unwrap_or(consts::OS);
    let arch = opts.arch.as_deref().unwrap_or(consts::ARCH);

    let name_template = opts.name.as_deref().unwrap_or(DEFAULT_NAME);
    let archive_name = render_name(name_template, name, &release, os, arch)?;

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

    let mut packager = CompressPackager { archive };
    let n = packaging::install_files(&mut packager, cx, repo)?;

    if n > 0 {
        bail!("Stopping due to {n} error(s)");
    };

    let contents = archive.finish()?;
    let output = opts.output.make_directory(cx, repo, ty.extension());

    let mut f = output.create_file(format!("{archive_name}.{}", ty.extension()))?;

    f.write_all(&contents)
        .with_context(|| anyhow!("Writing contents to {}", f.path().display()))?;
    Ok(())
}

struct CompressPackager<'a> {
    archive: &'a mut dyn Archive,
}

impl Packager for CompressPackager<'_> {
    fn add_binary(&mut self, name: &str, path: &Path) -> Result<()> {
        let infer = infer(path)?;
        let input = File::open(path)?;
        let name = format!("{name}{EXE_SUFFIX}");
        self.archive
            .append_file(input, &name, Mode::EXECUTABLE, infer.size, infer.mtime)?;
        Ok(())
    }

    fn add_file(&mut self, file: &PackageFile, path: &Path, dest: &RelativePath) -> Result<()> {
        let infer = infer(path)?;
        let mode = file.mode.unwrap_or(infer.mode);
        let input = File::open(path)?;
        self.archive
            .append_file(input, dest.as_str(), mode, infer.size, infer.mtime)?;
        Ok(())
    }
}

trait Archive {
    /// Append a file to the archive.
    fn append_file(
        &mut self,
        input: File,
        name: &str,
        mode: Mode,
        size: u64,
        mtime: Option<SystemTime>,
    ) -> Result<()>;

    /// Finish the archive and return the contents as a vector of bytes.
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
        mut input: File,
        name: &str,
        mode: Mode,
        size: u64,
        mtime: Option<SystemTime>,
    ) -> Result<()> {
        let builder = self.builder.as_mut().context("Archive already finished")?;

        let mut header = tar::Header::new_gnu();
        header.set_size(size);
        header.set_mode(mode.permissions());

        if let Some(m) = mtime
            && let Ok(d) = m.duration_since(SystemTime::UNIX_EPOCH)
        {
            header.set_mtime(d.as_secs());
        }

        builder
            .append_data(&mut header, name, &mut input)
            .context("Appending to archive")?;

        Ok(())
    }

    fn finish(&mut self) -> Result<Vec<u8>> {
        let builder = self.builder.take().context("Archive already finished")?;
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
        mut input: File,
        name: &str,
        mode: Mode,
        _: u64,
        mtime: Option<SystemTime>,
    ) -> Result<()> {
        use zip::write::FileOptions;
        use zip::{CompressionMethod, DateTime};

        let zip = self.zip.as_mut().context("Archive already finished")?;

        let mut options = FileOptions::<()>::default();

        options = options.compression_method(CompressionMethod::Deflated);
        options = options.unix_permissions(mode.permissions());

        if let Some(mtime) = mtime {
            let from = OffsetDateTime::from(mtime);
            options = options.last_modified_time(DateTime::try_from(from)?);
        }

        zip.start_file(name, options)?;
        io::copy(&mut input, zip).context("Copying file")?;
        Ok(())
    }

    fn finish(&mut self) -> Result<Vec<u8>> {
        let zip = self.zip.take().context("Archive already finished")?;
        Ok(zip.finish()?.into_inner())
    }
}

/// The default archive name template.
const DEFAULT_NAME: &str = "{project}-{release}-{arch}-{os}";

/// Render the archive name template given to `--name`.
///
/// This is a [minijinja] template whose variable delimiters are single braces,
/// so `{project}` expands a variable and expressions such as `{os | upper}`
/// work. Blocks and comments use `{% ... %}` and `{# ... #}`, and `{{` is a
/// literal `{`.
fn render_name(
    template: &str,
    project: &str,
    release: &dyn fmt::Display,
    os: &str,
    arch: &str,
) -> Result<String> {
    let mut env = minijinja::Environment::new();
    env.set_syntax(
        minijinja::syntax::SyntaxConfig::builder()
            .variable_delimiters("{", "}")
            .block_delimiters("{%", "%}")
            .comment_delimiters("{#", "#}")
            .build()
            .context("Building name template syntax")?,
    );
    env.set_auto_escape_callback(|_| minijinja::AutoEscape::None);
    env.set_undefined_behavior(minijinja::UndefinedBehavior::Strict);

    // A `{{` would otherwise open a variable whose expression starts with a
    // map literal, so replace it with an expression for a literal brace.
    let source = template.replace("{{", "{'{'}");

    let compiled = env
        .template_from_str(&source)
        .with_context(|| anyhow!("While parsing name template `{template}`"))?;

    compiled
        .render(minijinja::context! {
            project,
            release => release.to_string(),
            os,
            arch,
        })
        .with_context(|| anyhow!("While rendering name template `{template}`"))
}

#[cfg(test)]
mod tests {
    use super::{DEFAULT_NAME, render_name};

    fn render(template: &str) -> anyhow::Result<String> {
        render_name(template, "my_project", &"1.0.0", "linux", "x86_64")
    }

    #[test]
    fn default_name() {
        assert_eq!(
            render(DEFAULT_NAME).unwrap(),
            "my_project-1.0.0-x86_64-linux"
        );
    }

    #[test]
    fn custom_name() {
        assert_eq!(
            render("{ project }_{os}.{arch}").unwrap(),
            "my_project_linux.x86_64"
        );
        assert_eq!(render("static-name").unwrap(), "static-name");
        assert_eq!(render("{os | upper}").unwrap(), "LINUX");
        assert_eq!(
            render("{project}{% if os == 'linux' %}-gnu{% endif %}{# comment #}").unwrap(),
            "my_project-gnu"
        );
    }

    #[test]
    fn literal_braces() {
        assert_eq!(render("{{project}").unwrap(), "{project}");
        assert_eq!(render("{{{project}}").unwrap(), "{my_project}");
        assert_eq!(render("a}b").unwrap(), "a}b");
    }

    #[test]
    fn errors() {
        let error = format!("{:#}", render("{project}-{nope}").unwrap_err());
        assert!(error.contains("While rendering name template"), "{error}");
        assert!(error.contains("undefined"), "{error}");

        let error = format!("{:#}", render("{project").unwrap_err());
        assert!(error.contains("While parsing name template"), "{error}");

        let error = format!("{:#}", render("{}").unwrap_err());
        assert!(error.contains("While parsing name template"), "{error}");
    }
}
