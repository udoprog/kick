use std::env;
use std::env::consts::EXE_EXTENSION;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;

use anyhow::Result;

use super::git;
use super::wsl;

/// Detect system commands.
#[derive(Default)]
pub(crate) struct System {
    pub(crate) git: Vec<git::Git>,
    pub(crate) wsl: Vec<wsl::Wsl>,
}

/// Detect everything useful we can find in the environment.
pub(crate) fn detect() -> Result<System> {
    let mut system = System::default();

    if let Some(path) = env::var_os("GIT_PATH") {
        let path = PathBuf::from(path);

        if let Some(status) = git::version(path.as_os_str())? {
            if status.success() {
                system.git.push(git::Git::new(path));
            }
        }
    }

    let add_git =
        |system: &mut System, path: &Path| system.git.push(git::Git::new(path.to_owned()));
    let add_wsl =
        |system: &mut System, path: &Path| system.wsl.push(wsl::Wsl::new(path.to_owned()));

    let tests: &[(
        &str,
        fn(&OsStr) -> Result<Option<ExitStatus>>,
        fn(&mut System, &Path),
    )] = &[
        ("git", git::version, add_git),
        ("wsl", wsl::version, add_wsl),
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
