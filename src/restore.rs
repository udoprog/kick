use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

use crate::utils::move_paths;

struct Entry {
    from: PathBuf,
    to: PathBuf,
    /// What kick wrote to `to`, which it must still have to be replaced.
    written: Option<Vec<u8>>,
}

/// Files moved out of the way, which are moved back when restored or
/// dropped.
#[derive(Default)]
pub(crate) struct Restore {
    paths: Vec<Entry>,
}

impl Restore {
    /// Move `from` back to `to`, replacing whatever is at `to`.
    pub(crate) fn insert(&mut self, from: impl AsRef<Path>, to: impl AsRef<Path>) {
        self.paths.push(Entry {
            from: from.as_ref().to_owned(),
            to: to.as_ref().to_owned(),
            written: None,
        });
    }

    /// Move `from` back to `to`, unless `to` no longer has the content kick
    /// temporarily wrote to it, see [`restore_written`].
    pub(crate) fn insert_written(
        &mut self,
        from: impl AsRef<Path>,
        to: impl AsRef<Path>,
        written: Vec<u8>,
    ) {
        self.paths.push(Entry {
            from: from.as_ref().to_owned(),
            to: to.as_ref().to_owned(),
            written: Some(written),
        });
    }

    /// Restore every file, returning the first error after trying all of
    /// them.
    pub(crate) fn restore(&mut self) -> Result<()> {
        let mut first = None;

        for e in self.paths.drain(..) {
            let result = match &e.written {
                Some(written) => restore_written(&e.from, &e.to, written),
                None => move_paths(&e.from, &e.to),
            };

            if let Err(error) = result {
                tracing::error!("Failed to restore {}: {error:#}", e.to.display());
                first.get_or_insert(error);
            }
        }

        match first {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

impl Drop for Restore {
    #[inline]
    fn drop(&mut self) {
        _ = self.restore();
    }
}

/// Move the original file kept at `kept` back to `path`, which kick
/// temporarily replaced with `written`.
///
/// If `path` no longer has the written content, something else changed it in
/// the meantime. It is then left as it is, the original is left at `kept`,
/// and an error naming both is returned.
pub(crate) fn restore_written(kept: &Path, path: &Path, written: &[u8]) -> Result<()> {
    match fs::read(path) {
        Ok(current) if current != written => {
            bail!(
                "{}: changed while kick had temporarily modified it, so it was kept as it is; its original content is in {}",
                path.display(),
                kept.display()
            );
        }
        Ok(..) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e).context(path.display().to_string()),
    }

    move_paths(kept, path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use anyhow::Result;

    use super::{Restore, restore_written};

    #[test]
    fn restores_unchanged() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("Cargo.toml");
        let kept = dir.path().join("Cargo.toml.original");
        fs::write(&kept, "original")?;
        fs::write(&path, "written")?;

        restore_written(&kept, &path, b"written")?;
        assert_eq!(fs::read_to_string(&path)?, "original");
        assert!(!kept.exists());
        Ok(())
    }

    #[test]
    fn keeps_changed() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("Cargo.toml");
        let kept = dir.path().join("Cargo.toml.original");
        fs::write(&kept, "original")?;
        fs::write(&path, "edited")?;

        let mut restore = Restore::default();
        restore.insert_written(&kept, &path, b"written".to_vec());
        let error = restore.restore().unwrap_err().to_string();

        assert!(error.contains(&path.display().to_string()), "{error}");
        assert!(error.contains(&kept.display().to_string()), "{error}");
        assert_eq!(fs::read_to_string(&path)?, "edited");
        assert_eq!(fs::read_to_string(&kept)?, "original");
        Ok(())
    }

    #[test]
    fn restores_unverified_and_on_drop() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("Cargo.lock");
        let kept = dir.path().join("Cargo.lock.original");
        fs::write(&kept, "original")?;
        fs::write(&path, "generated")?;

        {
            let mut restore = Restore::default();
            restore.insert(&kept, &path);
        }

        assert_eq!(fs::read_to_string(&path)?, "original");
        assert!(!kept.exists());
        Ok(())
    }
}
