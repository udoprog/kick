use std::borrow::Cow;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

use anyhow::Result;
use bstr::ByteSlice;

use crate::process::Command;

const FIND_REQUIRES: &str = "/usr/lib/rpm/find-requires";

#[allow(unused)]
pub(super) fn detect() -> bool {
    let Ok(m) = fs::metadata(FIND_REQUIRES) else {
        return false;
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        // Test if the file is executable.
        if m.permissions().mode() & 0o111 == 0 {
            tracing::warn!("Found {FIND_REQUIRES}, but it's not executable");
            return false;
        }
    }

    true
}

pub(super) fn find<P>(exe: P) -> Result<Vec<String>>
where
    P: AsRef<Path>,
{
    let exe = exe.as_ref();
    let mut set = Vec::new();

    let mut child = Command::new(FIND_REQUIRES)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let mut stdin = child.stdin()?;
    let executable = as_bytes(exe);
    stdin.write_all(executable.as_ref())?;
    stdin.write_all(b"\n")?;
    drop(stdin);

    let output = child.wait_with_output()?;

    for line in output.stdout.split(|&b| b == b'\n') {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        set.push(String::from_utf8(line.to_vec())?);
    }

    Ok(set)
}

#[cfg(unix)]
fn as_bytes(path: &Path) -> Cow<'_, [u8]> {
    use std::os::unix::ffi::OsStrExt;
    Cow::Borrowed(path.as_os_str().as_bytes())
}

#[cfg(not(unix))]
fn as_bytes(path: &Path) -> Cow<'_, [u8]> {
    Cow::Owned(path.as_os_str().to_string_lossy().into_owned().into_bytes())
}
