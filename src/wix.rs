use std::env::consts::EXE_EXTENSION;
use std::env::{self, consts};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};

use crate::process::Command;

pub(crate) struct Wix {
    candle_bin: PathBuf,
    light_bin: PathBuf,
}

impl Wix {
    /// Find and store the location of available wix commands.
    pub(crate) fn find() -> Result<Self> {
        let wix_path = PathBuf::from(env::var_os("WIX").context("Missing environment: WIX")?);

        let candle_bin = wix_path
            .join("bin")
            .join("candle")
            .with_extension(EXE_EXTENSION);

        let light_bin = wix_path
            .join("bin")
            .join("light")
            .with_extension(EXE_EXTENSION);

        ensure!(
            wix_path.is_dir(),
            "Missing: {} (from WIX environment)",
            wix_path.display()
        );

        ensure!(
            candle_bin.is_file(),
            "Missing: {} (from WIX environment)",
            candle_bin.display()
        );

        ensure!(
            light_bin.is_file(),
            "Missing: {} (from WIX environment)",
            light_bin.display()
        );

        Ok(Self {
            candle_bin,
            light_bin,
        })
    }

    pub(crate) fn build(
        &self,
        source: impl AsRef<Path>,
        target_wixobj: impl AsRef<Path>,
        root: impl AsRef<Path>,
        binary_name: impl AsRef<str>,
        binary_path: impl AsRef<Path>,
        file_version: impl fmt::Display,
    ) -> Result<()> {
        let source = source.as_ref();
        let target_wixobj = target_wixobj.as_ref();
        let root = root.as_ref();
        let binary_name = binary_name.as_ref();
        let binary_path = binary_path.as_ref();

        if let Some(parent) = target_wixobj.parent() {
            if !parent.is_dir() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create: {}", parent.display()))?;
            }
        }

        if target_wixobj.is_file() {
            fs::remove_file(target_wixobj)
                .with_context(|| format!("Failed to remove: {}", target_wixobj.display()))?;
        }

        let (win64, platform, program_files_folder) = match consts::ARCH {
            "x86_64" => ("yes", "x64", "ProgramFiles64Folder"),
            "x86" => ("no", "x86", "ProgramFilesFolder"),
            arch => bail!("Unsupported arch: {arch}"),
        };

        let mut command = Command::new(&self.candle_bin);

        let status = command
            .arg(format!("-dRoot={}", root.display()))
            .arg(format!("-dVersion={file_version}"))
            .arg(format!("-dPlatform={platform}"))
            .arg(format!("-dWin64={win64}"))
            .arg(format!("-dProgramFilesFolder={program_files_folder}"))
            .arg(format!("-dBinaryName={binary_name}"))
            .arg(format!("-dBinaryPath={}", binary_path.display()))
            .args(["-ext", "WixUtilExtension"])
            .arg("-o")
            .arg(target_wixobj)
            .arg(source)
            .status()?;

        ensure!(status.success(), "Failed to build: {}", source.display());
        Ok(())
    }

    /// Link the current project.
    pub(crate) fn link(
        &self,
        target_wixobj: impl AsRef<Path>,
        installer_path: impl AsRef<Path>,
    ) -> Result<()> {
        let target_wixobj = target_wixobj.as_ref();

        if !target_wixobj.is_file() {
            bail!("Missing: {}", target_wixobj.display());
        }

        let installer_path = installer_path.as_ref();

        if let Some(parent) = installer_path.parent() {
            if !parent.is_dir() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("Failed to create: {}", parent.display()))?;
            }
        }

        if installer_path.is_file() {
            fs::remove_file(installer_path)
                .with_context(|| format!("Failed to remove: {}", installer_path.display()))?;
        }

        let mut command = Command::new(&self.light_bin);

        let status = command
            .arg("-spdb")
            .args(["-ext", "WixUIExtension"])
            .args(["-ext", "WixUtilExtension"])
            .arg("-cultures:en-us")
            .arg(target_wixobj)
            .arg("-out")
            .arg(installer_path)
            .status()?;

        ensure!(
            status.success(),
            "Failed to link: {}",
            installer_path.display()
        );

        Ok(())
    }
}
