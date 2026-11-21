use std::env::consts::{ARCH, EXE_SUFFIX, OS};
use std::fs::{self, File};
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::SharedOptions;
use crate::ctxt::Ctxt;
use crate::octokit::Asset;

pub(crate) async fn entry(cx: &mut Ctxt<'_>, _: &SharedOptions) -> Result<()> {
    let current = crate::VERSION;
    let client = cx.octokit()?;

    let Some(release) = client.latest_release(crate::OWNER, crate::REPO).await? else {
        bail!("No release found");
    };

    let latest = release.tag_name.trim_start_matches('v');

    if !is_newer(latest, current)? {
        println!("kick {current} is up to date");
        return Ok(());
    }

    let asset = select_asset(&release.assets, ARCH, OS)?;

    let exe = std::env::current_exe()
        .and_then(fs::canonicalize)
        .context("Locating the current executable")?;
    let dir = exe
        .parent()
        .context("Current executable has no directory")?;

    let work = dir.join(format!(".kick-update{}", std::process::id()));
    _ = fs::remove_dir_all(&work);
    fs::create_dir_all(&work).with_context(|| work.display().to_string())?;

    println!("Downloading {}", asset.name);

    let result = async {
        let bytes = client
            .download(&asset.browser_download_url)
            .await
            .with_context(|| asset.browser_download_url.clone())?;
        let archive = work.join(&asset.name);
        fs::write(&archive, &bytes).with_context(|| archive.display().to_string())?;
        let new = work.join(format!("kick{EXE_SUFFIX}"));
        extract_binary(&archive, &new)?;
        replace(&new, &exe)
    }
    .await;

    _ = fs::remove_dir_all(&work);
    result?;
    println!("kick updated from {current} to {latest}");
    Ok(())
}

/// Whether `latest` is newer than `current`, comparing the leading numeric
/// segments split on `.` and `-`, so both `1.2.3` and dated `2026-05-04`
/// releases order as expected.
fn is_newer(latest: &str, current: &str) -> Result<bool> {
    Ok(parse_version(latest)? > parse_version(current)?)
}

fn parse_version(version: &str) -> Result<Vec<u64>> {
    let mut parts = version
        .trim_start_matches('v')
        .split(['.', '-'])
        .map_while(|p| p.parse::<u64>().ok())
        .collect::<Vec<_>>();

    if parts.is_empty() {
        bail!("Bad version `{version}`");
    }

    while parts.len() > 1 && parts.last() == Some(&0) {
        parts.pop();
    }

    Ok(parts)
}

/// The asset named `kick-<release>-<arch>-<os>.<ext>` for this platform.
fn select_asset<'a>(assets: &'a [Asset], arch: &str, os: &str) -> Result<&'a Asset> {
    let ext = if os == "windows" { "zip" } else { "tar.gz" };
    let suffix = format!("-{arch}-{os}.{ext}");

    let Some(asset) = assets
        .iter()
        .find(|a| a.name.starts_with("kick-") && a.name.ends_with(&suffix))
    else {
        bail!("No prebuilt kick for {arch}-{os} in the latest release");
    };

    Ok(asset)
}

fn extract_binary(archive: &Path, to: &Path) -> Result<()> {
    let name = format!("kick{EXE_SUFFIX}");
    let context = || archive.display().to_string();
    let file = File::open(archive).with_context(context)?;

    let out = || File::create(to).with_context(|| to.display().to_string());

    if archive.extension().is_some_and(|e| e == "zip") {
        let mut zip = zip::ZipArchive::new(io::BufReader::new(file)).with_context(context)?;
        let mut entry = zip
            .by_name(&name)
            .with_context(|| format!("{}: {name}", archive.display()))?;
        io::copy(&mut entry, &mut out()?).with_context(context)?;
        return Ok(());
    }

    let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(io::BufReader::new(file)));

    for entry in tar.entries().with_context(context)? {
        let mut entry = entry.with_context(context)?;
        let path = entry.path().with_context(context)?;

        if path.components().eq(Path::new(&name).components()) {
            io::copy(&mut entry, &mut out()?).with_context(context)?;
            return Ok(());
        }
    }

    bail!("{}: No {name} in the archive", archive.display())
}

/// Renames `new` over `exe`. On Windows the running exe is first moved aside
/// to `kick.old.exe`, and restored if the rename fails.
fn replace(new: &Path, exe: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(new, fs::Permissions::from_mode(0o755))
            .with_context(|| new.display().to_string())?;
    }

    let context = || format!("Replacing {}", exe.display());

    if cfg!(windows) {
        let old: PathBuf = exe.with_file_name("kick.old.exe");
        _ = fs::remove_file(&old);
        fs::rename(exe, &old).with_context(context)?;

        if let Err(e) = fs::rename(new, exe) {
            _ = fs::rename(&old, exe);
            return Err(e).with_context(context);
        }

        return Ok(());
    }

    fs::rename(new, exe).with_context(context)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets(names: &[&str]) -> Vec<Asset> {
        names
            .iter()
            .enumerate()
            .map(|(id, n)| Asset {
                name: n.to_string(),
                id: id as u64,
                browser_download_url: format!("https://example.com/{n}"),
            })
            .collect()
    }

    #[test]
    fn compares_versions_numerically() {
        let newer = |a, b| is_newer(a, b).unwrap();
        assert!(newer("0.1.1", "0.1.0"));
        assert!(newer("0.10.0", "0.9.9"));
        assert!(newer("1.0", "0.9.9"));
        assert!(newer("v0.2.0", "0.1.0"));
        assert!(!newer("0.1.0", "0.1.0"));
        assert!(!newer("0.1", "0.1.0"));
        assert!(!newer("0.0.9", "0.1.0"));
        assert!(newer("0.1.0", "0.0.0"));
        assert!(newer("2026-05-04", "2026-05-03"));
        assert!(newer("2026-05-04", "2026-04-30"));
        assert!(newer("2026-05-04-1", "2026-05-04"));
        assert!(!newer("2026-05-04-nightly", "2026-05-04"));
        assert!(newer("2026-05-04", "0.0.0"));
        assert!(is_newer("nightly", "0.1.0").is_err());
    }

    #[test]
    fn selects_the_platform_asset() {
        let list = assets(&[
            "kick-0.2.0-x86_64-linux.tar.gz",
            "kick-0.2.0-aarch64-macos.tar.gz",
            "kick-0.2.0-x86_64-windows.zip",
            "checksums.txt",
        ]);

        let name = |arch, os| select_asset(&list, arch, os).map(|a| a.name.as_str()).ok();

        assert_eq!(
            name("x86_64", "linux"),
            Some("kick-0.2.0-x86_64-linux.tar.gz")
        );
        assert_eq!(
            name("aarch64", "macos"),
            Some("kick-0.2.0-aarch64-macos.tar.gz")
        );
        assert_eq!(
            name("x86_64", "windows"),
            Some("kick-0.2.0-x86_64-windows.zip")
        );
        assert_eq!(name("aarch64", "linux"), None);
        assert_eq!(name("x86_64", "macos"), None);
    }
}
