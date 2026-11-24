use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use chrono::{DateTime, Local, NaiveDateTime};
use relative_path::{RelativePath, RelativePathBuf};

use crate::model::Repo;

/// Date format for temporary set snapshots.
const DATE_FORMAT: &str = "%y%m%d%H%M%S";
/// Number of rolling timestamped snapshots retained per temporary set.
const PRUNE: usize = 3;

/// Distinguishes durable persistent sets from ephemeral temporary run sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum SetKind {
    Persistent,
    Temporary,
}

impl fmt::Display for SetKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Persistent => f.write_str("persistent"),
            Self::Temporary => f.write_str("temporary"),
        }
    }
}

/// Summary of an available set for listing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SetSummary {
    pub(crate) name: String,
    pub(crate) kind: SetKind,
    pub(crate) count: usize,
    pub(crate) snapshots: usize,
    pub(crate) latest: Option<NaiveDateTime>,
    pub(crate) shadowed: bool,
}

/// Validate a set name for safety and syntax.
pub(crate) fn validate_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("Set name cannot be empty");
    }

    if name.starts_with('@') {
        bail!("Set name `{name}` cannot start with `@`");
    }

    if name == "."
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        bail!("Invalid set name `{name}`");
    }

    Ok(())
}

/// Check whether a set name is reserved for temporary runs.
pub(crate) fn is_reserved_name(name: &str) -> bool {
    matches!(name, "good" | "bad")
}

/// Validate a name for a persistent set.
pub(crate) fn validate_persistent_name(name: &str) -> Result<()> {
    validate_name(name)?;

    if is_reserved_name(name) {
        bail!("Set name `{name}` is reserved for temporary runs");
    }

    Ok(())
}

/// Collection of persistent and temporary repo sets.
#[derive(Debug, Default)]
pub(crate) struct RepoSets {
    persistent_dir: PathBuf,
    temporary_dir: PathBuf,
    persistent: BTreeMap<String, PathBuf>,
    temporary: BTreeMap<String, Known>,
    updates: Vec<(String, RepoSet, String)>,
}

impl RepoSets {
    /// Locate the temporary sets directory for a project or global repos,
    /// mirroring [`crate::changes::ChangesFile::new`].
    pub(crate) fn temporary_dir(
        state: Option<&Path>,
        root: &Path,
        global: bool,
        persistent_dir: &Path,
    ) -> PathBuf {
        let Some(state) = state else {
            return persistent_dir.join("tmp");
        };

        if global {
            state.join("sets").join("global")
        } else {
            let root = root.canonicalize().unwrap_or_else(|_| root.to_owned());
            state
                .join("sets")
                .join("projects")
                .join(crate::changes::escape_path(&root))
        }
    }

    /// Load persistent and temporary sets from the given directories.
    #[tracing::instrument(level = "trace", ret, skip_all)]
    pub(crate) fn new<P, T>(persistent_dir: P, temporary_dir: T) -> Result<Self>
    where
        P: AsRef<Path>,
        T: AsRef<Path>,
    {
        let persistent_dir = persistent_dir.as_ref();
        let temporary_dir = temporary_dir.as_ref();

        let mut sets = Self {
            persistent_dir: persistent_dir.into(),
            temporary_dir: temporary_dir.into(),
            persistent: BTreeMap::default(),
            temporary: BTreeMap::default(),
            updates: Vec::default(),
        };

        sets.scan_persistent()?;
        sets.scan_temporary()?;
        Ok(sets)
    }

    fn scan_persistent(&mut self) -> Result<()> {
        let dir = match fs::read_dir(&self.persistent_dir) {
            Ok(dir) => dir,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e).context(self.persistent_dir.display().to_string()),
        };

        for e in dir {
            let e = e.with_context(|| anyhow!("{}", self.persistent_dir.display()))?;
            let path = e.path();

            if !e.file_type().is_ok_and(|ft| ft.is_file()) {
                continue;
            }

            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };

            if name.starts_with('.') {
                continue;
            }

