use std::borrow::Cow;
use std::collections::BTreeSet;
use std::io::Write;
use std::path::Path;
use std::process::Stdio;

use anyhow::Result;
use bstr::ByteSlice;

use crate::process::Command;

const FIND_REQUIRES: &str = "/usr/lib/rpm/find-requires";

pub(super) fn find_requires(executable: &Path) -> Result<BTreeSet<String>> {
    let mut set = BTreeSet::new();

    let mut child = Command::new(FIND_REQUIRES)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;

    let mut stdin = child.stdin()?;
    let executable = as_bytes(executable);
    stdin.write_all(executable.as_ref())?;
    stdin.write_all(b"\n")?;
    drop(stdin);

    let output = child.wait_with_output()?;

    for line in output.stdout.split(|&b| b == b'\n') {
        let line = line.trim();

        if line.is_empty() {
            continue;
        }

        set.insert(String::from_utf8(line.to_vec())?);
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
