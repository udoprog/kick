use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;
use clap::Parser;
use termcolor::NoColor;

use crate::cli::WithRepos;
use crate::config::{self, Distribution, Os};
use crate::ctxt::{Ctxt, Paths};
use crate::env::Env;
use crate::global::Global;
use crate::model::Repo;
use crate::overlay::Overlay;
use crate::repo_sets::RepoSets;
use crate::system::System;
use crate::templates::Templating;

const LIB_RS: &str = "\
//! A crate.
//!
//! ```toml
//! proj = \"0.1.0\"
//! ```

pub fn hello() {}
";

const README: &str = "\
# proj
A crate.

```toml
proj = \"0.1.0\"
```
";

/// A project registered in a global configuration in a scratch directory.
struct Project {
    dir: tempfile::TempDir,
    home: PathBuf,
    global: Global,
}

impl Project {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let proj = home.join("proj");
        fs::create_dir_all(proj.join("src")).unwrap();

        let path = dir.path().join("config/kick/Kick.toml");
        fs::create_dir_all(path.parent().unwrap()).unwrap();

        fs::write(
            &path,
            "[repo.\"~/proj\"]\nurl = \"https://github.com/example/proj\"\n",
        )
        .unwrap();

        fs::write(
            proj.join("Kick.toml"),
            "disabled = [\"ci\"]\n\
             \n\
             [[version]]\n\
             paths = [\"src/lib.rs\"]\n\
             pattern = 'proj = \"(?P<version>[0-9]+\\.[0-9]+\\.[0-9]+)\"'\n",
        )
        .unwrap();

        fs::write(
            proj.join("Cargo.toml"),
            "[package]\n\
             name = \"proj\"\n\
             version = \"0.1.0\"\n\
             edition = \"2024\"\n",
        )
        .unwrap();

        fs::write(proj.join("src/lib.rs"), LIB_RS).unwrap();
        fs::write(proj.join("README.md"), README).unwrap();

        let global = Global::new(&path, &home, true);
        Self { dir, home, global }
    }

    fn path(&self, path: &str) -> PathBuf {
        self.home.join("proj").join(path)
    }

    fn read(&self, path: &str) -> String {
        fs::read_to_string(self.path(path)).unwrap()
    }

    /// Run with a context for the project.
    fn with_cx<T>(&self, f: impl FnOnce(&mut WithRepos<'_>) -> Result<T>) -> (T, Ctxt<'_>) {
        // Leaked so the context can outlive this function in tests.
        let templating = Box::leak(Box::new(Templating::new().unwrap()));
        let defaults = Box::leak(Box::new(config::defaults()));
        let root: &Path = Box::leak(self.home.clone().into_boxed_path());
        let global = Box::leak(Box::new(self.global.clone()));

        let paths = Paths {
            root,
            current: None,
            config: None,
            cache: None,
            redirect: None,
        };

        let loaded = config::load_all_with(paths, templating, defaults, Some(global));
        assert!(loaded.errors.is_empty(), "{:?}", loaded.errors);
        let config = Box::leak(Box::new(loaded.config));

        let repos: &[Repo] = Box::leak(
            crate::collect_repos(config, root, None)
                .unwrap()
                .0
                .into_boxed_slice(),
        );

        let sets = Box::leak(Box::new(
            RepoSets::new(self.dir.path().join("sets")).unwrap(),
        ));

        let cx = Ctxt {
            term: Arc::new(AtomicBool::new(false)),
            system: Box::leak(Box::new(System::default())),
            git_credentials: &None,
            os: Os::Linux,
            dist: Distribution::Other,
            paths,
            config,
            repos,
            rustc_version: None,
            warnings: RefCell::new(Vec::new()),
            changes: RefCell::new(Vec::new()),
            sets,
            env: Box::leak(Box::new(Env::new())),
            overlay: Overlay::new(),
        };

        let mut with_repos = WithRepos::new(cx, 1);
        let output = f(&mut with_repos).unwrap();
        (output, with_repos.into_cx())
    }
}

fn version(args: &[&str]) -> crate::cli::version::Opts {
    crate::cli::version::Opts::parse_from(["version"].iter().chain(args))
}

fn apply(cx: &Ctxt<'_>, save: bool, partial: bool) -> (String, usize) {
    let mut o = NoColor::new(Vec::new());
    let failed = super::apply_changes(&mut o, cx, save, partial).unwrap();
    (String::from_utf8(o.into_inner()).unwrap(), failed)
}

#[test]
fn version_is_checked_with_its_changes() {
    let project = Project::new();

    let ((), cx) = project.with_cx(|r| crate::cli::version::entry(r, &version(&["--minor"])));

    let pending = cx
        .overlay
        .pending()
        .into_iter()
        .filter(|w| !w.is_unchanged())
        .map(|w| {
            let path = w.path.strip_prefix(project.path("")).unwrap().to_owned();
            (path, String::from_utf8(w.content).unwrap())
        })
        .collect::<Vec<_>>();

    let paths = pending
        .iter()
        .map(|(p, _)| p.to_str().unwrap())
        .collect::<Vec<_>>();

    // The readme is only out of date once the documentation in lib.rs has
    // the new version, so the check must have seen the version change.
    assert_eq!(paths, ["Cargo.toml", "README.md", "src/lib.rs"]);

    for (_, content) in &pending {
        assert!(content.contains("0.2.0"), "{content}");
        assert!(!content.contains("0.1.0"), "{content}");
    }

    // Nothing is written without saving.
    let (output, failed) = apply(&cx, false, false);
    assert_eq!(failed, 0);
    assert!(output.contains("-proj = \"0.1.0\""), "{output}");
    assert!(output.contains("+proj = \"0.2.0\""), "{output}");
    assert_eq!(project.read("src/lib.rs"), LIB_RS);
    assert_eq!(project.read("README.md"), README);

    let (_, failed) = apply(&cx, true, false);
    assert_eq!(failed, 0);
    assert!(cx.changes().iter().all(|c| c.written));
    // Fixes from the check are made to the manifest with the new version.
    let cargo_toml = project.read("Cargo.toml");
    assert!(cargo_toml.contains("version = \"0.2.0\""), "{cargo_toml}");
    assert!(
        cargo_toml.contains("readme = \"README.md\""),
        "{cargo_toml}"
    );
    assert!(project.read("src/lib.rs").contains("proj = \"0.2.0\""));
    assert!(project.read("README.md").contains("proj = \"0.2.0\""));
}