            self.persistent.insert(name.to_owned(), path);
        }

        Ok(())
    }

    fn scan_temporary(&mut self) -> Result<()> {
        let dir = match fs::read_dir(&self.temporary_dir) {
            Ok(dir) => dir,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e).context(self.temporary_dir.display().to_string()),
        };

        for e in dir {
            let e = e.with_context(|| anyhow!("{}", self.temporary_dir.display()))?;
            let path = e.path();

            if !e.file_type().is_ok_and(|ft| ft.is_file()) {
                continue;
            }

            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };

            if name.starts_with('.') {
                continue;
            }

            let (id, date, path) = 'out: {
                if let Some((id, tail)) = name.rsplit_once('-')
                    && let Ok(date) = NaiveDateTime::parse_from_str(tail, DATE_FORMAT)
                {
                    let base = path.with_file_name(id);
                    break 'out (id, Some(date), base);
                }

                (name, None, path.clone())
            };

            let known = self
                .temporary
                .entry(id.to_owned())
                .or_insert_with(|| Known::new(path.clone()));

            known.base |= date.is_none();
            known.dates.extend(date);
        }

        Ok(())
    }

    /// List all available persistent and temporary sets.
    pub(crate) fn list(&self) -> Result<Vec<SetSummary>> {
        let mut summaries = Vec::new();

        for (name, path) in &self.persistent {
            let count = load_file(path)?.map_or(0, |s| s.len());
            summaries.push(SetSummary {
                name: name.clone(),
                kind: SetKind::Persistent,
                count,
                snapshots: 0,
                latest: None,
                shadowed: false,
            });
        }

        for (name, known) in &self.temporary {
            let path = temporary_read_path(name, known)?;
            let count = load_file(&path)?.map_or(0, |s| s.len());
            summaries.push(SetSummary {
                name: name.clone(),
                kind: SetKind::Temporary,
                count,
                snapshots: known.dates.len(),
                latest: known.dates.last().copied(),
                shadowed: self.persistent.contains_key(name),
            });
        }

        summaries.sort_by(|a, b| a.name.cmp(&b.name).then(a.kind.cmp(&b.kind)));
        Ok(summaries)
    }

    /// Load a set by name, preferring persistent sets over temporary sets.
    pub(crate) fn load(&self, id: &str) -> Result<Option<RepoSet>> {
        Ok(self.load_with_kind(id)?.map(|(_, set)| set))
    }

    /// Load a set by name along with its kind, preferring persistent sets over
    /// temporary sets.
    pub(crate) fn load_with_kind(&self, id: &str) -> Result<Option<(SetKind, RepoSet)>> {
        if let Some(path) = self.persistent.get(id)
            && let Some(set) = load_file(path)?
        {
            return Ok(Some((SetKind::Persistent, set)));
        }

        if let Some(known) = self.temporary.get(id) {
            let path = temporary_read_path(id, known)?;
            if let Some(set) = load_file(&path)? {
                return Ok(Some((SetKind::Temporary, set)));
            }
        }

        Ok(None)
    }

    /// Create a persistent set from the given repo paths.
    pub(crate) fn create_persistent(
        &mut self,
        name: &str,
        repos: &[RelativePathBuf],
        force: bool,
    ) -> Result<PathBuf> {
        validate_persistent_name(name)?;

        let path = self.persistent_dir.join(name);

        if !force && (self.persistent.contains_key(name) || path.exists()) {
            bail!("Persistent set `{name}` already exists (pass `--force` to overwrite)");
        }

        let mut seen = HashSet::new();
        let mut lines = Vec::new();

        for repo in repos {
            if seen.insert(repo.as_relative_path()) {
                lines.push(repo.as_str().to_owned());
            }
        }

        write_lines(&self.persistent_dir, &path, &lines)?;
        self.persistent.insert(name.to_owned(), path.clone());
        Ok(path)
    }

    /// Add repos to an existing persistent set (or create it when `create` is
    /// set), preserving existing comments, blank lines, and ordering.
    pub(crate) fn add_persistent(
        &mut self,
        name: &str,
        repos: &[RelativePathBuf],
        create: bool,
    ) -> Result<(usize, PathBuf)> {
        validate_persistent_name(name)?;

        let Some(path) = self.persistent.get(name).cloned() else {
            if create {
                let mut seen = HashSet::new();
                let count = repos
                    .iter()
                    .filter(|r| seen.insert(r.as_relative_path()))
                    .count();
                let path = self.create_persistent(name, repos, false)?;
                return Ok((count, path));
            }

            bail!(
                "Persistent set `{name}` does not exist (pass `--create` or use `kick set create {name}`)"
            );
        };

        let existing = load_file(&path)?.unwrap_or_default();
        let mut lines = existing.raw;
        let mut present: HashSet<RelativePathBuf> = existing.order.into_iter().collect();
        let mut added = 0;

        for repo in repos {
            if present.insert(repo.clone()) {
                lines.push(repo.as_str().to_owned());
                added += 1;
            }
        }

        write_lines(&self.persistent_dir, &path, &lines)?;
        Ok((added, path))
    }

    /// Remove repos from an existing persistent set, preserving comments,
    /// blank lines, and the ordering of remaining entries.
    pub(crate) fn remove_persistent(
        &mut self,
        name: &str,
        repos: &[RelativePathBuf],
    ) -> Result<(usize, PathBuf)> {
        validate_persistent_name(name)?;

        let Some(path) = self.persistent.get(name).cloned() else {
            bail!("Persistent set `{name}` does not exist");
        };

        let existing = load_file(&path)?.unwrap_or_default();
        let to_remove: HashSet<&RelativePath> =
            repos.iter().map(|r| r.as_relative_path()).collect();
        let mut lines = Vec::with_capacity(existing.raw.len());
        let mut removed = 0;

        for line in existing.raw {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                lines.push(line);
                continue;
            }

            if to_remove.contains(RelativePath::new(trimmed)) {
                removed += 1;
            } else {
                lines.push(line);
            }
        }

        write_lines(&self.persistent_dir, &path, &lines)?;
        Ok((removed, path))
    }

    /// Delete a persistent or temporary set. If both exist with the same name,
    /// the persistent set is deleted (matching lookup precedence).
    pub(crate) fn delete(&mut self, name: &str) -> Result<SetKind> {
        validate_name(name)?;

        if let Some(path) = self.persistent.remove(name) {
            remove_if_exists(&path)?;
            return Ok(SetKind::Persistent);
        }

        if let Some(known) = self.temporary.remove(name) {
            remove_if_exists(&known.path)?;

            for date in &known.dates {
                let snapshot = self
                    .temporary_dir
                    .join(format!("{name}-{}", date.format(DATE_FORMAT)));
                remove_if_exists(&snapshot)?;
            }

            return Ok(SetKind::Temporary);
        }

        bail!("Set `{name}` does not exist");
    }

    /// Rename a persistent set.
    pub(crate) fn rename_persistent(&mut self, old: &str, new: &str) -> Result<PathBuf> {
        validate_persistent_name(old)?;
        validate_persistent_name(new)?;

        let Some(old_path) = self.persistent.get(old).cloned() else {
            if self.temporary.contains_key(old) {
                bail!(
                    "Cannot rename temporary set `{old}`; use `kick set create {new} --set {old}` to save it as a persistent set"
                );
            }

            bail!("Persistent set `{old}` does not exist");
        };

        let new_path = self.persistent_dir.join(new);

        if self.persistent.contains_key(new) || new_path.exists() {
            bail!("Persistent set `{new}` already exists");
        }

        fs::rename(&old_path, &new_path)
            .with_context(|| anyhow!("{} -> {}", old_path.display(), new_path.display()))?;
        self.persistent.remove(old);
        self.persistent.insert(new.to_owned(), new_path.clone());
        Ok(new_path)
    }

    /// Return the path to a persistent set file for editing, creating the
    /// persistent sets directory if needed.
    pub(crate) fn persistent_path_for_edit(&mut self, name: &str) -> Result<PathBuf> {
        validate_persistent_name(name)?;

        if !self.persistent_dir.is_dir() {
            fs::create_dir_all(&self.persistent_dir)
                .with_context(|| anyhow!("{}", self.persistent_dir.display()))?;
        }

        Ok(self.persistent_dir.join(name))
    }

    /// Queue a temporary set update to be written on [`Self::commit`].
    pub(crate) fn save_temporary<D>(&mut self, id: &str, set: RepoSet, hint: &D)
    where
        D: ?Sized + fmt::Display,
    {
        self.updates.push((id.into(), set, hint.to_string()));
    }

    /// Commit queued temporary set updates with timestamped snapshots and pruning.
    pub(crate) fn commit(&mut self) -> Result<()> {
        fn write_set(set: &RepoSet, hint: &str, now: DateTime<Local>, mut f: File) -> Result<()> {
            writeln!(f, "# {hint}")?;
            writeln!(f, "# date: {now}")?;

            for line in &set.raw {
                writeln!(f, "{line}")?;
            }

            for repo in &set.added {
                writeln!(f, "{repo}")?;
            }

            f.flush()?;
            Ok(())
        }

        let now = Local::now();
        let timestamp = now.naive_local().format(DATE_FORMAT).to_string();

        for (id, set, hint) in self.updates.drain(..) {
            tracing::trace!(?id, ?timestamp, "Saving temporary set");

            let base_path = self.temporary_dir.join(&id);

            let paths = [
                base_path.clone(),
                self.temporary_dir.join(format!("{id}-{timestamp}")),
            ];

            let known = self
                .temporary
                .entry(id.clone())
                .or_insert_with(|| Known::new(base_path));
            known.base = true;
            known.dates.insert(now.naive_local());

            for path in paths {
                if !self.temporary_dir.is_dir() {
                    fs::create_dir_all(&self.temporary_dir)
                        .with_context(|| anyhow!("{}", self.temporary_dir.display()))?;
                }

                let f = match File::create(&path) {
                    Ok(file) => file,
                    Err(e) => return Err(e).context(anyhow!("{}", path.display())),
                };

                write_set(&set, &hint, now, f).context(anyhow!("{}", path.display()))?;
            }
        }

        // Prune old temporary snapshots.
        for (id, known) in &mut self.temporary {
            while known.dates.len() > PRUNE {
                let Some(date) = known.dates.pop_first() else {
                    continue;
                };

                let path = self
                    .temporary_dir
                    .join(format!("{id}-{}", date.format(DATE_FORMAT)));
                tracing::trace!(
                    path = path.display().to_string(),
                    "Removing old temporary set"
                );
                let _ = fs::remove_file(&path);
            }
        }

        Ok(())
    }
}

