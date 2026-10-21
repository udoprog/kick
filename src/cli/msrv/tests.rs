use std::fs;

use anyhow::Result;

use super::{move_aside, replace_temporarily};
use crate::restore::Restore;

#[test]
fn temporary_manifest_is_restored() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("Cargo.toml");
    let original = dir.path().join("Cargo.toml.original");
    let lock = dir.path().join("Cargo.lock");
    let lock_original = dir.path().join("Cargo.lock.original");
    fs::write(&path, "original")?;
    fs::write(&lock, "lock")?;

    let mut restore = Restore::default();
    replace_temporarily(&mut restore, &path, &original, b"modified".to_vec())?;
    move_aside(&mut restore, &lock, &lock_original)?;
    assert_eq!(fs::read_to_string(&path)?, "modified");
    assert!(!lock.exists());

    // Cargo generates a new lock file, which is replaced.
    fs::write(&lock, "generated")?;
    restore.restore()?;

    assert_eq!(fs::read_to_string(&path)?, "original");
    assert_eq!(fs::read_to_string(&lock)?, "lock");
    assert!(!original.exists());
    assert!(!lock_original.exists());
    Ok(())
}

#[test]
fn changed_temporary_manifest_is_kept() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("Cargo.toml");
    let original = dir.path().join("Cargo.toml.original");
    fs::write(&path, "original")?;

    let mut restore = Restore::default();
    replace_temporarily(&mut restore, &path, &original, b"modified".to_vec())?;
    fs::write(&path, "edited")?;

    let error = restore.restore().unwrap_err().to_string();
    assert!(error.contains(&path.display().to_string()), "{error}");
    assert!(error.contains(&original.display().to_string()), "{error}");
    assert_eq!(fs::read_to_string(&path)?, "edited");
    assert_eq!(fs::read_to_string(&original)?, "original");

    // A leftover original is never overwritten by another run.
    let mut restore = Restore::default();
    assert!(replace_temporarily(&mut restore, &path, &original, b"modified".to_vec()).is_err());
    assert_eq!(fs::read_to_string(&path)?, "edited");
    assert_eq!(fs::read_to_string(&original)?, "original");
    Ok(())
}
