//! An in-memory overlay over the filesystem which holds pending file writes.
//!
//! Reads made through the overlay see pending writes, and record the content
//! of a file the first time it is read so that a write derived from it can be
//! verified against the filesystem before it is saved.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use musli::{Decode, Encode};
use tempfile::NamedTempFile;

/// The recorded original content of a file, `None` if it did not exist.
type Original = Option<Vec<u8>>;

#[derive(Default)]
struct Entry {
    /// How the path is displayed.
    display: PathBuf,
    /// The content the file had when it was first read or written through the
    /// overlay.
    original: Original,
    /// Pending content to write.
    pending: Option<Vec<u8>>,
    /// Indexes of the changes which contributed to the pending content.
    changes: BTreeSet<usize>,
}

/// An in-memory filesystem overlay.
#[derive(Default)]
pub(crate) struct Overlay {
    files: RefCell<BTreeMap<PathBuf, Entry>>,
}

/// A file which is expected to have the given content before a pending write
/// to it is saved, as stored with staged changes.
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
pub(crate) struct Expected {
    pub(crate) path: PathBuf,
    pub(crate) original: Option<Vec<u8>>,
}

/// A pending write to a file.
#[derive(Debug, Clone)]
pub(crate) struct PendingWrite {
    /// The absolute path to the file.
    pub(crate) path: PathBuf,
    /// How the path is displayed.
    pub(crate) display: PathBuf,
    /// The content the file is expected to have, `None` if it is expected to
    /// be missing.
    pub(crate) original: Original,
    /// The content to write.
    pub(crate) content: Vec<u8>,
    /// Indexes of changes which contributed to this write.
    pub(crate) changes: BTreeSet<usize>,
}

impl PendingWrite {
    /// Test if the write would leave the file as it is.
    pub(crate) fn is_unchanged(&self) -> bool {
        self.original.as_deref() == Some(self.content.as_slice())
    }
}

impl Overlay {
    /// Construct a new empty overlay.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Record the content a file is expected to have, as when loading staged
    /// changes. Reads through the overlay see this content instead of what is
    /// on disk, so that changes are derived from what they were staged
    /// against.
    pub(crate) fn expect(&self, expected: Expected) {
        let path = key(&expected.path);
        let display = display(&path);
        let mut files = self.files.borrow_mut();
        let entry = files.entry(path).or_default();
        entry.display = display;
        entry.original = expected.original;
        entry.pending = None;
        entry.changes.clear();
    }

    /// Read a file through the overlay, `None` if it does not exist.
    pub(crate) fn read(&self, path: &Path) -> io::Result<Option<Vec<u8>>> {
        let key = key(path);
        let mut files = self.files.borrow_mut();

        if let Some(entry) = files.get(&key) {
            return Ok(entry.pending.as_ref().or(entry.original.as_ref()).cloned());
        }

        let original = read_original(path)?;

        files.insert(
            key,
            Entry {
                display: path.to_owned(),
                original: original.clone(),
                ..Entry::default()
            },
        );

        Ok(original)
    }

    /// Read a file through the overlay as a string.
    pub(crate) fn read_to_string(&self, path: &Path) -> io::Result<Option<String>> {
        let Some(bytes) = self.read(path)? else {
            return Ok(None);
        };

        String::from_utf8(bytes)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }

    /// Test if the given path is a file as seen through the overlay.
    pub(crate) fn is_file(&self, path: &Path) -> bool {
        let files = self.files.borrow();

        match files.get(&key(path)) {
            Some(entry) => entry.pending.is_some() || entry.original.is_some(),
            None => path.is_file(),
        }
    }

    /// Get the names of files which are pending to be written directly in
    /// the given directory.
    pub(crate) fn pending_in(&self, dir: &Path) -> Vec<PathBuf> {
        let dir = key(dir);
        let files = self.files.borrow();

        files
            .iter()
            .filter(|(path, e)| e.pending.is_some() && path.parent() == Some(dir.as_path()))
            .flat_map(|(path, _)| Some(dir.join(path.file_name()?)))
            .collect()
    }