fn temporary_read_path(id: &str, known: &Known) -> Result<PathBuf> {
    let mut path = known.path.clone();

    if !known.base {
        let latest = known
            .dates
            .last()
            .with_context(|| anyhow!("{id}: missing latest set"))?;

        path.set_file_name(format!("{id}-{}", latest.format(DATE_FORMAT)));
    }

    Ok(path)
}

fn load_file(path: &Path) -> Result<Option<RepoSet>> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context(path.display().to_string()),
    };

    let mut set = RepoSet::default();
    let reader = BufReader::new(file);

    for (n, line) in reader.lines().enumerate() {
        let line = line.with_context(|| path.display().to_string())?;
        set.raw.push(line.clone());

        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let repo = RelativePathBuf::from(trimmed);
        if set.repos.insert(repo.clone(), n).is_none() {
            set.order.push(repo);
        }
    }

    Ok(Some(set))
}

fn write_lines(dir: &Path, path: &Path, lines: &[String]) -> Result<()> {
    if !dir.is_dir() {
        fs::create_dir_all(dir).with_context(|| anyhow!("{}", dir.display()))?;
    }

    let mut f = File::create(path).with_context(|| anyhow!("{}", path.display()))?;

    for line in lines {
        writeln!(f, "{line}").with_context(|| anyhow!("{}", path.display()))?;
    }

    f.flush().with_context(|| anyhow!("{}", path.display()))?;
    Ok(())
}

