use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::str;
use std::sync::Mutex;
use std::thread;

use anyhow::{Context, Result, anyhow, bail};
use bstr::BString;
use gix::ObjectId;
use relative_path::RelativePathBuf;
use semver::Version;
use serde::{Deserialize, Serialize};
use tracing::Level;

use crate::action::ActionKind;
use crate::ctxt::Ctxt;
use crate::rstr::RStr;
use crate::workflows::Eval;

use super::{ActionRunner, ActionRunners};

const GITHUB_BASE: &str = "https://github.com";
const WORKDIR: &str = "workdir";
const GIT: &str = "git";
const KICK_META_JSON: &str = ".kick-meta.json";
/// Version of the workdir layout.
///
/// * `v1` exported only the main, pre and post scripts of node actions,
///   renamed and flat in the workdir. Such workdirs are re-exported.
/// * `v2` exports the whole tree of every action.
const CURRENT_VERSION: &str = "v2";
/// Older layout versions whose meta can still be read, but whose workdir must
/// be re-exported.
const STALE_VERSIONS: &[&str] = &["v1"];

#[derive(PartialEq, Eq, Hash)]
pub(crate) struct StringObjectId(pub(crate) ObjectId);

impl Serialize for StringObjectId {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(&self.0.to_hex())
    }
}

impl<'de> Deserialize<'de> for StringObjectId {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<StringObjectId, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        ObjectId::from_hex(s.as_bytes())
            .map(StringObjectId)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Deserialize)]
struct KickMeta {
    id: StringObjectId,
    files: Vec<(RelativePathBuf, StringObjectId)>,
    /// Whether the workdir uses the current layout.
    #[serde(skip, default)]
    current: bool,
}

#[derive(Serialize)]
struct KickMetaRef<'a> {
    version: &'a str,
    id: StringObjectId,
    files: &'a [(RelativePathBuf, StringObjectId)],
}

/// Loaded uses.
#[derive(Default)]
pub(crate) struct Actions {
    actions: BTreeSet<(String, String, String)>,
    changed: Vec<(String, String, String)>,
    pub(crate) found_node_versions: BTreeSet<Version>,
}

impl Actions {
    /// Add an action by id.
    pub(super) fn insert_action(&mut self, id: impl AsRef<RStr>) -> Result<()> {
        let id = id.as_ref().to_exposed();
        let u = Use::parse(id.as_ref()).with_context(|| anyhow!("Bad action `{id}`"))?;

        match u {
            Use::Github(repo, name, version) => {
                let inserted = self
                    .actions
                    .insert((repo.clone(), name.clone(), version.clone()));

                if inserted {
                    self.changed.push((repo, name, version));
                }

                Ok(())
            }
        }
    }

    /// Synchronize github uses.
    ///
    /// Distinct repos are fetched concurrently, after which every action is
    /// loaded, exported and registered in the order it was inserted.
    pub(super) fn synchronize(
        &mut self,
        runners: &mut ActionRunners,
        cx: &Ctxt<'_>,
        eval: &Eval,
    ) -> Result<()> {
        let pending = self
            .changed
            .drain(..)
            .filter(|(repo, name, version)| !runners.contains(&format!("{repo}/{name}@{version}")))
            .collect::<Vec<_>>();

        if pending.is_empty() {
            return Ok(());
        }

        let cache_dir = cx
            .paths
            .cache
            .context("Kick does not have project directories")?;

        let actions_dir = cache_dir.join("actions");

        let mut errors = Vec::new();

        // Group entries by repo, so that a single fetch covers every version
        // of it and no two fetches write the same bare git dir at once.
        let mut groups = Vec::<RepoGroup<'_>>::new();
        let mut group_index = HashMap::<(&str, &str), usize>::new();

        for (repo, name, version) in &pending {
            let index = *group_index
                .entry((repo.as_str(), name.as_str()))
                .or_insert_with(|| {
                    groups.push(RepoGroup {
                        repo,
                        name,
                        repo_dir: actions_dir.join(repo).join(name),
                        versions: Vec::new(),
                    });

                    groups.len() - 1
                });

            groups[index].versions.push(version);
        }

        let mut opened = Vec::with_capacity(groups.len());

        for group in &groups {
            match open_repo(&group.repo_dir) {
                Ok(repo) => opened.push(Some(repo)),
                Err(error) => {
                    errors.push(error.context(group.context()));
                    opened.push(None);
                }
            }
        }

        let fetched = fetch_all(&groups, &opened);

        // Load, export and register in a deterministic order.
        for (repo, name, version) in &pending {
            let index = group_index[&(repo.as_str(), name.as_str())];

            let (Some((r, _)), Some(remotes)) = (&opened[index], &fetched[index]) else {
                continue;
            };

            let result = load_action(
                runners,
                eval,
                &r.to_thread_local(),
                &groups[index].repo_dir,
                repo,
                name,
                version,
                remotes.as_deref(),
                &mut self.found_node_versions,
            );

            if let Err(error) = result {
                errors.push(error.context(format!(
                    "Failed to sync GitHub action {repo}/{name}@{version}"
                )));
            }
        }

        let mut errors = errors.into_iter();

        let Some(first) = errors.next() else {
            return Ok(());
        };

        for error in errors {
            tracing::error!("{error:?}");
        }

        Err(first)
    }
}

