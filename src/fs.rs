use std::fmt;
use std::fs::{self, Permissions};
use std::io;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use anyhow::{bail, Result};

#[must_use]
pub(crate) enum FileIssue {
    Mode,
    Error(io::Error),
}

impl FileIssue {
    pub(crate) fn fix(&self, p: impl AsRef<Path>) -> Result<()> {
        let p = p.as_ref();

        match self {
            Self::Mode => {
                let m = fs::metadata(p)?;

                #[cfg(unix)]
                {
                    let mut perm = m.permissions();
                    perm.set_mode(0o500);
                    fs::set_permissions(p, perm)?;
                }
            }
            Self::Error(..) => {
                bail!("Can't fix errors")
            }
        }

        Ok(())
    }
}

impl fmt::Display for FileIssue {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileIssue::Mode => write!(f, "Expected mode 0500"),
            FileIssue::Error(e) => e.fmt(f),
        }
    }
}

/// Test if the path is secure.
pub(crate) fn test_secure(path: impl AsRef<Path>) -> Vec<FileIssue> {
    let mut issues = Vec::new();

    match fs::metadata(path) {
        Ok(m) =>
        {
            #[cfg(unix)]
            if m.mode() != 0o500 {
                issues.push(FileIssue::Mode);
            }
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => {
            issues.push(FileIssue::Error(e));
        }
    }

    issues
}

/// Set as secure file permissions as possible.
pub(crate) fn set_secure(p: &mut Permissions) {
    #[cfg(unix)]
    {
        p.set_mode(0o500);
    }
}
