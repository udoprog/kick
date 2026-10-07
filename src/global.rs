//! The user-global kick configuration.
//!
//! This is a `Kick.toml` stored in kick's user configuration directory, such as
//! `~/.config/kick/Kick.toml` on Linux. It has the same schema as a project
//! `Kick.toml` and is loaded after every project `Kick.toml` as the least
//! specific layer.
//!
//! Paths in it, including the `[repo."<path>"]` keys, may be absolute, start
//! with `~/`, or be relative in which case they are resolved against the
//! user's home directory. Environment variables are interpolated in them as
//! described in [`crate::interpolate`].

use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use relative_path::RelativePathBuf;
use toml_edit::{DocumentMut, Item, Table};

use crate::interpolate;

/// The user-global configuration.
#[derive(Debug, Clone)]
pub(crate) struct Global {
    /// The path of the global `Kick.toml`.
    pub(crate) path: PathBuf,
    /// The home directory which relative paths are resolved against.
    pub(crate) home: PathBuf,
    /// Whether the global configuration provides the repos kick acts on. This
    /// is the case when kick runs outside of any project.
    pub(crate) provides_repos: bool,
}

impl Global {
    /// The path of the global configuration as it is shown to the user, with
    /// the home directory abbreviated to `~`.
    pub(crate) fn display_path(&self) -> String {
        display(&self.home, &self.path)
    }

    /// Resolve a path-valued setting from the global configuration,
    /// interpolating `~` and environment variables in it.
    pub(crate) fn resolve(&self, value: &str) -> Result<PathBuf, interpolate::Error> {
        resolve_config(&self.home, &self.home, value, interpolate::env)
    }

    /// Resolve the `[repo."<key>"]` key of a repo declared in the global
    /// configuration to a path relative to `root`.
    pub(crate) fn repo_path(
        &self,
        root: &Path,
        key: &str,
    ) -> Result<RelativePathBuf, interpolate::Error> {
        Ok(relative_to(root, &self.resolve(key)?))
    }
}

/// Resolve a path-valued setting read from configuration.
///
/// `~` and environment variables are interpolated as described in
/// [`crate::interpolate`], after which an absolute path is used as-is and
/// anything else is resolved against `base`.
pub(crate) fn resolve_config(
    home: &Path,
    base: &Path,
    value: &str,
    lookup: impl Fn(&str) -> Option<std::ffi::OsString>,
) -> Result<PathBuf, interpolate::Error> {
    let path = interpolate::path(value, Some(home), lookup)?;

    let path = if path.is_absolute() {
        path
    } else {
        base.join(path)
    };

    Ok(normalize(&path))
}

/// Resolve a path given on the command line.
///
/// No environment variables are interpolated, since the shell has already
/// done so. A path starting with `~/` (or which is exactly `~`) is resolved against
/// `home`, an absolute path is used as-is, and anything else is resolved
/// against `base`.
pub(crate) fn resolve(home: &Path, base: &Path, value: &str) -> PathBuf {
    let path = if value == "~" {
        home.to_owned()
    } else if let Some(rest) = value.strip_prefix("~/") {
        home.join(rest)
    } else {
        let path = Path::new(value);

        if path.is_absolute() {
            path.to_owned()
        } else {
            base.join(path)
        }
    };

    normalize(&path)
}

/// Lexically normalize a path, removing `.` components and resolving `..`
/// components where possible.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();

    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if matches!(
                    out.components().next_back(),
                    Some(Component::Normal(..)) | None
                ) && out.pop()
                {
                    continue;
                }

                if !out.has_root() {
                    out.push(c);
                }
            }
            c => out.push(c),
        }
    }

    out
}

/// Express the absolute `path` relative to `root`, using `..` components if
/// it is not inside of `root`.
pub(crate) fn relative_to(root: &Path, path: &Path) -> RelativePathBuf {
    let root = normalize(root);
    let path = normalize(path);

    let mut root_c = root.components().peekable();
    let mut path_c = path.components().peekable();

    while let (Some(a), Some(b)) = (root_c.peek(), path_c.peek()) {
        if a != b {
            break;
        }

        root_c.next();
        path_c.next();
    }

    let mut out = RelativePathBuf::new();

    for _ in root_c {
        out.push("..");
    }

    for c in path_c {
        out.push(c.as_os_str().to_string_lossy().as_ref());
    }

    out
}

/// Display `path`, abbreviating the home directory to `~`.
pub(crate) fn display(home: &Path, path: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => String::from("~"),
        Ok(rest) => format!("~/{}", to_slashes(rest)),
        Err(..) => path.display().to_string(),
    }
}