#[test]
fn version_without_check() {
    let project = Project::new();

    let ((), cx) =
        project.with_cx(|r| crate::cli::version::entry(r, &version(&["--minor", "--no-check"])));

    let mut paths = cx
        .overlay
        .pending()
        .into_iter()
        .map(|w| w.path)
        .collect::<Vec<_>>();

    paths.sort();
    assert_eq!(
        paths,
        [project.path("Cargo.toml"), project.path("src/lib.rs")]
    );
}

#[test]
fn save_refuses_files_changed_since_staged() {
    let project = Project::new();

    let ((), cx) = project.with_cx(|r| crate::cli::version::entry(r, &version(&["--minor"])));

    fs::write(project.path("README.md"), "edited\n").unwrap();

    let (_, failed) = apply(&cx, true, false);
    assert_eq!(failed, cx.changes().len());
    assert!(cx.changes().iter().all(|c| !c.written));
    assert!(project.read("Cargo.toml").contains("version = \"0.1.0\""));
    assert_eq!(project.read("src/lib.rs"), LIB_RS);
    assert_eq!(project.read("README.md"), "edited\n");

    // The expected content of every file is kept for when the changes are
    // staged again.
    assert_eq!(cx.overlay.expected().len(), 3);

    // With partial, everything but the conflicting file is saved.
    let (_, failed) = apply(&cx, true, true);
    assert_eq!(failed, 1);
    assert!(project.read("Cargo.toml").contains("version = \"0.2.0\""));
    assert!(project.read("src/lib.rs").contains("proj = \"0.2.0\""));
    assert_eq!(project.read("README.md"), "edited\n");
    assert_eq!(cx.overlay.expected().len(), 1);
}

#[test]
fn staged_changes_are_verified_when_loaded() {
    let project = Project::new();
    let changes_file = super::ChangesFile {
        path: project.dir.path().join("changes.gz"),
        legacy: None,
    };

    let ((), cx) = project.with_cx(|r| crate::cli::version::entry(r, &version(&["--minor"])));
    super::save_changes(&cx.changes(), &cx.overlay.expected(), &changes_file.path).unwrap();
    drop(cx);

    // Loading staged changes shows the same diff.
    let ((), cx) = project.with_cx(|r| crate::cli::changes::entry(&r.cx, &changes_file));
    let (output, _) = apply(&cx, false, false);
    assert!(output.contains("+proj = \"0.2.0\""), "{output}");
    drop(cx);

    fs::write(project.path("src/lib.rs"), "// edited\n").unwrap();

    let ((), cx) = project.with_cx(|r| crate::cli::changes::entry(&r.cx, &changes_file));
    let (_, failed) = apply(&cx, true, false);
    assert!(failed > 0);
    assert_eq!(project.read("README.md"), README);
    assert!(project.read("Cargo.toml").contains("version = \"0.1.0\""));
}

#[test]
fn upgrade_is_staged() {
    use relative_path::RelativePathBuf;

    use crate::changes::{Change, UpgradedFile};

    let project = Project::new();
    let changes_file = super::ChangesFile {
        path: project.dir.path().join("changes.gz"),
        legacy: None,
    };
    let original = project.read("Cargo.toml");
    let upgraded = original.replace(
        "edition = \"2024\"",
        "edition = \"2024\"\n\n[dependencies]\nserde = \"1.0.200\"",
    );

    let stage = |r: &mut WithRepos<'_>| {
        r.cx.change(Change::Upgrade {
            path: RelativePathBuf::from("proj"),
            files: vec![UpgradedFile {
                path: RelativePathBuf::from("proj/Cargo.toml"),
                content: upgraded.clone().into_bytes(),
            }],
        })
    };

    let ((), cx) = project.with_cx(stage);
    let (output, failed) = apply(&cx, false, false);
    assert_eq!(failed, 0);
    assert!(output.contains("+serde = \"1.0.200\""), "{output}");
    assert_eq!(project.read("Cargo.toml"), original);
    super::save_changes(&cx.changes(), &cx.overlay.expected(), &changes_file.path).unwrap();
    drop(cx);

    let ((), cx) = project.with_cx(|r| crate::cli::changes::entry(&r.cx, &changes_file));
    let (output, failed) = apply(&cx, true, false);
    assert_eq!(failed, 0);
    assert!(output.contains("Cargo.toml: Saved"), "{output}");
    assert_eq!(project.read("Cargo.toml"), upgraded);
}

/// The readme is in sync with lib.rs before versions change.
#[test]
fn check_alone_leaves_readme() {
    let project = Project::new();

    let ((), cx) = project.with_cx(|r| block_on(crate::cli::check::entry(r, &Default::default())));

    let paths = cx
        .overlay
        .pending()
        .into_iter()
        .filter(|w| !w.is_unchanged())
        .map(|w| w.path)
        .collect::<Vec<_>>();

    assert_eq!(paths, [project.path("Cargo.toml")]);
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(f)
}
