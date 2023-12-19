use std::env::consts::EXE_EXTENSION;
use std::env::{self, consts};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, ensure, Context, Result};

use crate::process::Command;

pub(crate) struct Builder {
    binary_name: String,
    binary_path: PathBuf,
    candle_bin: PathBuf,
    light_bin: PathBuf,
    wixobj_path: PathBuf,
    installer_path: PathBuf,
}

impl Builder {
    /// Construct a new WIX builder.
    pub(crate) fn new<B, O>(
        binary_path: B,
        output: O,
        name: impl fmt::Display,
        release: impl fmt::Display,
    ) -> Result<Self>
    where
        B: AsRef<Path>,
        O: AsRef<Path>,
    {
        let wix_env = env::var_os("WIX").context("Missing environment: WIX")?;
        let wix_bin = PathBuf::from(wix_env).join("bin");

        ensure!(wix_bin.is_dir(), "missing: {}", wix_bin.display());

        let candle_bin = wix_bin.join("candle").with_extension(EXE_EXTENSION);
        ensure!(candle_bin.is_file(), "missing: {}", candle_bin.display());

        let light_bin = wix_bin.join("light").with_extension(EXE_EXTENSION);
        ensure!(light_bin.is_file(), "missing: {}", light_bin.display());

        let base = format!(
            "{name}-{release}-{os}-{arch}",
            os = consts::OS,
            arch = consts::ARCH
        );

        let output = output.as_ref();
        let wixobj_path = output.join(format!("{base}.wixobj"));
        let installer_path = output.join(format!("{base}.msi"));

        let binary_path = binary_path.as_ref().to_owned();

        let Some(binary_name) = binary_path.file_name().and_then(|name| name.to_str()) else {
            bail!("Missing or invalid file name: {}", binary_path.display());
        };

        Ok(Self {
            binary_name: binary_name.into(),
            binary_path,
            candle_bin,
            light_bin,
            wixobj_path,
            installer_path,
        })
    }

    pub(crate) fn build<S>(&self, source: S, file_version: &str) -> Result<()>
    where
        S: AsRef<Path>,
    {
        let source = source.as_ref();

        if self.wixobj_path.is_file() {
            fs::remove_file(&self.wixobj_path)
                .with_context(|| format!("Failed to remove: {}", self.wixobj_path.display()))?;
        }

        let (win64, platform, program_files_folder) = match consts::ARCH {
            "x86_64" => ("yes", "x64", "ProgramFiles64Folder"),
            "x86" => ("no", "x86", "ProgramFilesFolder"),
            arch => bail!("Unsupported arch: {arch}"),
        };

        let mut command = Command::new(&self.candle_bin);

        let status = command
            .arg(format!("-dVersion={}", file_version))
            .arg(format!("-dPlatform={}", platform))
            .arg(format!("-dWin64={}", win64))
            .arg(format!("-dProgramFilesFolder={}", program_files_folder))
            .arg(format!("-dBinaryName={}", self.binary_name))
            .arg(format!("-dBinaryPath={}", self.binary_path.display()))
            .args(["-ext", "WixUtilExtension"])
            .arg("-o")
            .arg(&self.wixobj_path)
            .arg(source)
            .status()?;

        ensure!(status.success(), "Failed to build: {}", source.display());
        Ok(())
    }

    /// Link the current project.
    pub(crate) fn link(&self) -> Result<()> {
        if !self.wixobj_path.is_file() {
            bail!("Missing: {}", self.wixobj_path.display());
        }

        if self.installer_path.is_file() {
            fs::remove_file(&self.installer_path)
                .with_context(|| format!("Failed to remove: {}", self.installer_path.display()))?;
        }

        let mut command = Command::new(&self.light_bin);

        let status = command
            .arg("-spdb")
            .args(["-ext", "WixUIExtension"])
            .args(["-ext", "WixUtilExtension"])
            .arg("-cultures:en-us")
            .arg(&self.wixobj_path)
            .arg("-out")
            .arg(&self.installer_path)
            .status()?;

        ensure!(
            status.success(),
            "Failed to link: {}",
            self.installer_path.display()
        );
        Ok(())
    }
}