/// The key a directory is registered under in the global configuration: a
/// path starting with `~/` if it is inside of the home directory, otherwise
/// the absolute path.
pub(crate) fn storage_key(home: &Path, path: &Path) -> String {
    display(&normalize(home), &normalize(path))
}

fn to_slashes(path: &Path) -> String {
    path.components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

/// The `[repo."<path>"]` keys in the global configuration at `path` which
/// declare a `url`, and so are repos kick can act on.
///
/// Errors reading or parsing the file are ignored here, they are reported when
/// the configuration is loaded.
pub(crate) fn declared_repos(path: &Path) -> Vec<String> {
    let Ok(string) = fs::read_to_string(path) else {
        return Vec::new();
    };

    let Ok(table) = toml::from_str::<toml::Table>(&string) else {
        return Vec::new();
    };

    let Some(repos) = table.get("repo").and_then(|r| r.as_table()) else {
        return Vec::new();
    };

    repos
        .iter()
        .filter(|(_, v)| v.get("url").is_some_and(|u| u.is_str()))
        .map(|(k, _)| k.clone())
        .collect()
}

/// A project registered in the global configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Project {
    /// The `[repo."<key>"]` key the project is registered under.
    pub(crate) key: String,
    /// The resolved path of the project, or the error raised interpolating
    /// its key.
    pub(crate) path: Result<PathBuf, interpolate::Error>,
    /// The url of the project, if any.
    pub(crate) url: Option<String>,
}

/// Edits the projects registered in a global configuration file, preserving
/// its formatting and comments.
pub(crate) struct Projects {
    path: PathBuf,
    home: PathBuf,
    doc: DocumentMut,
}

impl Projects {
    /// Open the global configuration at `path`. A missing file is treated as
    /// empty.
    pub(crate) fn open(path: &Path, home: &Path) -> Result<Self> {
        let doc = match fs::read_to_string(path) {
            Ok(string) => string
                .parse::<DocumentMut>()
                .with_context(|| format!("Parsing {}", path.display()))?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => DocumentMut::new(),
            Err(e) => return Err(e).with_context(|| format!("Reading {}", path.display())),
        };

        Ok(Self {
            path: path.to_owned(),
            home: home.to_owned(),
            doc,
        })
    }

    /// List registered projects in the order they are declared.
    pub(crate) fn list(&self) -> Vec<Project> {
        let Some(repos) = self.doc.get("repo").and_then(|r| r.as_table_like()) else {
            return Vec::new();
        };

        repos
            .iter()
            .map(|(key, item)| Project {
                key: key.to_owned(),
                path: resolve_config(&self.home, &self.home, key, interpolate::env),
                url: item
                    .as_table_like()
                    .and_then(|t| t.get("url"))
                    .and_then(|u| u.as_str())
                    .map(str::to_owned),
            })
            .collect()
    }

    /// Find a registered project by its key or its resolved path.
    fn find(&self, key: Option<&str>, path: &Path) -> Option<Project> {
        let path = normalize(path);

        self.list().into_iter().find(|p| {
            key.is_some_and(|k| k == p.key) || p.path.as_ref().is_ok_and(|p| same_path(p, &path))
        })
    }

    /// Register the directory at `path` with the given `url`, returning the
    /// key it was registered under.
    pub(crate) fn add(&mut self, path: &Path, url: &str) -> Result<String> {
        if let Some(existing) = self.find(None, path) {
            bail!(
                "{} is already registered as [repo.\"{}\"] in {}",
                path.display(),
                existing.key,
                self.path.display()
            );
        }

        let key = storage_key(&self.home, path);

        let repos = self
            .doc
            .entry("repo")
            .or_insert_with(|| {
                let mut table = Table::new();
                table.set_implicit(true);
                Item::Table(table)
            })
            .as_table_mut()
            .context("`repo` in the global configuration is not a table")?;

        let mut table = Table::new();
        table.insert("url", toml_edit::value(url));
        repos.insert(&key, Item::Table(table));
        Ok(key)
    }

    /// Remove the project registered under the key `raw` or at the resolved
    /// `path`, returning the removed project.
    pub(crate) fn remove(&mut self, raw: &str, path: &Path) -> Result<Project> {
        let Some(project) = self.find(Some(raw), path) else {
            bail!(
                "No project registered at {} in {}",
                path.display(),
                self.path.display()
            );
        };

        if let Some(repos) = self.doc.get_mut("repo").and_then(|r| r.as_table_like_mut()) {
            repos.remove(&project.key);
        }

        Ok(project)
    }

    /// Write the configuration back to disk, creating its directory if
    /// needed.
    pub(crate) fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| format!("Creating {}", parent.display()))?;
        }

        fs::write(&self.path, self.doc.to_string())
            .with_context(|| format!("Writing {}", self.path.display()))
    }
}

