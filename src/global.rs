//! The user-global kick configuration.
//!
//! This is a `Kick.toml` stored in kick's user configuration directory, such as
//! `~/.config/kick/Kick.toml` on Linux. It has the same schema as a project
//! `Kick.toml` and is loaded after every project `Kick.toml` as the least
//! specific layer.
//!
//! Every `*.toml` file under the `user.d/` directory next to it, in nested
//! sub-directories too, is loaded after it as a further global layer, sorted
//! by its path relative to `user.d/`. Later files are more specific.
//!
//! Paths in them, including the `[repo."<path>"]` keys, may be absolute, start
//! with `~/`, or be relative in which case they are resolved against the
//! directory holding the file which sets them, just like relative paths in a
//! project `Kick.toml`. Environment variables are interpolated in them as
//! described in [`crate::interpolate`].

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context, Result, bail};
use relative_path::RelativePathBuf;
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::interpolate;

/// The directory next to the global `Kick.toml` holding further global
/// configuration files.
pub(crate) const USER_D: &str = "user.d";

/// The user-global configuration.
#[derive(Debug, Clone)]
pub(crate) struct Global {
    /// The path of the global `Kick.toml`, or of the single file this layer
    /// is loaded from.
    pub(crate) path: PathBuf,
    /// The home directory, which a leading `~` expands to.
    pub(crate) home: PathBuf,
    /// Whether the global configuration provides the repos kick acts on. This
    /// is the case when kick runs outside of any project.
    pub(crate) provides_repos: bool,
    /// The `*.toml` files under `user.d/`, in load order.
    pub(crate) user_d: Vec<PathBuf>,
}

impl Global {
    /// The global configuration at `path`, discovering the files under the
    /// `user.d/` directory next to it.
    pub(crate) fn new(path: &Path, home: &Path, provides_repos: bool) -> Self {
        let user_d = user_d_files(&config_dir(path, home).join(USER_D));

        Self {
            path: path.to_owned(),
            home: home.to_owned(),
            provides_repos,
            user_d,
        }
    }

    /// Every global configuration file in load order, from least to most
    /// specific: the global `Kick.toml` followed by the files under `user.d/`.
    pub(crate) fn files(&self) -> impl Iterator<Item = &Path> {
        [self.path.as_path()]
            .into_iter()
            .chain(self.user_d.iter().map(PathBuf::as_path))
    }