fn remove_if_exists(path: &Path) -> Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e).context(path.display().to_string()),
    }
}

#[derive(Debug)]
struct Known {
    /// Indicates if there is a base set.
    base: bool,
    /// Base path of the set.
    path: PathBuf,
    /// Known dates.
    dates: BTreeSet<NaiveDateTime>,
}

impl Known {
    fn new(path: PathBuf) -> Self {
        Self {
            base: false,
            path,
            dates: BTreeSet::default(),
        }
    }
}

/// A set of loaded repos.
#[derive(Default, Debug, Clone)]
pub(crate) struct RepoSet {
    raw: Vec<String>,
    repos: HashMap<RelativePathBuf, usize>,
    order: Vec<RelativePathBuf>,
    added: BTreeSet<RelativePathBuf>,
}

impl RepoSet {
    /// Add the given repo to the set.
    pub(crate) fn insert(&mut self, repo: &Repo) {
        self.insert_path(repo.path().to_owned());
    }

    /// Add the given repo path to the set.
    pub(crate) fn insert_path(&mut self, path: RelativePathBuf) {
        if !self.repos.contains_key(&path) {
            self.added.insert(path);
        }
    }

    /// Number of unique repos in the set.
    pub(crate) fn len(&self) -> usize {
        self.order.len() + self.added.len()
    }