    /// Stage a write of the given content, contributed by the change with the
    /// given index.
    pub(crate) fn write(&self, path: &Path, content: Vec<u8>, change: usize) -> Result<()> {
        let key = key(path);
        let mut files = self.files.borrow_mut();

        let entry = match files.entry(key) {
            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
            std::collections::btree_map::Entry::Vacant(e) => {
                let original = read_original(path).with_context(|| path.display().to_string())?;

                e.insert(Entry {
                    display: path.to_owned(),
                    original,
                    ..Entry::default()
                })
            }
        };

        entry.pending = Some(content);
        entry.changes.insert(change);
        Ok(())
    }

    /// Get every pending write.
    pub(crate) fn pending(&self) -> Vec<PendingWrite> {
        let files = self.files.borrow();
        let mut writes = Vec::new();

        for (path, entry) in files.iter() {
            let Some(content) = &entry.pending else {
                continue;
            };

            writes.push(PendingWrite {
                path: path.clone(),
                display: entry.display.clone(),
                original: entry.original.clone(),
                content: content.clone(),
                changes: entry.changes.clone(),
            });
        }

        writes
    }

    /// Mark the given write as saved, so its content is the new original.
    pub(crate) fn saved(&self, path: &Path) {
        if let Some(entry) = self.files.borrow_mut().get_mut(&key(path)) {
            entry.original = entry.pending.take();
            entry.changes.clear();
        }
    }

    /// The expected original content of every file with a pending write, to be
    /// stored with staged changes.
    pub(crate) fn expected(&self) -> Vec<Expected> {
        let files = self.files.borrow();

        files
            .iter()
            .filter(|(_, e)| e.pending.is_some())
            .map(|(path, e)| Expected {
                path: path.clone(),
                original: e.original.clone(),
            })
            .collect()
    }
}

/// A file whose content differs from what a pending write was derived from.
#[derive(Debug)]
pub(crate) struct Conflict {
    pub(crate) display: PathBuf,
    pub(crate) reason: &'static str,
}

impl fmt::Display for Conflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.display.display(), self.reason)
    }
}

/// The outcome of [`commit`].
#[derive(Debug, Default)]
pub(crate) struct Commit {
    /// Paths which were written.
    pub(crate) written: Vec<PathBuf>,
    /// Files which were not written since they changed after their writes
    /// were staged.
    pub(crate) conflicts: Vec<Conflict>,
    /// Files which failed to be written.
    pub(crate) errors: Vec<(PathBuf, anyhow::Error)>,
    /// Files which failed to be restored after another write failed.
    pub(crate) rollback_errors: Vec<(PathBuf, anyhow::Error)>,
}

impl Commit {
    /// Test if every write was saved.
    pub(crate) fn is_complete(&self) -> bool {
        self.conflicts.is_empty() && self.errors.is_empty()
    }
}

/// Save the given writes.
///
/// Every file is first verified to still have the content its write was
/// derived from. Unless `partial` is set, nothing is written if any file
/// changed, and if writing one file fails the files already written are
/// restored. With `partial` set, every write which can be saved is saved.
pub(crate) fn commit(writes: &[PendingWrite], partial: bool) -> Commit {
    commit_with(writes, partial, write_atomic)
}

fn commit_with(
    writes: &[PendingWrite],
    partial: bool,
    mut write: impl FnMut(&Path, &[u8]) -> Result<()>,
) -> Commit {
    let mut commit = Commit::default();
    let mut ready = Vec::new();

    for w in writes {
        match verify(w) {
            Ok(()) => ready.push(w),
            Err(conflict) => commit.conflicts.push(conflict),
        }
    }

    if !partial && !commit.conflicts.is_empty() {
        return commit;
    }

    let mut done = Vec::<(&PendingWrite, Vec<PathBuf>)>::new();

    for w in ready {
        if w.is_unchanged() {
            commit.written.push(w.path.clone());
            continue;
        }

        let created = match create_parents(&w.path) {
            Ok(created) => created,
            Err(error) => {
                commit.errors.push((w.display.clone(), error));

                if !partial {
                    break;
                }

                continue;
            }
        };

        match write(&w.path, &w.content).with_context(|| w.display.display().to_string()) {
            Ok(()) => {
                commit.written.push(w.path.clone());
                done.push((w, created));
            }
            Err(error) => {
                remove_dirs(&created);
                commit.errors.push((w.display.clone(), error));

                if !partial {
                    break;
                }
            }
        }
    }

    if !partial && !commit.errors.is_empty() {
        commit.written.clear();

        for (w, created) in done.into_iter().rev() {
            let result = match &w.original {
                Some(original) => write(&w.path, original),
                None => fs::remove_file(&w.path).map_err(anyhow::Error::from),
            };

            match result {
                Ok(()) => remove_dirs(&created),
                Err(error) => commit.rollback_errors.push((w.display.clone(), error)),
            }
        }
    }

    commit
}

