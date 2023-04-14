use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fmt;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use chrono::Local;
use chrono::NaiveDateTime;
use relative_path::{RelativePath, RelativePathBuf};

use crate::model::Module;

/// Date format for sets.
const DATE_FORMAT: &str = "%Y-%m-%d-%H%M%S";
/// Prune the three last sets.
const PRUNE: usize = 3;

/// Collection of known sets.
#[derive(Debug, Default)]
pub(crate) struct ModuleSets {
    path: PathBuf,
    known: HashMap<String, Known>,
    updates: Vec<(String, ModuleSet, bool, String)>,
}

impl ModuleSets {
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

            sets.known
                .entry(name.to_owned())
                .or_insert_with(|| Known {
                    path,
                    dates: Default::default(),
                })
                .dates
                .extend(date);
        }

        Ok(sets)
    }

    /// Get the given set.
    pub(crate) fn load(&self, id: &str) -> Result<Option<ModuleSet>> {
        let Some(Known { path, .. }) = self.known.get(id) else {
            return Ok(None);
        };

        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).context(anyhow!("{}", path.display())),
        };

        let mut set = ModuleSet::default();
        let reader = BufReader::new(file);

        for (n, line) in reader.lines().enumerate() {
            let line = line.with_context(|| anyhow!("{}", path.display()))?;
            set.raw.push(line.clone());

            if line.trim().starts_with('#') {
                continue;
            }

            set.modules.insert(RelativePathBuf::from(line), n);
        }

        Ok(Some(set))
    }

    /// Save the given set.
    pub(crate) fn save<D>(&mut self, id: &str, set: ModuleSet, primary: bool, hint: &D)
    where
        D: ?Sized + fmt::Display,
    {
        self.updates
            .push((id.into(), set, primary, hint.to_string()));
    }

    /// Commit updates.
    pub(crate) fn commit(&mut self) -> Result<()> {
        fn write_set(set: ModuleSet, hint: &str, mut f: File) -> Result<(), anyhow::Error> {
            writeln!(f, "# {hint}")?;

            for line in set.raw {
                writeln!(f, "{line}")?;
            }

            for module in set.added {
                writeln!(f, "{module}")?;
            }

            f.flush()?;
            Ok(())
        }

        let now = Local::now().naive_local();

        for (id, set, primary, hint) in self.updates.drain(..) {
            tracing::info!(?id, "Saving set");

            let path = self.path.join(&id);
            let mut write_path = path.clone();

            if !primary {
                write_path.set_extension(now.format(DATE_FORMAT).to_string());
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

            write_set(set, &hint, f).context(anyhow!("{}", write_path.display()))?;

            let known = self.known.entry(id).or_insert_with(|| Known::new(path));

            if !primary {
                known.dates.insert(now);
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
    path: PathBuf,
    dates: BTreeSet<NaiveDateTime>,
}

impl Known {
    fn new(path: PathBuf) -> Self {
        Self {
            path,
            dates: BTreeSet::default(),
        }
    }
}

/// A single loaded list of repos.
#[derive(Debug, Default)]
pub(crate) struct ModuleSet {
    raw: Vec<String>,
    modules: HashMap<RelativePathBuf, usize>,
    added: BTreeSet<RelativePathBuf>,
}

impl ModuleSet {
    /// Add the given module to the list.
    pub(crate) fn insert(&mut self, module: &Module) {
        if !self.modules.contains_key(module.path()) {
            self.added.insert(module.path().to_owned());
        }
    }

    /// Iterate over all modules in the set.
    pub(crate) fn iter(&self) -> impl Iterator<Item = &RelativePath> {
        self.modules
            .keys()
            .map(|p| p.as_relative_path())
            .chain(self.added.iter().map(|p| p.as_relative_path()))
    }
}
