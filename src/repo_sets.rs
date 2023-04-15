use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Local, NaiveDateTime};
use relative_path::{RelativePath, RelativePathBuf};

use crate::model::Repo;

/// Date format for sets.
const DATE_FORMAT: &str = "%Y-%m-%d-%H%M%S";
/// Prune the three last sets.
const PRUNE: usize = 3;

/// Collection of known sets.
#[derive(Debug, Default)]
pub(crate) struct RepoSets {
    path: PathBuf,
    known: HashMap<String, Known>,
    updates: Vec<(String, RepoSet, bool, String)>,
}

impl RepoSets {
    /// Load sets from the given path.
    #[tracing::instrument(level = "trace", ret, skip_all)]
    pub(crate) fn new<P>(path: P) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        let path = path.as_ref();

        let mut sets = Self {
            path: path.into(),
            known: HashMap::default(),
            updates: Vec::default(),
        };

        let dir = match std::fs::read_dir(path) {
            Ok(dir) => dir,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(sets),
            Err(e) => return Err(e).context(anyhow!("{}", path.display())),
        };

        for e in dir {
            let e = e.with_context(|| anyhow!("{}", path.display()))?;
            let path = e.path();

            let Some(name) = path.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };

            let date = path
                .extension()
                .and_then(|ext| NaiveDateTime::parse_from_str(ext.to_str()?, DATE_FORMAT).ok());

            let known = sets
                .known
                .entry(name.to_owned())
                .or_insert_with(|| Known::new(path));

            known.base |= date.is_none();
            known.dates.extend(date);
        }

        Ok(sets)
    }

    /// Get the given set.
    pub(crate) fn load(&self, id: &str) -> Result<Option<RepoSet>> {
        let Some(known) = self.known.get(id) else {
            return Ok(None);
        };

        let mut path = known.path.clone();

        if !known.base {
            let latest = known
                .dates
                .last()
                .with_context(|| anyhow!("{id}: missing latest set"))?;

            path.set_extension(latest.format(DATE_FORMAT).to_string());
        }

        let file = match File::open(&path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).context(anyhow!("{}", path.display())),
        };

        let mut set = RepoSet::default();
        let reader = BufReader::new(file);

        for (n, line) in reader.lines().enumerate() {
            let line = line.with_context(|| anyhow!("{}", path.display()))?;
            set.raw.push(line.clone());

            if line.trim().starts_with('#') {
                continue;
            }

            set.repos.insert(RelativePathBuf::from(line), n);
        }

        Ok(Some(set))
    }

    /// Save the given set.
    pub(crate) fn save<D>(&mut self, id: &str, set: RepoSet, primary: bool, hint: &D)
    where
        D: ?Sized + fmt::Display,
    {
        self.updates
            .push((id.into(), set, primary, hint.to_string()));
    }

    /// Commit updates.
    pub(crate) fn commit(&mut self) -> Result<()> {
        fn write_set(
            set: RepoSet,
            hint: &str,
            now: DateTime<Local>,
            mut f: File,
        ) -> Result<(), anyhow::Error> {
            writeln!(f, "# {hint}")?;
            writeln!(f, "# date: {now}")?;

            for line in set.raw {
                writeln!(f, "{line}")?;
            }

            for repo in set.added {
                writeln!(f, "{repo}")?;
            }

            f.flush()?;
            Ok(())
        }

        let now = Local::now();

        for (id, set, primary, hint) in self.updates.drain(..) {
            tracing::info!(?id, "Saving set");

            let path = self.path.join(&id);
            let mut write_path = path.clone();

            if !primary {
                write_path.set_extension(now.naive_local().format(DATE_FORMAT).to_string());
            }

            if !self.path.is_dir() {
                std::fs::create_dir_all(&self.path)
                    .with_context(|| anyhow!("{}", self.path.display()))?;
            }

            let f = match File::create(&write_path) {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e).context(anyhow!("{}", write_path.display())),
            };

            write_set(set, &hint, now, f).context(anyhow!("{}", write_path.display()))?;

            let known = self.known.entry(id).or_insert_with(|| Known::new(path));

            if !primary {
                known.dates.insert(now.naive_local());
            }
        }

        // Prune old sets.
        for (id, known) in &mut self.known {
            while known.dates.len() > PRUNE {
                let Some(date) = known.dates.pop_first() else {
                    continue;
                };

                let mut path = self.path.join(id);
                path.set_extension(date.format(DATE_FORMAT).to_string());
                tracing::trace!(path = path.display().to_string(), "Removing old set");
                let _ = std::fs::remove_file(&path);
            }
        }

        Ok(())
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
#[derive(Debug, Default)]
pub(crate) struct RepoSet {
    raw: Vec<String>,
    repos: HashMap<RelativePathBuf, usize>,
    added: BTreeSet<RelativePathBuf>,
}

impl RepoSet {
    /// Add the given repo to the set.
    pub(crate) fn insert(&mut self, repo: &Repo) {
        if !self.repos.contains_key(repo.path()) {
            self.added.insert(repo.path().to_owned());
        }
    }

    /// Iterate over all repos in the set.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &RelativePath> {
        self.repos
            .keys()
            .map(|p| p.as_relative_path())
            .chain(self.added.iter().map(|p| p.as_relative_path()))
    }
}