    /// The layer of the global configuration loaded from the file at `path`,
    /// which relative paths in it are resolved against.
    pub(crate) fn layer(&self, path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            home: self.home.clone(),
            provides_repos: self.provides_repos,
            user_d: Vec::new(),
        }
    }

    /// Every layer of the global configuration in load order.
    pub(crate) fn layers(&self) -> impl Iterator<Item = Self> {
        self.files().map(|path| self.layer(path))
    }

    /// Test if `path` is, or would be, a global configuration file: the global
    /// `Kick.toml`, or a `*.toml` file under `user.d/` which is not hidden.
    pub(crate) fn loads(&self, path: &Path) -> bool {
        let path = normalize(path);

        if path == normalize(&self.path) || self.files().any(|f| same_path(f, &path)) {
            return true;
        }

        let user_d = normalize(&self.dir().join(USER_D));

        let Ok(rest) = path.strip_prefix(&user_d) else {
            return false;
        };

        path.extension().is_some_and(|e| e == "toml")
            && rest.components().all(|c| match c {
                Component::Normal(name) => !name.to_string_lossy().starts_with('.'),
                _ => false,
            })
    }

    /// Test if any global configuration file declares a repo kick can act on.
    pub(crate) fn declares_repos(&self) -> bool {
        self.files().any(|path| !declared_repos(path).is_empty())
    }

    /// The resolved directories of the repos declared in every global
    /// configuration file which declare a `url`.
    pub(crate) fn repo_dirs(&self) -> Vec<PathBuf> {
        self.layers()
            .flat_map(|layer| {
                declared_repos(&layer.path)
                    .into_iter()
                    .filter_map(move |key| layer.resolve(&key).ok())
            })
            .collect()
    }

    /// The path of the global configuration as it is shown to the user, with
    /// the home directory abbreviated to `~`.
    pub(crate) fn display_path(&self) -> String {
        display(&self.home, &self.path)
    }

    /// The directory holding the global `Kick.toml`, which relative paths in
    /// it are resolved against.
    pub(crate) fn dir(&self) -> &Path {
        config_dir(&self.path, &self.home)
    }

    /// Resolve a path-valued setting from the global configuration,
    /// interpolating `~` and environment variables in it. A relative path is
    /// resolved against the directory holding the global `Kick.toml`.
    pub(crate) fn resolve(&self, value: &str) -> Result<PathBuf, interpolate::Error> {
        resolve_config(&self.home, self.dir(), value, interpolate::env)
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

/// The directory holding the configuration file at `path`, falling back to
/// `home` if it has none.
fn config_dir<'a>(path: &'a Path, home: &'a Path) -> &'a Path {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => home,
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
///
/// If only one of them is absolute, the other is resolved against the current
/// directory first, since the root of a project is typically relative to it
/// (such as `.`).
pub(crate) fn relative_to(root: &Path, path: &Path) -> RelativePathBuf {
    let (root, path) = if root.is_absolute() != path.is_absolute() {
        (absolute(root), absolute(path))
    } else {
        (normalize(root), normalize(path))
    };

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

/// Make `path` absolute against the current directory and normalize it.
fn absolute(path: &Path) -> PathBuf {
    normalize(&std::path::absolute(path).unwrap_or_else(|_| path.to_owned()))
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

/// Find the `*.toml` files under the `user.d` directory `dir`, recursively,
/// sorted by their path relative to it.
///
/// Hidden files and directories (such as `.git`) are skipped. Symbolic links
/// are followed, but a directory is only visited once so link loops end. A
/// missing directory has no files, and errors reading one are logged and
/// otherwise ignored.
pub(crate) fn user_d_files(dir: &Path) -> Vec<PathBuf> {
    fn visit(
        root: &Path,
        dir: &Path,
        seen: &mut HashSet<PathBuf>,
        out: &mut Vec<(PathBuf, PathBuf)>,
    ) {
        let Ok(canonical) = dir.canonicalize() else {
            return;
        };

        if !seen.insert(canonical) {
            return;
        }

        let entries = match fs::read_dir(dir) {
            Ok(entries) => entries,
            Err(e) => {
                if e.kind() != io::ErrorKind::NotFound {
                    tracing::warn!("Reading {}: {e}", dir.display());
                }

                return;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry,
                Err(e) => {
                    tracing::warn!("Reading {}: {e}", dir.display());
                    continue;
                }
            };

            if entry.file_name().to_string_lossy().starts_with('.') {
                continue;
            }

            let path = entry.path();

            // NB: Follows symbolic links.
            let Ok(metadata) = fs::metadata(&path) else {
                continue;
            };

            if metadata.is_dir() {
                visit(root, &path, seen, out);
            } else if metadata.is_file() && path.extension().is_some_and(|e| e == "toml") {
                let relative = path.strip_prefix(root).unwrap_or(&path).to_owned();
                out.push((relative, path));
            }
        }
    }

    let mut out = Vec::new();
    visit(dir, dir, &mut HashSet::new(), &mut out);
    out.sort();
    out.into_iter().map(|(_, path)| path).collect()
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

    /// List registered projects in the order they are declared. Relative keys
    /// are resolved against the directory holding the configuration file.
    pub(crate) fn list(&self) -> Vec<Project> {
        let Some(repos) = self.doc.get("repo").and_then(|r| r.as_table_like()) else {
            return Vec::new();
        };

        repos
            .iter()
            .map(|(key, item)| Project {
                key: key.to_owned(),
                path: resolve_config(
                    &self.home,
                    config_dir(&self.path, &self.home),
                    key,
                    interpolate::env,
                ),
                url: item
                    .as_table_like()
                    .and_then(|t| t.get("url"))
                    .and_then(|u| u.as_str())
                    .map(str::to_owned),
            })
            .collect()
    }

    /// The path of the configuration file.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// Find a registered project by its key or its resolved path.
    pub(crate) fn find(&self, key: Option<&str>, path: &Path) -> Option<Project> {
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

    /// Find the project registered under the key `raw` or at the resolved
    /// `path`, failing with an error which points at `kick project add` if
    /// there is none.
    pub(crate) fn get(&self, raw: Option<&str>, path: &Path) -> Result<Project> {
        match self.find(raw, path) {
            Some(project) => Ok(project),
            None => bail!(
                "No project registered at {} in {}, register it with `kick project add`",
                path.display(),
                self.path.display()
            ),
        }
    }

    /// The settings of the project registered under `key`, which is the body
    /// of its `[repo."<key>"]` table, as TOML.
    pub(crate) fn settings(&self, key: &str) -> Result<String> {
        let table = match self.repo_item(key)? {
            Item::Table(table) => table.clone(),
            Item::Value(Value::InlineTable(table)) => table.clone().into_table(),
            _ => bail!("[repo.\"{key}\"] in {} is not a table", self.path.display()),
        };

        let mut doc = DocumentMut::new();
        let root = doc.as_table_mut();
        *root = table;
        root.decor_mut().clear();
        root.set_implicit(false);
        root.set_position(Some(0));
        Ok(doc.to_string())
    }

    /// Replace the settings of the project registered under `key` with
    /// `settings`, which is the body of its `[repo."<key>"]` table as TOML.
    ///
    /// The table keeps its place and the comments in front of its header,
    /// and the rest of the configuration is left untouched.
    pub(crate) fn set_settings(&mut self, key: &str, settings: &str) -> Result<()> {
        let mut doc = settings.parse::<DocumentMut>()?;

        let path = self.path.clone();
        let item = self.repo_item_mut(key)?;

        let mut table = std::mem::take(doc.as_table_mut());
        clear_positions(&mut table);

        match item {
            Item::Table(old) => {
                *table.decor_mut() = old.decor().clone();
                table.set_implicit(false);
                table.set_position(old.position());
                *old = table;
            }
            Item::Value(Value::InlineTable(old)) => {
                let mut table = table.into_inline_table();
                *table.decor_mut() = old.decor().clone();
                *old = table;
            }
            _ => bail!("[repo.\"{key}\"] in {} is not a table", path.display()),
        }

        Ok(())
    }

    fn repo_item(&self, key: &str) -> Result<&Item> {
        self.doc
            .get("repo")
            .and_then(|r| r.as_table_like())
            .and_then(|r| r.get(key))
            .with_context(|| {
                format!(
                    "No project registered as [repo.\"{key}\"] in {}",
                    self.path.display()
                )
            })
    }

    fn repo_item_mut(&mut self, key: &str) -> Result<&mut Item> {
        let path = &self.path;

        self.doc
            .get_mut("repo")
            .and_then(|r| r.as_table_like_mut())
            .and_then(|r| r.get_mut(key))
            .with_context(|| {
                format!(
                    "No project registered as [repo.\"{key}\"] in {}",
                    path.display()
                )
            })
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

/// Clear the positions of the tables nested under `table`, so that they are
/// written directly after it in the order they are declared.
fn clear_positions(table: &mut Table) {
    for (_, item) in table.iter_mut() {
        match item {
            Item::Table(table) => {
                table.set_position(None);
                clear_positions(table);
            }
            Item::ArrayOfTables(array) => {
                for table in array.iter_mut() {
                    table.set_position(None);
                    clear_positions(table);
                }
            }
            _ => {}
        }
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

    use super::{
        Global, Projects, declared_repos, display, normalize, relative_to, resolve, user_d_files,
    };

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

        // A relative root is resolved against the current directory.
        let current = std::env::current_dir().unwrap();
        assert_eq!(relative_to(Path::new("."), &current), "");
        assert_eq!(relative_to(Path::new("."), &current.join("a/b")), "a/b");
        assert_eq!(
            relative_to(Path::new(".."), &current),
            current.file_name().unwrap().to_str().unwrap()
        );
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
    fn relative_keys_resolve_against_config_dir() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let config = dir.path().join("config/kick/Kick.toml");
        fs::create_dir_all(config.parent().unwrap()).unwrap();

        fs::write(
            &config,
            "[repo.\"projects/a\"]\nurl = \"https://example.com/a\"\n\n[repo.\"~/b\"]\nurl = \"https://example.com/b\"\n",
        )
        .unwrap();

        let list = Projects::open(&config, &home).unwrap().list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].path, Ok(dir.path().join("config/kick/projects/a")));
        assert_eq!(list[1].path, Ok(home.join("b")));
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

    #[test]
    fn edits_project_settings() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let config = dir.path().join("Kick.toml");

        let before = concat!(
            "# My settings\n",
            "license = \"MIT\" # keep this\n",
            "\n",
            "# The a project\n",
            "[repo.\"~/a\"]\n",
            "# its url\n",
            "url = \"https://example.com/a\"\n",
            "branch = \"main\"\n",
            "\n",
            "[repo.\"~/a\".workflows.ci]\n",
            "name = \"CI\"\n",
            "\n",
            "# The b project\n",
            "[repo.\"~/b\"]\n",
            "url = \"https://example.com/b\" # b url\n",
        );

        fs::write(&config, before).unwrap();

        let mut projects = Projects::open(&config, &home).unwrap();
        let project = projects.get(None, &home.join("a")).unwrap();
        assert_eq!(project.key, "~/a");

        let settings = projects.settings("~/a").unwrap();
        assert_eq!(
            settings,
            concat!(
                "# its url\n",
                "url = \"https://example.com/a\"\n",
                "branch = \"main\"\n",
                "\n",
                "[workflows.ci]\n",
                "name = \"CI\"\n",
            )
        );

        // Writing back the settings unchanged leaves the file untouched.
        projects.set_settings("~/a", &settings).unwrap();
        projects.save().unwrap();
        assert_eq!(fs::read_to_string(&config).unwrap(), before);

        let edited = concat!(
            "# its new url\n",
            "url = \"https://example.com/a2\"\n",
            "\n",
            "[workflows.ci]\n",
            "name = \"CI\"\n",
            "\n",
            "[workflows.weekly]\n",
            "disable = true\n",
        );

        projects.set_settings("~/a", edited).unwrap();
        projects.save().unwrap();

        assert_eq!(
            fs::read_to_string(&config).unwrap(),
            concat!(
                "# My settings\n",
                "license = \"MIT\" # keep this\n",
                "\n",
                "# The a project\n",
                "[repo.\"~/a\"]\n",
                "# its new url\n",
                "url = \"https://example.com/a2\"\n",
                "\n",
                "[repo.\"~/a\".workflows.ci]\n",
                "name = \"CI\"\n",
                "\n",
                "[repo.\"~/a\".workflows.weekly]\n",
                "disable = true\n",
                "\n",
                "# The b project\n",
                "[repo.\"~/b\"]\n",
                "url = \"https://example.com/b\" # b url\n",
            )
        );

        let list = Projects::open(&config, &home).unwrap().list();
        assert_eq!(list[0].url.as_deref(), Some("https://example.com/a2"));
        assert_eq!(list[1].url.as_deref(), Some("https://example.com/b"));
    }

    #[test]
    fn edits_inline_project_settings() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let config = dir.path().join("Kick.toml");

        fs::write(
            &config,
            "repo = { \"~/a\" = { url = \"https://example.com/a\" } }\n",
        )
        .unwrap();

        let mut projects = Projects::open(&config, &home).unwrap();
        let settings = projects.settings("~/a").unwrap();
        assert_eq!(settings, "url = \"https://example.com/a\"\n");

        projects
            .set_settings("~/a", "url = \"https://example.com/a2\"\n")
            .unwrap();
        let list = projects.list();
        assert_eq!(list[0].url.as_deref(), Some("https://example.com/a2"));
    }

    #[test]
    fn rejects_invalid_project_settings() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let config = dir.path().join("Kick.toml");
        let before = "[repo.\"~/a\"]\nurl = \"https://example.com/a\"\n";
        fs::write(&config, before).unwrap();

        let mut projects = Projects::open(&config, &home).unwrap();
        assert!(projects.set_settings("~/a", "url = ").is_err());
        assert!(projects.set_settings("~/missing", "").is_err());
        projects.save().unwrap();
        assert_eq!(fs::read_to_string(&config).unwrap(), before);

        // Unregistered projects point at `kick project add`.
        let error = projects.get(None, &home.join("b")).unwrap_err();
        assert!(error.to_string().contains("kick project add"), "{error}");
        assert!(projects.get(Some("~/a"), Path::new("/nope")).is_ok());
    }

    #[test]
    fn finds_user_d_files() {
        let dir = tempfile::tempdir().unwrap();
        let user_d = dir.path().join("user.d");

        assert!(user_d_files(&user_d).is_empty());

        fs::create_dir_all(user_d.join("b/nested")).unwrap();
        fs::create_dir_all(user_d.join(".git")).unwrap();
        fs::write(user_d.join("z.toml"), "").unwrap();
        fs::write(user_d.join("b/nested/x.toml"), "").unwrap();
        fs::write(user_d.join("b/a.toml"), "").unwrap();
        fs::write(user_d.join("b/notes.md"), "").unwrap();
        fs::write(user_d.join(".hidden.toml"), "").unwrap();
        fs::write(user_d.join(".git/config.toml"), "").unwrap();

        #[cfg(unix)]
        std::os::unix::fs::symlink(&user_d, user_d.join("b/loop")).unwrap();

        assert_eq!(
            user_d_files(&user_d),
            [
                user_d.join("b/a.toml"),
                user_d.join("b/nested/x.toml"),
                user_d.join("z.toml"),
            ]
        );
    }

    #[test]
    fn loads_global_files() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let kick = dir.path().join("kick");
        let config = kick.join("Kick.toml");
        fs::create_dir_all(kick.join("user.d/private")).unwrap();
        fs::write(kick.join("user.d/private/a.toml"), "").unwrap();

        let global = Global::new(&config, &home, false);
        assert_eq!(
            global.files().map(Path::to_owned).collect::<Vec<_>>(),
            [config.clone(), kick.join("user.d/private/a.toml")]
        );

        assert!(global.loads(&config));
        assert!(global.loads(&kick.join("user.d/private/a.toml")));
        assert!(global.loads(&kick.join("user.d/new/b.toml")));
        assert!(!global.loads(&kick.join("user.d/.git/b.toml")));
        assert!(!global.loads(&kick.join("user.d/b.txt")));
        assert!(!global.loads(&kick.join("other.toml")));
        assert!(!global.loads(&kick.join("user.d/../other.toml")));
    }
}