/// Maximum number of repos fetched at once.
const MAX_CONCURRENT_FETCHES: usize = 4;

/// The lock file in an action's repo directory, which serializes kick
/// processes fetching or exporting the same action.
const LOCK: &str = ".kick.lock";

/// Every pending version of one action repo.
struct RepoGroup<'a> {
    repo: &'a str,
    name: &'a str,
    repo_dir: PathBuf,
    versions: Vec<&'a str>,
}

impl RepoGroup<'_> {
    fn context(&self) -> String {
        format!("Failed to sync GitHub action {}", self.label())
    }

    /// Every pending action of the group, like `owner/name@v1, owner/name@v2`.
    fn label(&self) -> String {
        let mut out = String::new();

        for (n, version) in self.versions.iter().enumerate() {
            if n > 0 {
                out.push_str(", ");
            }

            out.push_str(&format!("{}/{}@{version}", self.repo, self.name));
        }

        out
    }

    /// Refspecs fetching every version into a local ref of the same name, so
    /// that the next fetch has it to negotiate with and receives nothing if
    /// the version is unchanged.
    fn refspecs(&self) -> Vec<BString> {
        let mut refspecs = Vec::new();

        for version in &self.versions {
            for name in version_refs(version) {
                refspecs.push(BString::from(format!("+{name}:{name}")));
            }
        }

        refspecs
    }
}

/// The remote refs that a version can name.
fn version_refs(version: &str) -> [BString; 2] {
    [
        BString::from(format!("refs/heads/{version}")),
        BString::from(format!("refs/tags/{version}")),
    ]
}

/// Open or initialize the bare git dir of an action repo.
///
/// Returns the repository and whether it already existed.
fn open_repo(repo_dir: &Path) -> Result<(gix::ThreadSafeRepository, bool)> {
    let git_dir = repo_dir.join(GIT);

    if !git_dir.is_dir() {
        fs::create_dir_all(&git_dir)
            .with_context(|| anyhow!("Failed to create repo directory: {}", git_dir.display()))?;
    }

    let _lock = lock_repo(repo_dir)?;

    let (r, open) = match gix::open(&git_dir) {
        Ok(r) => (r, true),
        Err(error) if error.is_not_found() => (gix::init_bare(&git_dir)?, false),
        Err(error) => return Err(error).context("Failed to open or initialize cache repository"),
    };

    Ok((r.into_sync(), open))
}

/// Take an exclusive lock on an action repo directory.
fn lock_repo(repo_dir: &Path) -> Result<Option<File>> {
    let path = repo_dir.join(LOCK);

    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .with_context(|| anyhow!("Failed to open lock file: {}", path.display()))?;

    match file.lock() {
        Ok(()) => Ok(Some(file)),
        Err(error) if error.kind() == io::ErrorKind::Unsupported => {
            tracing::warn!(?path, "File locking is not supported");
            Ok(None)
        }
        Err(error) => Err(error).with_context(|| anyhow!("Failed to lock: {}", path.display())),
    }
}

/// The outcome of fetching one repo group, indexed like the groups.
///
/// The outer `None` means the group could not be fetched or loaded at all and
/// its error has been reported. An inner `None` means the fetch failed, so its
/// actions are loaded from the cached workdir instead.
type Fetched = Option<Option<Vec<(BString, ObjectId)>>>;