/// Verify that the file still has the content the write was derived from.
fn verify(w: &PendingWrite) -> Result<(), Conflict> {
    let current = match read_original(&w.path) {
        Ok(current) => current,
        Err(..) => {
            return Err(Conflict {
                display: w.display.clone(),
                reason: "could not be read to verify that it is unchanged",
            });
        }
    };

    match (&w.original, &current) {
        (None, None) => Ok(()),
        (Some(a), Some(b)) if a == b => Ok(()),
        (None, Some(..)) => Err(Conflict {
            display: w.display.clone(),
            reason: "was created after changes to it were staged",
        }),
        (Some(..), None) => Err(Conflict {
            display: w.display.clone(),
            reason: "was removed after changes to it were staged",
        }),
        (Some(..), Some(..)) => Err(Conflict {
            display: w.display.clone(),
            reason: "was modified after changes to it were staged",
        }),
    }
}

/// Create missing parent directories of the given path, returning the ones
/// created from the outermost one.
fn create_parents(path: &Path) -> Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    let mut current = path.parent();

    while let Some(dir) = current {
        if dir.as_os_str().is_empty() || dir.exists() {
            break;
        }

        missing.push(dir.to_owned());
        current = dir.parent();
    }

    missing.reverse();

    for (n, dir) in missing.iter().enumerate() {
        if let Err(error) = fs::create_dir(dir) {
            remove_dirs(&missing[..n]);
            return Err(error).with_context(|| dir.display().to_string());
        }
    }

    Ok(missing)
}

/// Remove directories created by [`create_parents`] if they are empty.
fn remove_dirs(created: &[PathBuf]) {
    for dir in created.iter().rev() {
        _ = fs::remove_dir(dir);
    }
}

/// Write a file by persisting a temporary file next to it, keeping the
/// permissions of an existing file and writing through symbolic links.
fn write_atomic(path: &Path, content: &[u8]) -> Result<()> {
    let path = match fs::canonicalize(path) {
        Ok(path) => path,
        Err(e) if e.kind() == io::ErrorKind::NotFound => path.to_owned(),
        Err(e) => return Err(e.into()),
    };

    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));

    let permissions = match fs::metadata(&path) {
        Ok(m) => Some(m.permissions()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
    };

    let mut builder = tempfile::Builder::new();
    builder.prefix(".kick-");

    #[cfg(unix)]
    if permissions.is_none() {
        use std::os::unix::fs::PermissionsExt;
        // Subject to the umask, as for any other new file.
        builder.permissions(fs::Permissions::from_mode(0o666));
    }

    let mut file: NamedTempFile = builder.tempfile_in(parent)?;
    file.write_all(content)?;
    file.as_file().sync_all()?;

    if let Some(permissions) = permissions {
        fs::set_permissions(file.path(), permissions)?;
    }

    file.persist(&path).map_err(|e| anyhow!(e.error))?;
    Ok(())
}

fn read_original(path: &Path) -> io::Result<Original> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// The key a path is stored under: absolute and lexically normalized, so
/// that it is the same however it was spelled and from wherever kick runs.
fn key(path: &Path) -> PathBuf {
    let path = std::path::absolute(path).unwrap_or_else(|_| path.to_owned());
    let mut out = PathBuf::new();

    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push(c);
                }
            }
            c => out.push(c),
        }
    }

    out
}

/// How to display a path stored under its key: relative to the current
/// directory when it is under it.
fn display(path: &Path) -> PathBuf {
    let Ok(current) = std::env::current_dir() else {
        return path.to_owned();
    };

    match path.strip_prefix(&current) {
        Ok(rest) if !rest.as_os_str().is_empty() => rest.to_owned(),
        _ => path.to_owned(),
    }
}

#[cfg(test)]
mod tests;
