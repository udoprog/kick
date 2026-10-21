use std::fs;
use std::path::Path;

use anyhow::bail;

use super::{Expected, Overlay, commit, commit_with, write_atomic};

fn read(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

#[test]
fn reads_see_pending_writes() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let new = dir.path().join("sub/new.txt");
    fs::write(&a, "one\n").unwrap();

    let overlay = Overlay::new();
    assert_eq!(overlay.read_to_string(&a).unwrap().as_deref(), Some("one\n"));
    assert!(!overlay.is_file(&new));

    overlay.write(&a, b"two\n".to_vec(), 0).unwrap();
    overlay.write(&new, b"new\n".to_vec(), 1).unwrap();

    assert_eq!(overlay.read_to_string(&a).unwrap().as_deref(), Some("two\n"));
    assert_eq!(
        overlay.read_to_string(&new).unwrap().as_deref(),
        Some("new\n")
    );
    assert!(overlay.is_file(&new));
    // The same file however it is spelled.
    let spelled = dir.path().join("sub/../a.txt");
    assert_eq!(
        overlay.read_to_string(&spelled).unwrap().as_deref(),
        Some("two\n")
    );

    let pending = overlay.pending_in(&dir.path().join("sub"));
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].file_name().unwrap(), "new.txt");

    // Nothing is written to disk.
    assert_eq!(read(&a).as_deref(), Some("one\n"));
    assert!(!new.exists());
}

#[test]
fn writes_keep_the_first_original() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    fs::write(&a, "one\n").unwrap();

    let overlay = Overlay::new();
    overlay.read(&a).unwrap();
    // A file changed on disk after it was read is not picked up.
    fs::write(&a, "changed\n").unwrap();
    overlay.write(&a, b"two\n".to_vec(), 0).unwrap();
    overlay.write(&a, b"three\n".to_vec(), 3).unwrap();

    let pending = overlay.pending();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].original.as_deref(), Some(&b"one\n"[..]));
    assert_eq!(pending[0].content, b"three\n");
    assert_eq!(pending[0].changes.iter().copied().collect::<Vec<_>>(), [0, 3]);

    assert_eq!(
        overlay.expected(),
        [Expected {
            path: super::key(&a),
            original: Some(b"one\n".to_vec()),
        }]
    );
}

#[test]
fn expected_content_is_read() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    fs::write(&a, "on disk\n").unwrap();

    let overlay = Overlay::new();

    overlay.expect(Expected {
        path: a.clone(),
        original: Some(b"staged\n".to_vec()),
    });

    assert_eq!(
        overlay.read_to_string(&a).unwrap().as_deref(),
        Some("staged\n")
    );

    overlay.write(&a, b"new\n".to_vec(), 0).unwrap();

    // The file changed since the write was staged.
    let result = commit(&overlay.pending(), false);
    assert_eq!(result.written.len(), 0);
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(read(&a).as_deref(), Some("on disk\n"));
}

#[test]
fn commit_saves_every_file() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let new = dir.path().join("x/y/new.txt");
    fs::write(&a, "one\n").unwrap();

    let overlay = Overlay::new();
    overlay.write(&a, b"two\n".to_vec(), 0).unwrap();
    overlay.write(&new, b"new\n".to_vec(), 1).unwrap();

    let result = commit(&overlay.pending(), false);
    assert!(result.is_complete());
    assert_eq!(result.written.len(), 2);
    assert_eq!(read(&a).as_deref(), Some("two\n"));
    assert_eq!(read(&new).as_deref(), Some("new\n"));

    for path in &result.written {
        overlay.saved(path);
    }

    assert!(overlay.pending().is_empty());
    assert_eq!(overlay.read_to_string(&a).unwrap().as_deref(), Some("two\n"));
}