/// Fetch every opened repo group, at most [`MAX_CONCURRENT_FETCHES`] at once.
fn fetch_all(
    groups: &[RepoGroup<'_>],
    opened: &[Option<(gix::ThreadSafeRepository, bool)>],
) -> Vec<Fetched> {
    let jobs = groups
        .iter()
        .zip(opened)
        .enumerate()
        .filter_map(|(index, (group, opened))| Some((index, group, opened.as_ref()?)))
        .collect::<Vec<_>>();

    let mut results = (0..groups.len()).map(|_| None).collect::<Vec<Fetched>>();

    let next = Mutex::new(jobs.into_iter());
    let workers = MAX_CONCURRENT_FETCHES.min(groups.len());

    let parent = tracing::Span::current();

    // Rendered until every fetch is done, which is when this is dropped.
    let progress = crate::gix::Fetches::new();

    let done = thread::scope(|s| {
        let handles = (0..workers)
            .map(|_| {
                let next = &next;
                let parent = &parent;
                let progress = &progress;

                s.spawn(move || {
                    let _enter = parent.enter();
                    let mut done = Vec::new();

                    loop {
                        let Some((index, group, (r, open))) =
                            next.lock().unwrap_or_else(|e| e.into_inner()).next()
                        else {
                            break;
                        };

                        done.push((index, fetch(progress, group, r, *open)));
                    }

                    done
                })
            })
            .collect::<Vec<_>>();

        handles
            .into_iter()
            .flat_map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|e| std::panic::resume_unwind(e))
            })
            .collect::<Vec<_>>()
    });

    for (index, result) in done {
        results[index] = Some(result);
    }

    results
}

/// Fetch every version of a repo group in one call.
///
/// Returns `None` if the fetch failed.
fn fetch(
    progress: &crate::gix::Fetches,
    group: &RepoGroup<'_>,
    r: &gix::ThreadSafeRepository,
    open: bool,
) -> Option<Vec<(BString, ObjectId)>> {
    let url = format!("{GITHUB_BASE}/{}/{}", group.repo, group.name);

    let span = tracing::span!(Level::DEBUG, "fetch_action", ?url, versions = ?group.versions);
    let _enter = span.enter();

    let _lock = match lock_repo(&group.repo_dir) {
        Ok(lock) => lock,
        Err(error) => {
            tracing::warn!(?error, "Failed to lock repo");
            return None;
        }
    };

    tracing::debug!(git_dir = ?r.git_dir(), "Syncing");

    let mut progress = progress.fetch(group.label());
    let result = crate::gix::sync(
        &r.to_thread_local(),
        &url,
        &group.refspecs(),
        open,
        &mut progress,
    );
    progress.finish(result.is_ok());

    match result {
        Ok(remotes) => {
            tracing::debug!(?remotes, "Found remotes");
            Some(remotes)
        }
        Err(error) => {
            tracing::warn!(?error, "Failed to sync remote");
            None
        }
    }
}

