use core::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::Parser;
use relative_path::RelativePathBuf;

use crate::ctxt::{Ctxt, Paths};
use crate::model::RepoRef;

#[derive(Debug, Default, Parser)]
pub(super) struct OutputOpts {
    /// Output directory to write to.
    #[clap(long, value_name = "output")]
    output: Option<RelativePathBuf>,
}

impl OutputOpts {
    /// Make an appropriate output directory.
    pub(super) fn make_directory<'a>(
        &self,
        cx: &Ctxt<'a>,
        repo: &'a RepoRef,
        name: &str,
    ) -> OutputDirectory<'a> {
        let path = match &self.output {
            Some(output) => repo.path().join(output),
            None => {
                let mut path = repo.path().to_owned();
                path.push("target");
                path.push(name);
                path
            }
        };

        OutputDirectory {
            paths: cx.paths,
            path,
        }
    }
}

pub(super) struct OutputDirectory<'a> {
    paths: Paths<'a>,
    path: RelativePathBuf,
}

impl OutputDirectory<'_> {
    /// Create the path to a file inside of the output directory.
    ///
    /// After calling this function, the output directory is guaranteed to have
    /// been created.
    pub(super) fn make_path<N>(&self, name: N) -> Result<PathBuf>
    where
        N: fmt::Display,
    {
        let mut path = self.paths.to_path(&self.path);

        if !path.is_dir() {
            fs::create_dir_all(&path)
                .with_context(|| anyhow!("Creating directory: {}", path.display()))?;
        }

        path.push(name.to_string());

        tracing::info!("Writing {}", path.display());
        Ok(path)
    }

    /// Create a file in the output directory.
    pub(super) fn create_file<N>(&self, name: N) -> Result<CreatedFile>
    where
        N: fmt::Display,
    {
        let path = self.make_path(name)?;
        let file =
            File::create(&path).with_context(|| anyhow!("Creating file: {}", path.display()))?;

        Ok(CreatedFile { path, file })
    }
}

pub(super) struct CreatedFile {
    path: PathBuf,
    file: File,
}

impl CreatedFile {
    /// Get the path to the created file.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Write for CreatedFile {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.file.write(buf)
    }

    #[inline]
    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}