/// Test if two normalized paths refer to the same directory.
fn same_path(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }

    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::{Projects, declared_repos, display, normalize, relative_to, resolve};

    #[test]
    fn resolves_paths() {
        let home = Path::new("/home/user");
        let cwd = Path::new("/work/dir");

        assert_eq!(resolve(home, home, "~"), Path::new("/home/user"));
        assert_eq!(
            resolve(home, home, "~/projects/a"),
            Path::new("/home/user/projects/a")
        );
        assert_eq!(
            resolve(home, home, "projects/a"),
            Path::new("/home/user/projects/a")
        );
        assert_eq!(resolve(home, home, "/opt/a"), Path::new("/opt/a"));
        assert_eq!(resolve(home, cwd, "a/./b/../c"), Path::new("/work/dir/a/c"));
        assert_eq!(resolve(home, cwd, ".."), Path::new("/work"));
    }

    #[test]
    fn normalizes() {
        assert_eq!(normalize(Path::new("/a/../../b")), Path::new("/b"));
        assert_eq!(normalize(Path::new("a/../../b")), Path::new("../b"));
    }

    #[test]
    fn relative_paths() {
        let home = Path::new("/home/user");
        assert_eq!(
            relative_to(home, Path::new("/home/user/projects/a")),
            "projects/a"
        );
        assert_eq!(relative_to(home, Path::new("/home/user")), "");
        assert_eq!(relative_to(home, Path::new("/opt/a")), "../../opt/a");
        assert_eq!(relative_to(home, Path::new("/home/other")), "../other");
    }

    #[test]
    fn displays_paths() {
        let home = Path::new("/home/user");
        assert_eq!(display(home, Path::new("/home/user/a/b")), "~/a/b");
        assert_eq!(display(home, Path::new("/home/user")), "~");
        assert_eq!(display(home, Path::new("/opt/a")), "/opt/a");
    }

    #[test]
    fn add_list_remove_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let config = dir.path().join("config/kick/Kick.toml");
        let project = home.join("projects/a");
        fs::create_dir_all(&project).unwrap();

        let mut projects = Projects::open(&config, &home).unwrap();
        assert!(projects.list().is_empty());

        let key = projects
            .add(&project, "https://example.com/a")
            .expect("added");
        assert_eq!(key, "~/projects/a");

        let key = projects
            .add(Path::new("/opt/b"), "https://example.com/b")
            .unwrap();
        assert_eq!(key, "/opt/b");

        // Duplicates are refused, however they are spelled.
        assert!(
            projects
                .add(&home.join("projects/./a"), "https://example.com/a")
                .is_err()
        );

        projects.save().unwrap();

        let written = fs::read_to_string(&config).unwrap();
        assert!(written.contains("[repo.\"~/projects/a\"]"), "{written}");
        assert!(!written.contains("[repo]\n"), "{written}");

        let mut projects = Projects::open(&config, &home).unwrap();
        let list = projects.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].key, "~/projects/a");
        assert_eq!(list[0].path, Ok(project.clone()));
        assert_eq!(list[0].url.as_deref(), Some("https://example.com/a"));
        assert_eq!(list[1].path, Ok(Path::new("/opt/b").to_owned()));

        assert_eq!(
            declared_repos(&config),
            vec![String::from("~/projects/a"), String::from("/opt/b")]
        );

        // Remove by path.
        let removed = projects.remove("ignored", &project).unwrap();
        assert_eq!(removed.key, "~/projects/a");

        // Remove by key.
        let removed = projects.remove("/opt/b", Path::new("/nope")).unwrap();
        assert_eq!(removed.key, "/opt/b");

        assert!(projects.remove("/opt/b", Path::new("/opt/b")).is_err());
        projects.save().unwrap();
        assert!(Projects::open(&config, &home).unwrap().list().is_empty());
    }

    #[test]
    fn edits_preserve_comments() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let config = dir.path().join("Kick.toml");

        fs::write(
            &config,
            "# My settings\nlicense = \"MIT\" # keep this\n\n[repo.\"~/a\"]\n# the a project\nurl = \"https://example.com/a\"\n",
        )
        .unwrap();

        let mut projects = Projects::open(&config, &home).unwrap();
        projects
            .add(&home.join("b"), "https://example.com/b")
            .unwrap();
        projects.save().unwrap();

        let written = fs::read_to_string(&config).unwrap();
        assert!(written.starts_with("# My settings\nlicense = \"MIT\" # keep this\n"));
        assert!(written.contains("# the a project\n"));
        assert!(written.contains("[repo.\"~/b\"]\nurl = \"https://example.com/b\"\n"));
    }
}
