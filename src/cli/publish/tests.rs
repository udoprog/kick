use std::fs;

use anyhow::Result;

use crate::cargo::Manifest;
use crate::workspace::Crates;

use super::{
    CircularDev, ManifestBackup, check_leftover_backup, is_already_exists, plan,
    strip_dev_dependencies,
};

fn workspace(manifests: &[(&str, &str)]) -> Result<Crates> {
    let manifests = manifests
        .iter()
        .map(|(path, toml)| Manifest::parse_for_test(path, toml))
        .collect::<Result<Vec<_>>>()?;

    Ok(Crates::from_manifests_for_test(manifests))
}

fn dev(target: Option<&str>, key: &str, package: &str) -> CircularDev {
    CircularDev {
        target: target.map(str::to_owned),
        key: key.to_owned(),
        package: package.to_owned(),
    }
}

/// Order of crates, and the circular dev-dependencies and dependencies of
/// each.
fn order(crates: &Crates) -> Result<Vec<(String, Vec<CircularDev>, Vec<String>)>> {
    Ok(plan(crates)?
        .into_iter()
        .map(|p| (p.name.to_owned(), p.circular_dev, p.depends_on))
        .collect())
}

const ROOT: &str = r#"
[workspace]
members = ["a", "b"]
"#;

/// `a` depends on `b`, while `b` dev-depends on `a` with an exact version.
const A: &str = r#"
[package]
name = "a"
version = "0.1.0"

[dependencies]
b = { path = "../b", version = "0.1.0" }
"#;

const B: &str = r#"
[package]
name = "b"
version = "0.1.0"

[dependencies]
serde = "1.0"

[dev-dependencies]
a = { path = "../a", version = "=0.1.0" }
anyhow = "1.0"
"#;

#[test]
fn circular_dev_dependency() -> Result<()> {
    let crates = workspace(&[
        ("Cargo.toml", ROOT),
        ("a/Cargo.toml", A),
        ("b/Cargo.toml", B),
    ])?;

    assert_eq!(
        order(&crates)?,
        [
            (
                String::from("b"),
                vec![dev(None, "a", "a")],
                Vec::<String>::new()
            ),
            (String::from("a"), vec![], vec![String::from("b")]),
        ]
    );

    Ok(())
}

#[test]
fn strips_only_circular_dev_dependency() -> Result<()> {
    let mut manifest = Manifest::parse_for_test("b/Cargo.toml", B)?;
    let removed = strip_dev_dependencies(&mut manifest, false, &[dev(None, "a", "a")]);
    assert_eq!(removed.len(), 1);

    let output = manifest.to_toml_string();
    assert!(!output.contains("../a"), "{output}");
    // Other dev-dependencies are kept.
    assert!(output.contains("anyhow = \"1.0\""), "{output}");
    assert!(output.contains("serde = \"1.0\""), "{output}");
    Ok(())
}

#[test]
fn path_only_dev_dependency_is_not_an_edge() -> Result<()> {
    // Cargo strips dev-dependencies without a version when packaging, so this
    // is not a cycle at all.
    let b = r#"
[package]
name = "b"
version = "0.1.0"

[dev-dependencies]
a = { path = "../a" }
"#;

    let crates = workspace(&[
        ("Cargo.toml", ROOT),
        ("a/Cargo.toml", A),
        ("b/Cargo.toml", b),
    ])?;

    assert_eq!(
        order(&crates)?,
        [
            (String::from("b"), vec![], vec![]),
            (String::from("a"), vec![], vec![String::from("b")]),
        ]
    );

    Ok(())
}

#[test]
fn target_tables() -> Result<()> {
    // `a` only depends on `b` through a target table, and the circular
    // dev-dependency of `b` is renamed and in a target table too.
    let a = r#"
[package]
name = "a"
version = "0.1.0"

[target.'cfg(any())'.dependencies]
b = { path = "../b", version = "=0.1.0" }
"#;

    let b = r#"
[package]
name = "b"
version = "0.1.0"

[target.'cfg(unix)'.dev-dependencies]
a2 = { package = "a", path = "../a", version = "0.1.0" }
"#;

    let crates = workspace(&[
        ("Cargo.toml", ROOT),
        ("a/Cargo.toml", a),
        ("b/Cargo.toml", b),
    ])?;

    let circular = vec![dev(Some("cfg(unix)"), "a2", "a")];

    assert_eq!(
        order(&crates)?,
        [
            (String::from("b"), circular.clone(), vec![]),
            (String::from("a"), vec![], vec![String::from("b")]),
        ]
    );

    let mut manifest = Manifest::parse_for_test("b/Cargo.toml", b)?;
    let removed = strip_dev_dependencies(&mut manifest, false, &circular);
    assert_eq!(removed.len(), 1);
    assert!(!manifest.to_toml_string().contains("a2"));
    Ok(())
}