    /// Iterate over all repos in the set in deterministic order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &RelativePath> {
        self.order
            .iter()
            .chain(self.added.iter())
            .map(|p| p.as_relative_path())
    }

    /// Iterate over all repos in the set.
    pub(crate) fn into_iter(self) -> impl Iterator<Item = RelativePathBuf> {
        self.order.into_iter().chain(self.added)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use relative_path::RelativePathBuf;

    use super::{
        RepoSet, RepoSets, SetKind, is_reserved_name, validate_name, validate_persistent_name,
    };

    #[test]
    fn name_validation_and_reserved() {
        assert!(validate_name("core").is_ok());
        assert!(validate_name("my_set_1").is_ok());
        assert!(validate_name("good").is_ok());
        assert!(validate_name("bad").is_ok());

        assert!(validate_name("").is_err());
        assert!(validate_name("@all").is_err());
        assert!(validate_name("a/b").is_err());
        assert!(validate_name("a\\b").is_err());
        assert!(validate_name("..").is_err());
        assert!(validate_name("a..b").is_err());
        assert!(validate_name(".").is_err());

        assert!(is_reserved_name("good"));
        assert!(is_reserved_name("bad"));
        assert!(!is_reserved_name("core"));

        assert!(validate_persistent_name("good").is_err());
        assert!(validate_persistent_name("bad").is_err());
        assert!(validate_persistent_name("core").is_ok());
    }

    #[test]
    fn persistent_and_temporary_storage_separation_and_precedence() {
        let dir = tempfile::tempdir().unwrap();
        let persistent_dir = dir.path().join("persistent");
        let temporary_dir = dir.path().join("temporary");

        let mut sets = RepoSets::new(&persistent_dir, &temporary_dir).unwrap();

        let mut tmp_set = RepoSet::default();
        tmp_set.insert_path(RelativePathBuf::from("repos/tmp_only"));
        sets.save_temporary("bad", tmp_set.clone(), "run failed");
        sets.save_temporary("shared", tmp_set, "remaining");
        sets.commit().unwrap();

        // Temporary sets land in temporary_dir with snapshots; persistent_dir stays clean.
        assert!(!persistent_dir.exists());
        assert!(temporary_dir.join("bad").is_file());
        assert!(temporary_dir.join("shared").is_file());
        let bad_snapshots: Vec<_> = fs::read_dir(&temporary_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().starts_with("bad-"))
            .collect();
        assert_eq!(bad_snapshots.len(), 1);

        // Creating persistent set writes a plain file without snapshots.
        sets.create_persistent(
            "shared",
            &[RelativePathBuf::from("repos/persistent_only")],
            false,
        )
        .unwrap();
        assert!(persistent_dir.join("shared").is_file());
        let p_entries: Vec<_> = fs::read_dir(&persistent_dir).unwrap().collect();
        assert_eq!(p_entries.len(), 1);
        assert_eq!(
            fs::read_to_string(persistent_dir.join("shared")).unwrap(),
            "repos/persistent_only\n"
        );

        // Reload and verify lookup precedence: persistent wins over temporary.
        let sets = RepoSets::new(&persistent_dir, &temporary_dir).unwrap();
        let (kind, loaded) = sets.load_with_kind("shared").unwrap().unwrap();
        assert_eq!(kind, SetKind::Persistent);
        let repos: Vec<_> = loaded.into_iter().collect();
        assert_eq!(repos, vec![RelativePathBuf::from("repos/persistent_only")]);

        let (kind, loaded) = sets.load_with_kind("bad").unwrap().unwrap();
        assert_eq!(kind, SetKind::Temporary);
        let repos: Vec<_> = loaded.into_iter().collect();
        assert_eq!(repos, vec![RelativePathBuf::from("repos/tmp_only")]);

        let list = sets.list().unwrap();
        assert_eq!(list.len(), 3);
        assert!(
            list.iter()
                .any(|s| s.name == "shared" && s.kind == SetKind::Persistent && !s.shadowed)
        );
        assert!(
            list.iter()
                .any(|s| s.name == "shared" && s.kind == SetKind::Temporary && s.shadowed)
        );
        assert!(
            list.iter()
                .any(|s| s.name == "bad" && s.kind == SetKind::Temporary && !s.shadowed)
        );
    }

    #[test]
    fn comment_and_blank_line_preserving_add_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let persistent_dir = dir.path().join("persistent");
        let temporary_dir = dir.path().join("temporary");

        fs::create_dir_all(&persistent_dir).unwrap();
        fs::write(
            persistent_dir.join("core"),
            "# Core repositories\n\nrepos/alpha\n# keep this note\nrepos/beta\n",
        )
        .unwrap();

        let mut sets = RepoSets::new(&persistent_dir, &temporary_dir).unwrap();

        // Load skips comments and blank lines.
        let loaded = sets.load("core").unwrap().unwrap();
        let repos: Vec<_> = loaded.into_iter().collect();
        assert_eq!(
            repos,
            vec![
                RelativePathBuf::from("repos/alpha"),
                RelativePathBuf::from("repos/beta"),
            ]
        );

        // Add skips existing and appends new while preserving comments/blank lines.
        let (added, _) = sets
            .add_persistent(
                "core",
                &[
                    RelativePathBuf::from("repos/beta"),
                    RelativePathBuf::from("repos/gamma"),
                ],
                false,
            )
            .unwrap();
        assert_eq!(added, 1);
        assert_eq!(
            fs::read_to_string(persistent_dir.join("core")).unwrap(),
            "# Core repositories\n\nrepos/alpha\n# keep this note\nrepos/beta\nrepos/gamma\n"
        );

        // Remove drops matching entries and preserves comments/blank lines/order.
        let (removed, _) = sets
            .remove_persistent("core", &[RelativePathBuf::from("repos/alpha")])
            .unwrap();
        assert_eq!(removed, 1);
        assert_eq!(
            fs::read_to_string(persistent_dir.join("core")).unwrap(),
            "# Core repositories\n\n# keep this note\nrepos/beta\nrepos/gamma\n"
        );
    }

    #[test]
    fn delete_and_rename() {
        let dir = tempfile::tempdir().unwrap();
        let persistent_dir = dir.path().join("persistent");
        let temporary_dir = dir.path().join("temporary");

        let mut sets = RepoSets::new(&persistent_dir, &temporary_dir).unwrap();
        sets.create_persistent("alpha", &[RelativePathBuf::from("repos/a")], false)
            .unwrap();

        // Duplicate create fails without force, succeeds with force.
        assert!(sets.create_persistent("alpha", &[], false).is_err());
        sets.create_persistent("alpha", &[RelativePathBuf::from("repos/b")], true)
            .unwrap();

        // Rename to reserved name fails.
        assert!(sets.rename_persistent("alpha", "bad").is_err());

        // Rename to valid name succeeds.
        sets.rename_persistent("alpha", "beta").unwrap();
        assert!(!persistent_dir.join("alpha").exists());
        assert!(persistent_dir.join("beta").is_file());

        // Delete persistent set.
        assert_eq!(sets.delete("beta").unwrap(), SetKind::Persistent);
        assert!(!persistent_dir.join("beta").exists());

        // Save and delete temporary set (removes base + snapshots).
        let mut tmp = RepoSet::default();
        tmp.insert_path(RelativePathBuf::from("repos/a"));
        sets.save_temporary("bad", tmp, "failed");
        sets.commit().unwrap();
        assert_eq!(sets.delete("bad").unwrap(), SetKind::Temporary);
        let remaining: Vec<_> = fs::read_dir(&temporary_dir).unwrap().collect();
        assert!(remaining.is_empty());
    }
}
