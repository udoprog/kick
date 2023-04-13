use std::collections::BTreeSet;
use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use relative_path::{RelativePath, RelativePathBuf};

use crate::model::Module;

const EXT: &str = "modules";

/// Collection of known sets.
#[derive(Debug, Default)]
pub(crate) struct Sets {
    path: PathBuf,
    known: HashMap<String, Known>,
    new: HashMap<String, Set>,
}

impl Sets {
    /// Load sets from the given path.
    #[tracing::instrument(ret, skip_all)]
    pub(crate) fn new<P>(path: P) -> Result<Self>
    where
        P: AsRef<Path>,
    {
        let path = path.as_ref();

        let mut sets = Self {
            path: path.into(),
            known: HashMap::default(),
            new: HashMap::default(),
        };

        let dir = match std::fs::read_dir(path) {
            Ok(dir) => dir,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(sets),
            Err(e) => return Err(e).context(anyhow!("{}", path.display())),
        };

        for e in dir {
            let e = e.with_context(|| anyhow!("{}", path.display()))?;
            let path = e.path();

            let Some(EXT) = path.extension().and_then(|ext| ext.to_str()) else {
                continue;
            };

            let Some(name) = path.file_stem().and_then(|name| name.to_str()) else {
                continue;
            };

            sets.known.insert(name.to_owned(), Known { path });
        }

        Ok(sets)
    }

    /// Get the given set.
    pub(crate) fn load(&self, id: &str) -> Result<Option<Set>> {
        let Some(Known { path }) = self.known.get(id) else {
            return Ok(None);
        };

        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).context(anyhow!("{}", path.display())),
        };

        let mut set = Set::default();
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
    pub(crate) fn save(&mut self, id: &str, set: Set) {
        self.new.insert(id.into(), set);
    }

    pub(crate) fn commit(self) -> Result<()> {
        fn write_set(set: Set, mut f: File) -> Result<(), anyhow::Error> {
            for line in set.raw {
                writeln!(f, "{line}")?;
            }

            for module in set.added {
                writeln!(f, "{module}")?;
            }

            f.flush()?;

            Ok(())
        }

        for (id, set) in self.new {
            tracing::info!(?id, "saving set");

            let mut path = self.path.join(id);
            path.set_extension(EXT);

            if !self.path.is_dir() {
                std::fs::create_dir_all(&self.path)
                    .with_context(|| anyhow!("{}", self.path.display()))?;
            }

            let f = match File::create(&path) {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(e).context(anyhow!("{}", path.display())),
            };

            write_set(set, f).context(anyhow!("{}", path.display()))?;
        }

        Ok(())
    }
}

#[derive(Debug)]
struct Known {
    path: PathBuf,
}

/// A single loaded list of repos.
#[derive(Debug, Default)]
pub(crate) struct Set {
    raw: Vec<String>,
    modules: HashMap<RelativePathBuf, usize>,
    added: BTreeSet<RelativePathBuf>,
}

impl Set {
    /// Add the given module to the list.
    pub(crate) fn add(&mut self, module: &Module) {
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
