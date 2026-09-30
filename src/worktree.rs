//! Detection of git checkouts nested inside of a registered repo, such as a
//! git worktree of the repo placed in a subdirectory of its main checkout.

use std::fs;
use std::path::{Path, PathBuf};

use relative_path::{RelativePath, RelativePathBuf};

/// A git checkout nested strictly below a registered repo.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Nested {
    /// A git worktree of the repo it is nested in.
    Worktree {
        repo: RelativePathBuf,
        checkout: RelativePathBuf,
    },
    /// Any other checkout, like a nested clone or a worktree of a different
    /// repo.
    Other {
        repo: RelativePathBuf,
        checkout: RelativePathBuf,
    },
    /// A worktree whose `.git` file links to a `gitdir` which does not exist,
    /// such as after its main checkout was moved.
    Broken {
        repo: RelativePathBuf,
        checkout: RelativePathBuf,
        gitdir: PathBuf,
    },
}

/// The outcome of resolving the git common directory of a checkout.
#[derive(Debug)]
enum CommonDir {
    /// The canonical common directory.
    Found(PathBuf),
    /// The `.git` file links to a `gitdir` which does not exist.
    Missing(PathBuf),
    /// The common directory could not be determined.
    Unknown,
}

/// Detect if `current` is inside of a git checkout which is nested strictly
/// below one of the registered `repos`.
///
/// Returns `None` if the nearest checkout is itself a registered repo, is not
/// below any registered repo, or if there is no checkout.
pub(crate) fn detect<'a>(
    root: &Path,
    current: &RelativePath,
    repos: impl IntoIterator<Item = &'a RelativePath>,
) -> Option<Nested> {
    let checkout = crate::unregistered_git_checkout(root, current)?;

    let mut enclosing = None::<&RelativePath>;

    for repo in repos {
        if repo == checkout {
            return None;
        }

        if repo.as_str().is_empty() || !checkout.starts_with(repo) {
            continue;
        }

        // Pick the innermost enclosing repo.
        if enclosing.is_none_or(|e| repo.starts_with(e)) {
            enclosing = Some(repo);
        }
    }

    let repo = enclosing?;

    let common = common_dir(&checkout.to_path(root));
    let repo_git = repo.to_path(root).join(".git").canonicalize();

    let repo = repo.to_owned();
    let checkout = checkout.to_owned();

    Some(match (common, repo_git) {
        (CommonDir::Found(a), Ok(b)) if a == b => Nested::Worktree { repo, checkout },
        (CommonDir::Missing(gitdir), _) => Nested::Broken {
            repo,
            checkout,
            gitdir,
        },
        _ => Nested::Other { repo, checkout },
    })
}

/// Resolve the canonical git common directory of the checkout at `dir`, which
/// for a worktree is the `.git` directory of its main checkout.
fn common_dir(dir: &Path) -> CommonDir {
    let dot_git = dir.join(".git");

    let Ok(meta) = fs::metadata(&dot_git) else {
        return CommonDir::Unknown;
    };

    if meta.is_dir() {
        return found(dot_git.canonicalize());
    }

    let Ok(content) = fs::read_to_string(&dot_git) else {
        return CommonDir::Unknown;
    };

    let Some(git_dir) = content
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))
    else {
        return CommonDir::Unknown;
    };

    // Relative gitdir paths are relative to the checkout.
    let git_dir = dir.join(git_dir.trim());

    if !git_dir.try_exists().unwrap_or(true) {
        return CommonDir::Missing(git_dir);
    }

    let common = match fs::read_to_string(git_dir.join("commondir")) {
        Ok(common) => git_dir.join(common.trim()),
        Err(_) => git_dir,
    };

    found(common.canonicalize())
}

fn found(path: std::io::Result<PathBuf>) -> CommonDir {
    match path {
        Ok(path) => CommonDir::Found(path),
        Err(_) => CommonDir::Unknown,
    }
}

/// Build the error explaining that the current directory is inside a checkout
/// nested in a registered repo which kick can't use as that repo.
pub(crate) fn nested_checkout_message(
    root: &Path,
    checkout_dir: &Path,
    repo: &RelativePath,
    repo_dir: &Path,
) -> String {
    format!(
        "The current directory is inside git checkout `{checkout_dir}`, which is nested inside repo `{repo}` of kick's {kick_toml} at `{root}` but is not a git worktree of it.\n\
         \n\
         Run kick from the main checkout `{repo_dir}` instead, or pass `--all`, `-p`/`--path` or `--set` to select repos explicitly.",
        checkout_dir = checkout_dir.display(),
        kick_toml = crate::KICK_TOML,
        root = root.display(),
        repo_dir = repo_dir.display(),
    )
}

