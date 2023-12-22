use std::env::consts::EXE_EXTENSION;
use std::path::PathBuf;

use anyhow::{bail, Result};
use relative_path::{RelativePath, RelativePathBuf};

use crate::config::PackageFile;
use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::model::Repo;

/// Construct a collection of files to install based on the repo configuration.
pub(crate) fn install_files<'a>(cx: &Ctxt<'a>, repo: &Repo) -> Result<Vec<InstallFile<'a>>> {
    let workspace = repo.workspace(cx)?;

    let mut files = Vec::new();

    let package = workspace.primary_package()?;
    let name = package.name()?;

    let mut buf = RelativePathBuf::from(repo.path());
    buf.push("target/release");
    buf.push(name);
    buf.set_extension(EXE_EXTENSION);

    files.push(InstallFile::Binary(name.to_owned(), cx.to_path(buf)));

    for file in cx.config.package_files(repo) {
        let from = cx.to_path(repo.path());

        let source = RelativePath::new(&file.source);
        let glob = Glob::new(&from, source);
        let dest = RelativePath::new(&file.dest);

        for source in glob.matcher() {
            let relative = source?;

            let Some(file_name) = relative.file_name() else {
                bail!("Missing file name: {relative}");
            };

            let source = cx.to_path(repo.path().join(&relative));
            let dest = dest.join(file_name);
            files.push(InstallFile::File(file, source, dest));
        }
    }

    Ok(files)
}

pub(crate) enum InstallFile<'a> {
    /// Binary file to be installed.
    Binary(String, PathBuf),
    /// Install the specified file.
    File(&'a PackageFile, PathBuf, RelativePathBuf),
}
