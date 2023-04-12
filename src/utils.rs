use std::fmt;
use std::path::{Path, PathBuf};

use relative_path::RelativePath;

/// Helper to format command outputs.
pub(crate) struct CommandRepr<'a, S>(&'a [S]);

impl<'a, S> CommandRepr<'a, S> {
    pub(crate) fn new(command: &'a [S]) -> Self {
        Self(command)
    }
}

impl<S> fmt::Display for CommandRepr<'_, S>
where
    S: AsRef<str>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut it = self.0.iter();
        let last = it.next_back();

        for part in it {
            write!(f, "{} ", part.as_ref())?;
        }

        if let Some(part) = last {
            write!(f, "{}", part.as_ref())?;
        }

        Ok(())
    }
}

/// Proper path conversion.
pub(crate) fn to_path<A, B>(path: A, root: B) -> PathBuf
where
    A: AsRef<RelativePath>,
    B: AsRef<Path>,
{
    if path.as_ref().components().next().is_none() && root.as_ref().components().next().is_none() {
        return PathBuf::from(std::path::Component::CurDir.as_os_str());
    }

    path.as_ref().to_path(root)
}
