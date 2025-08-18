use std::path::{Path, PathBuf};

use crate::utils::move_paths;

#[derive(Default)]
pub(crate) struct Restore {
    paths: Vec<(PathBuf, PathBuf)>,
}

impl Restore {
    pub(crate) fn insert(&mut self, from: impl AsRef<Path>, to: impl AsRef<Path>) {
        self.paths
            .push((from.as_ref().to_owned(), to.as_ref().to_owned()));
    }

    pub(crate) fn restore(&mut self) {
        for (from, to) in self.paths.drain(..) {
            if let Err(error) = move_paths(&from, &to) {
                tracing::error!("Failed to restore {}: {}", from.display(), error);
            }
        }
    }
}

impl Drop for Restore {
    #[inline]
    fn drop(&mut self) {
        self.restore();
    }
}
