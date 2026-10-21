use std::fs;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;

use anyhow::{Result, bail};

use super::run_staged;
use crate::overlay::Overlay;

struct Files {
    _dir: tempfile::TempDir,
    toml: PathBuf,
    lock: PathBuf,
    member_lock: PathBuf,
}

impl Files {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let toml = dir.path().join("Cargo.toml");
        let lock = dir.path().join("Cargo.lock");
        fs::create_dir(dir.path().join("member")).unwrap();
        let member_lock = dir.path().join("member").join("Cargo.lock");
        fs::write(&toml, "serde = \"1.0.0\"\n").unwrap();
        fs::write(&lock, "serde 1.0.0\n").unwrap();

        Self {
            _dir: dir,
            toml,
            lock,
            member_lock,
        }
    }

    fn paths(&self) -> Vec<PathBuf> {
        vec![
            self.toml.clone(),
            self.lock.clone(),
            self.member_lock.clone(),
        ]
    }

    fn upgrade(&self) -> Result<()> {
        fs::write(&self.toml, "serde = \"1.0.200\"\n")?;
        fs::write(&self.lock, "serde 1.0.200\n")?;
        Ok(())
    }

    fn assert_original(&self) {
        assert_eq!(
            fs::read_to_string(&self.toml).unwrap(),
            "serde = \"1.0.0\"\n"
        );
        assert_eq!(fs::read_to_string(&self.lock).unwrap(), "serde 1.0.0\n");
        assert!(!self.member_lock.exists());
    }
}

#[test]
fn upgrade_is_restored_and_returned() -> Result<()> {
    let files = Files::new();
    let overlay = Overlay::new();

    let changed = run_staged(&overlay, &files.paths(), || {
        files.upgrade()?;
        // A file which did not exist is created.
        fs::write(&files.member_lock, "new\n")?;
        Ok(())
    })?;

    files.assert_original();

    let changed = changed
        .into_iter()
        .map(|(i, content)| (i, String::from_utf8(content).unwrap()))
        .collect::<Vec<_>>();

    assert_eq!(
        changed,
        [
            (0, String::from("serde = \"1.0.200\"\n")),
            (1, String::from("serde 1.0.200\n")),
            (2, String::from("new\n")),
        ]
    );

    // The files were read through the overlay, so writes staged from the
    // returned contents are verified against the originals.
    for (i, content) in changed {
        overlay.write(&files.paths()[i], content.into_bytes(), 0)?;
    }

    let pending = overlay.pending();
    assert_eq!(pending.len(), 3);
    assert_eq!(
        pending[0].original.as_deref(),
        Some(&b"serde 1.0.0\n"[..]),
        "{}",
        pending[0].path.display()
    );
    assert_eq!(pending[2].original, None);
    Ok(())
}

#[test]
fn nothing_changed() -> Result<()> {
    let files = Files::new();
    let changed = run_staged(&Overlay::new(), &files.paths(), || Ok(()))?;
    assert!(changed.is_empty());
    files.assert_original();
    Ok(())
}

#[test]
fn restored_on_error() {
    let files = Files::new();

    let result = run_staged(&Overlay::new(), &files.paths(), || {
        files.upgrade()?;
        bail!("Command failed")
    });

    assert!(result.is_err());
    files.assert_original();
}

#[test]
fn restored_on_panic() {
    let files = Files::new();

    let result = catch_unwind(AssertUnwindSafe(|| {
        run_staged(&Overlay::new(), &files.paths(), || {
            files.upgrade()?;
            panic!("upgrade panicked");
        })
    }));

    assert!(result.is_err());
    files.assert_original();
}

#[test]
fn refused_with_pending_writes() -> Result<()> {
    let files = Files::new();
    let overlay = Overlay::new();
    overlay.write(&files.toml, b"staged\n".to_vec(), 0)?;

    let mut ran = false;

    let result = run_staged(&overlay, &files.paths(), || {
        ran = true;
        Ok(())
    });

    let error = result.unwrap_err().to_string();
    assert!(error.contains("unsaved changes"), "{error}");
    assert!(!ran);
    files.assert_original();
    Ok(())
}

#[test]
fn refused_if_changed_since_read() -> Result<()> {
    let files = Files::new();
    let overlay = Overlay::new();
    overlay.read(&files.lock)?;
    fs::write(&files.lock, "edited\n")?;

    let mut ran = false;

    let result = run_staged(&overlay, &files.paths(), || {
        ran = true;
        Ok(())
    });

    assert!(result.is_err());
    assert!(!ran);
    assert_eq!(fs::read_to_string(&files.lock)?, "edited\n");
    Ok(())
}
