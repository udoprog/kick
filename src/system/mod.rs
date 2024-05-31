pub(crate) mod git;
mod powershell;
mod wsl;

use std::env;
use std::env::consts::EXE_EXTENSION;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use anyhow::Result;

pub(crate) use self::git::Git;
pub(crate) use self::powershell::PowerShell;
pub(crate) use self::wsl::Wsl;

/// Detect system commands.
#[derive(Default)]
pub(crate) struct System {
    pub(crate) git: Vec<Git>,
    pub(crate) wsl: Vec<Wsl>,
    pub(crate) powershell: Vec<PowerShell>,
}

/// Detect everything useful we can find in the environment.
pub(crate) fn detect() -> Result<System> {
    let mut system = System::default();

    if let Some(path) = env::var_os("GIT_PATH") {
        let path = PathBuf::from(path);

        if let Some(status) = git::test(path.as_os_str())? {
            if status.success() {
                system.git.push(Git::new(path));
            }
        }
    }

    let git = |s: &mut System, path: &Path| s.git.push(Git::new(path.to_owned()));
    let wsl = |s: &mut System, path: &Path| s.wsl.push(Wsl::new(path.to_owned()));
    let powershell =
        |s: &mut System, path: &Path| s.powershell.push(PowerShell::new(path.to_owned()));

    let tests: &[(
        &str,
        fn(&OsStr) -> Result<Option<ExitStatus>>,
        fn(&mut System, &Path),
    )] = &[
        ("git", git::test, git),
        ("wsl", wsl::test, wsl),
        ("powershell", powershell::test, powershell),
    ];

    if let Some(path) = env::var_os("PATH") {
        // Look for the command in the PATH.
        for mut path in env::split_paths(&path) {
            for &(name, test, add_to) in tests {
                path.push(name);
                path.set_extension(EXE_EXTENSION);

                if let Some(status) = test(path.as_os_str())? {
                    if status.success() {
                        add_to(&mut system, &path);
                    }
                }

                path.pop();
            }
        }
    }

    Ok(system)
}
