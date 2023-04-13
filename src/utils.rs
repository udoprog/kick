use std::path::{Path, PathBuf};

use relative_path::RelativePath;

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