#[test]
fn commit_refuses_all_on_conflict() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    let c = dir.path().join("c.txt");
    fs::write(&a, "a\n").unwrap();
    fs::write(&b, "b\n").unwrap();

    let overlay = Overlay::new();
    overlay.write(&a, b"a2\n".to_vec(), 0).unwrap();
    overlay.write(&b, b"b2\n".to_vec(), 1).unwrap();
    overlay.write(&c, b"c2\n".to_vec(), 2).unwrap();

    fs::write(&b, "modified\n").unwrap();
    fs::write(&c, "created\n").unwrap();

    let result = commit(&overlay.pending(), false);
    assert!(!result.is_complete());
    assert!(result.written.is_empty());

    let conflicts = result
        .conflicts
        .iter()
        .map(|c| c.reason)
        .collect::<Vec<_>>();

    assert_eq!(
        conflicts,
        [
            "was modified after changes to it were staged",
            "was created after changes to it were staged"
        ]
    );

    assert_eq!(read(&a).as_deref(), Some("a\n"));
    assert_eq!(read(&b).as_deref(), Some("modified\n"));
    assert_eq!(read(&c).as_deref(), Some("created\n"));

    // A removed file is a conflict too.
    fs::remove_file(&a).unwrap();
    let result = commit(&overlay.pending()[..1], false);
    assert_eq!(
        result.conflicts[0].reason,
        "was removed after changes to it were staged"
    );
}

#[test]
fn commit_partial_saves_what_it_can() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    fs::write(&a, "a\n").unwrap();
    fs::write(&b, "b\n").unwrap();

    let overlay = Overlay::new();
    overlay.write(&a, b"a2\n".to_vec(), 0).unwrap();
    overlay.write(&b, b"b2\n".to_vec(), 1).unwrap();
    fs::write(&b, "modified\n").unwrap();

    let result = commit(&overlay.pending(), true);
    assert_eq!(result.conflicts.len(), 1);
    assert_eq!(result.written, [super::key(&a)]);
    assert_eq!(read(&a).as_deref(), Some("a2\n"));
    assert_eq!(read(&b).as_deref(), Some("modified\n"));
}

#[test]
fn commit_rolls_back_on_failure() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    let new = dir.path().join("new/file.txt");
    fs::write(&a, "a\n").unwrap();
    fs::write(&b, "b\n").unwrap();

    let overlay = Overlay::new();
    overlay.write(&a, b"a2\n".to_vec(), 0).unwrap();
    overlay.write(&b, b"b2\n".to_vec(), 1).unwrap();
    overlay.write(&new, b"new\n".to_vec(), 2).unwrap();

    let pending = overlay.pending();
    // Writes are ordered by path.
    assert_eq!(pending[2].path, super::key(&new));

    let fail = |path: &Path, content: &[u8]| {
        if path.ends_with("file.txt") {
            bail!("Permission denied");
        }

        write_atomic(path, content)
    };

    let result = commit_with(&pending, false, fail);
    assert_eq!(result.errors.len(), 1);
    assert!(result.rollback_errors.is_empty());
    assert!(result.written.is_empty());
    assert_eq!(read(&a).as_deref(), Some("a\n"));
    assert_eq!(read(&b).as_deref(), Some("b\n"));
    assert!(!dir.path().join("new").exists());

    // With partial, the files which can be written are.
    let result = commit_with(&pending, true, fail);
    assert_eq!(result.errors.len(), 1);
    assert_eq!(result.written.len(), 2);
    assert_eq!(read(&a).as_deref(), Some("a2\n"));
    assert_eq!(read(&b).as_deref(), Some("b2\n"));
}

#[test]
fn rollback_removes_created_files() {
    let dir = tempfile::tempdir().unwrap();
    let a = dir.path().join("a/new.txt");
    let b = dir.path().join("b.txt");
    fs::write(&b, "b\n").unwrap();

    let overlay = Overlay::new();
    overlay.write(&a, b"new\n".to_vec(), 0).unwrap();
    overlay.write(&b, b"b2\n".to_vec(), 1).unwrap();

    let fail = |path: &Path, content: &[u8]| {
        if path.ends_with("b.txt") {
            bail!("Permission denied");
        }

        write_atomic(path, content)
    };

    let result = commit_with(&overlay.pending(), false, fail);
    assert_eq!(result.errors.len(), 1);
    assert!(!a.exists());
    assert!(!dir.path().join("a").exists());
    assert_eq!(read(&b).as_deref(), Some("b\n"));
}

#[cfg(unix)]
#[test]
fn write_atomic_keeps_permissions_and_links() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");
    let link = dir.path().join("link.txt");
    fs::write(&target, "old\n").unwrap();
    fs::set_permissions(&target, fs::Permissions::from_mode(0o640)).unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();

    write_atomic(&link, b"new\n").unwrap();

    assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
    assert_eq!(read(&target).as_deref(), Some("new\n"));

    let mode = fs::metadata(&target).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o640);
}