#[test]
fn keeps_dev_dependencies_outside_of_cycle() -> Result<()> {
    // `c` dev-depends on `a`, which is not part of a cycle with `c`, so only
    // the dev-dependency of `b` which closes the cycle is removed and `c` is
    // published after `a`.
    let root = r#"
[workspace]
members = ["a", "b", "c"]
"#;

    let c = r#"
[package]
name = "c"
version = "0.1.0"

[dev-dependencies]
a = { path = "../a", version = "0.1.0" }
"#;

    let crates = workspace(&[
        ("Cargo.toml", root),
        ("c/Cargo.toml", c),
        ("a/Cargo.toml", A),
        ("b/Cargo.toml", B),
    ])?;

    assert_eq!(
        order(&crates)?,
        [
            (String::from("b"), vec![dev(None, "a", "a")], vec![]),
            (String::from("a"), vec![], vec![String::from("b")]),
            (String::from("c"), vec![], vec![String::from("a")]),
        ]
    );

    Ok(())
}

#[test]
fn runtime_cycle_is_an_error() -> Result<()> {
    let b = r#"
[package]
name = "b"
version = "0.1.0"

[build-dependencies]
a = { path = "../a", version = "0.1.0" }
"#;

    let crates = workspace(&[
        ("Cargo.toml", ROOT),
        ("a/Cargo.toml", A),
        ("b/Cargo.toml", b),
    ])?;
    let error = plan(&crates).err().expect("expected an error");
    assert!(error.to_string().contains("cyclic dependencies"), "{error}");
    Ok(())
}

#[test]
fn manifest_backup_restores() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cargo_toml = dir.path().join("Cargo.toml");
    fs::write(&cargo_toml, B)?;

    let mut manifest = Manifest::parse_for_test("Cargo.toml", B)?;
    strip_dev_dependencies(&mut manifest, false, &[dev(None, "a", "a")]);

    {
        let _backup = ManifestBackup::replace(dir.path(), &manifest)?;
        assert_ne!(fs::read_to_string(&cargo_toml)?, B);
        // A second publish while the first one is in progress is refused.
        assert!(ManifestBackup::replace(dir.path(), &manifest).is_err());
        // Dropped without an explicit restore, like on an error or panic.
    }

    assert_eq!(fs::read_to_string(&cargo_toml)?, B);
    // The backup directory is cleaned up.
    assert!(!dir.path().join("target").exists());
    check_leftover_backup(dir.path())?;
    Ok(())
}

#[test]
fn leftover_backup_is_detected() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let cargo_toml = dir.path().join("Cargo.toml");
    fs::write(&cargo_toml, "modified")?;
    fs::write(dir.path().join("Cargo.toml.keep"), B)?;

    let manifest = Manifest::parse_for_test("Cargo.toml", B)?;
    assert!(check_leftover_backup(dir.path()).is_err());
    assert!(ManifestBackup::replace(dir.path(), &manifest).is_err());
    // Nothing is touched.
    assert_eq!(fs::read_to_string(&cargo_toml)?, "modified");
    assert_eq!(fs::read_to_string(dir.path().join("Cargo.toml.keep"))?, B);
    Ok(())
}

#[test]
fn already_exists() {
    assert!(is_already_exists(
        "error: crate musli-core@0.1.9 already exists on crates.io index\n",
        "musli-core"
    ));
    assert!(is_already_exists(
        "\x1b[1m\x1b[33mwarning\x1b[0m: crate musli@0.1.9 already exists on crates.io index",
        "musli"
    ));
    assert!(!is_already_exists(
        "error: crate musli-core@0.1.9 already exists on crates.io index",
        "musli"
    ));
    assert!(!is_already_exists("   Uploading musli v0.1.9", "musli"));
}