/// Load, export and register a single fetched action.
///
/// If `remotes` is `None` or contains no action for `version`, the action is
/// loaded from the id recorded in its cached workdir.
#[allow(clippy::too_many_arguments)]
fn load_action(
    runners: &mut ActionRunners,
    eval: &Eval,
    r: &gix::Repository,
    repo_dir: &Path,
    repo: &str,
    name: &str,
    version: &str,
    remotes: Option<&[(BString, ObjectId)]>,
    node_versions: &mut BTreeSet<Version>,
) -> Result<()> {
    let key = format!("{repo}/{name}@{version}");

    let work_dir = repo_dir.join(WORKDIR).join(version);
    let meta_path = work_dir.join(KICK_META_JSON);

    let span = tracing::span!(Level::DEBUG, "load_action", ?key, ?repo_dir);
    let _enter = span.enter();

    // Serialize with other kick processes exporting the same action.
    let _lock = lock_repo(repo_dir)?;

    let mut expected = version_refs(version).into_iter().collect::<HashSet<_>>();

    let mut found = None;

    for (remote_name, id) in remotes.into_iter().flatten() {
        if !expected.remove(remote_name) {
            continue;
        };

        let mut files = Vec::new();

        let (kind, action) = match crate::action::load(r, eval, *id, &mut files) {
            Ok(found) => found,
            Err(error) => {
                tracing::debug!(?remote_name, ?id, ?error, "Not an action");
                continue;
            }
        };

        tracing::debug!(?remote_name, ?id, ?kind, "Found action");

        fs::create_dir_all(&work_dir)
            .with_context(|| anyhow!("Failed to create work directory: {}", work_dir.display()))?;

        let meta = match load_meta(&meta_path)? {
            Some(meta) => KickMeta {
                id: StringObjectId(*id),
                ..meta
            },
            None => KickMeta {
                id: StringObjectId(*id),
                files: Vec::new(),
                current: false,
            },
        };

        found = Some((kind, action, files, meta));
        break;
    }

    // Try to read out remaining versions from the workdir cache.
    if found.is_none() {
        let Some(meta) = load_meta(&meta_path)? else {
            bail!("Could not find meta: {}", meta_path.display());
        };

        // Load an action runner directly out of a repository without checking it out.
        let mut files = Vec::new();
        let (kind, action) = crate::action::load(r, eval, meta.id.0, &mut files)?;
        found = Some((kind, action, files, meta));
    }

    let (kind, action, repo_files, meta) = found.context("No action found")?;

    let mut current = meta
        .files
        .iter()
        .map(|(k, v)| (k, v))
        .collect::<HashMap<_, _>>();

    let export = 'export: {
        if !meta.current {
            break 'export true;
        }

        // TODO: Only look at files that we care about instead of every file.
        for (path, actual_hash) in &repo_files {
            let Some(hash) = current.remove(path) else {
                break 'export true;
            };

            if *hash != *actual_hash {
                break 'export true;
            }
        }

        // Files removed from the action.
        !current.is_empty()
    };

    tracing::debug!(export, "Loading runner");

    if export {
        // Start from an empty workdir, so that files from an older layout or
        // an older version of the action do not linger.
        match fs::remove_dir_all(&work_dir) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(e).with_context(|| {
                    anyhow!("Failed to clear work directory: {}", work_dir.display())
                });
            }
        }

        fs::create_dir_all(&work_dir)
            .with_context(|| anyhow!("Failed to create work directory: {}", work_dir.display()))?;
    }

    let action = action.load(kind, &work_dir, export)?;

    if export {
        write_meta(
            &meta_path,
            KickMetaRef {
                version: CURRENT_VERSION,
                id: meta.id,
                files: &repo_files,
            },
        )?;
    }

    if let ActionKind::Node { node_version, .. } = action.kind {
        node_versions.insert(Version::new(node_version, 0, 0));
    }

    let runner = ActionRunner::new(
        action.kind,
        action.defaults,
        action.outputs,
        Rc::from(work_dir),
        Rc::from(repo_dir),
    );

    runners.insert(key, runner);
    Ok(())
}

fn load_meta(path: &Path) -> Result<Option<KickMeta>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e).context(path.display().to_string()),
    };

    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .with_context(|| anyhow!("{}: Failed to parse JSON", path.display()))?;

    let Some(version) = value.get("version").and_then(|version| version.as_str()) else {
        _ = fs::remove_file(path);
        return Ok(None);
    };

    let current = version == CURRENT_VERSION;

    if !current && !STALE_VERSIONS.contains(&version) {
        _ = fs::remove_file(path);
        return Ok(None);
    }

    match serde_json::from_value::<KickMeta>(value) {
        Ok(meta) => Ok(Some(KickMeta { current, ..meta })),
        Err(error) => {
            _ = fs::remove_file(path);
            tracing::warn!(?error, ?path, "Failed to parse kick meta");
            Ok(None)
        }
    }
}

fn write_meta(path: &Path, value: KickMetaRef<'_>) -> Result<()> {
    let w = File::create(path)
        .with_context(|| anyhow!("{}: Failed to create kick meta", path.display()))?;

    serde_json::to_writer_pretty(w, &value)
        .with_context(|| anyhow!("{}: Failed to write kick meta", path.display()))?;

    Ok(())
}

enum Use {
    Github(String, String, String),
}

impl Use {
    fn parse(uses: &str) -> Result<Self> {
        let ((repo, name), version) = uses
            .split_once('@')
            .and_then(|(k, v)| Some((k.split_once('/')?, v)))
            .context("Expected <repo>/<name>@<version>")?;

        Ok(Self::Github(
            repo.to_owned(),
            name.to_owned(),
            version.to_owned(),
        ))
    }
}