/// Build the error explaining that the current directory is inside a worktree
/// nested in a registered repo whose link to its `gitdir` is broken.
pub(crate) fn broken_worktree_message(
    root: &Path,
    checkout_dir: &Path,
    gitdir: &Path,
    repo: &RelativePath,
    repo_dir: &Path,
) -> String {
    format!(
        "The current directory is inside git worktree `{checkout_dir}`, which is nested inside repo `{repo}` of kick's {kick_toml} at `{root}`, but its link is broken: its .git file points to gitdir `{gitdir}`, which does not exist. This usually happens when the repo was moved.\n\
         \n\
         To fix the link, run `git worktree repair {checkout_dir}` from the main checkout `{repo_dir}`. Otherwise run kick from the main checkout, or pass `--all`, `-p`/`--path` or `--set` to select repos explicitly.",
        checkout_dir = checkout_dir.display(),
        gitdir = gitdir.display(),
        kick_toml = crate::KICK_TOML,
        root = root.display(),
        repo_dir = repo_dir.display(),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use relative_path::RelativePath;

    use super::{Nested, broken_worktree_message, detect};

    fn git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .args(["-c", "user.name=test", "-c", "user.email=test@example.com"])
            .args([
                "-c",
                "init.defaultBranch=main",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(dir)
            .output()
            .expect("running git");

        assert!(
            status.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&status.stderr)
        );
    }

    fn init(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        git(dir, &["init", "-q"]);
        fs::write(dir.join("README"), "hello").unwrap();
        git(dir, &["add", "README"]);
        git(dir, &["commit", "-q", "-m", "initial"]);
    }

    #[test]
    fn detects_nested_checkouts() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        init(&root.join("repos/track"));
        init(&root.join("repos/other"));
        fs::create_dir_all(root.join("repos/track/src/deep")).unwrap();

        // A worktree of the same repo, nested inside of it.
        git(
            &root.join("repos/track"),
            &["worktree", "add", "-q", ".claude/worktrees/wt"],
        );
        fs::create_dir_all(root.join("repos/track/.claude/worktrees/wt/src")).unwrap();
        // A worktree of a different repo, nested inside of track.
        git(
            &root.join("repos/other"),
            &["worktree", "add", "-q", "../track/foreign"],
        );
        // A nested clone.
        init(&root.join("repos/track/vendor/clone"));
        // A worktree outside of any registered repo.
        git(
            &root.join("repos/track"),
            &["worktree", "add", "-q", "../track-sibling"],
        );

        // A worktree of track whose link points to a gitdir that no longer
        // exists, as if track had been moved from `repos/old-track`.
        git(
            &root.join("repos/track"),
            &["worktree", "add", "-q", ".claude/worktrees/stale"],
        );
        let missing = root.join("repos/old-track/.git/worktrees/stale");
        fs::write(
            root.join("repos/track/.claude/worktrees/stale/.git"),
            format!("gitdir: {}\n", missing.display()),
        )
        .unwrap();

        let repos = [
            RelativePath::new("repos/track"),
            RelativePath::new("repos/other"),
        ];

        let check = |p: &str| detect(root, RelativePath::new(p), repos);

        let worktree = Some(Nested::Worktree {
            repo: "repos/track".into(),
            checkout: "repos/track/.claude/worktrees/wt".into(),
        });

        assert_eq!(check("repos/track/.claude/worktrees/wt"), worktree);
        assert_eq!(check("repos/track/.claude/worktrees/wt/src"), worktree);

        assert_eq!(
            check("repos/track/foreign"),
            Some(Nested::Other {
                repo: "repos/track".into(),
                checkout: "repos/track/foreign".into(),
            })
        );
        assert_eq!(
            check("repos/track/vendor/clone"),
            Some(Nested::Other {
                repo: "repos/track".into(),
                checkout: "repos/track/vendor/clone".into(),
            })
        );

        let broken = Some(Nested::Broken {
            repo: "repos/track".into(),
            checkout: "repos/track/.claude/worktrees/stale".into(),
            gitdir: missing.clone(),
        });

        assert_eq!(check("repos/track/.claude/worktrees/stale"), broken);

        // Plain directories and the main checkouts themselves.
        assert_eq!(check("repos/track"), None);
        assert_eq!(check("repos/track/src/deep"), None);
        assert_eq!(check("repos/track/.claude"), None);
        assert_eq!(check("repos/other"), None);
        assert_eq!(check("repos"), None);
        assert_eq!(check(""), None);
        // Sibling worktrees are left to the unregistered checkout check.
        assert_eq!(check("repos/track-sibling"), None);
    }

    #[test]
    fn broken_worktree_message_names_gitdir() {
        let message = broken_worktree_message(
            Path::new("/root"),
            Path::new("/root/repos/track/.claude/worktrees/stale"),
            Path::new("/root/repos/old-track/.git/worktrees/stale"),
            RelativePath::new("repos/track"),
            Path::new("/root/repos/track"),
        );

        assert!(message.contains("gitdir `/root/repos/old-track/.git/worktrees/stale`"));
        assert!(message.contains("does not exist"));
        assert!(message.contains("repo was moved"));
        assert!(message.contains(
            "`git worktree repair /root/repos/track/.claude/worktrees/stale` from the main checkout `/root/repos/track`"
        ));
        assert!(message.contains("`--all`, `-p`/`--path` or `--set`"));
    }
}
