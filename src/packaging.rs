use core::fmt;
use core::str::FromStr;

use std::env::consts::EXE_EXTENSION;
use std::fs::{self, Metadata};
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail, ensure};
use relative_path::{RelativePath, RelativePathBuf};

use crate::config::PackageFile;
use crate::ctxt::Ctxt;
use crate::glob::Glob;
use crate::model::Repo;

/// Filesystem mode for regular files.
const REGULAR_FILE: u16 = 0o100000;

/// Trait used for interacting with packagers.
pub(crate) trait Packager {
    /// Install a binary. Binaries are assumed to be files that should be
    /// installed into the "default" binary location and have executable
    /// permissions.
    fn add_binary(&mut self, name: &str, path: &Path) -> Result<()>;

    /// Install a single file.
    fn add_file(&mut self, file: &PackageFile, source: &Path, dest: &RelativePath) -> Result<()>;
}

/// Construct a collection of files to install based on the repo configuration.
pub(crate) fn install_files(
    packager: &mut dyn Packager,
    cx: &Ctxt<'_>,
    repo: &Repo,
) -> Result<usize> {
    let workspace = repo.workspace(cx)?;

    let mut binaries = Vec::new();
    let mut names = Vec::new();
    let mut errors = 0;

    if cx.config.package_binaries(repo) {
        for manifest in workspace.packages() {
            manifest.binaries(&mut binaries)?;

            for binary in binaries.drain(..) {
                binary.list(cx, repo, &mut names)?;

                for name in names.drain(..) {
                    let mut b = RelativePathBuf::from(repo.path());

                    b.push("target");
                    b.push("release");
                    b.push(&name);
                    b.set_extension(EXE_EXTENSION);

                    let path = cx.to_path(b);

                    if !path.is_file() {
                        tracing::error!("Binary not found: {}", path.display());
                        errors += 1;
                        continue;
                    }

                    tracing::info!("Adding binary `{name}`: {}", path.display());
                    packager
                        .add_binary(&name, &path)
                        .with_context(|| path.display().to_string())?;
                }
            }
        }
    }

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

            let path = cx.to_path(repo.path().join(&relative));
            let dest = dest.join(file_name);

            tracing::info!("Adding regular file {} to {dest}", path.display());
            packager
                .add_file(file, &path, &dest)
                .with_context(|| path.display().to_string())?;
        }
    }

    Ok(errors)
}

#[derive(Default, Clone, Copy)]
pub(crate) struct Mode {
    raw: u16,
}

impl Mode {
    /// The default executable mode.
    pub(crate) const EXECUTABLE: Self = Self { raw: 0o755 };

    /// The default read-write mode.
    pub(crate) const READ_WRITE: Self = Self { raw: 0o644 };

    /// Check if the mode is executable.
    pub(crate) fn is_executable(self) -> bool {
        self.raw & 0o111 != 0
    }

    /// Coerce into a raw mode for unix regular files.
    pub(crate) fn regular_file(self) -> u16 {
        REGULAR_FILE | self.raw
    }
}

impl FromStr for Mode {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match u16::from_str_radix(s, 8) {
            Ok(raw) => {
                if raw & 0o777 != raw {
                    return Err(anyhow!("file mode `{raw:o}` contains invalid bits"));
                }

                Ok(Self { raw })
            }
            Err(err) => Err(anyhow!("not an octal mode string `{s}`: {err}")),
        }
    }
}

impl fmt::Debug for Mode {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:o}", self.raw)
    }
}

#[cfg(unix)]
fn infer_mode_from_meta(m: Metadata, _: &Path) -> Result<(Mode, bool)> {
    use std::os::unix::fs::PermissionsExt;
    ensure!(m.is_file(), "Not a file");
    let mode = m.permissions().mode() as u16;
    debug_assert!(mode & REGULAR_FILE == REGULAR_FILE);
    Ok((Mode { raw: mode & 0o777 }, mode & 0o111 != 0))
}

#[cfg(not(unix))]
fn infer_mode_from_meta(_: Metadata, path: &Path) -> Result<(Mode, bool)> {
    if path.extension().and_then(|s| s.to_str()) == Some(EXE_EXTENSION) {
        Ok((Mode::EXECUTABLE, true))
    } else {
        Ok((Mode::READ_WRITE, false))
    }
}

/// Infer mode from path.
pub(crate) fn infer_mode(path: &Path) -> Result<(Mode, bool)> {
    infer_mode_inner(path).with_context(|| path.display().to_string())
}

fn infer_mode_inner(path: &Path) -> Result<(Mode, bool)> {
    let m = fs::metadata(path)?;
    infer_mode_from_meta(m, path)
}
