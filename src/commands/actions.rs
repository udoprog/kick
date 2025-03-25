use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self};
use std::path::Path;
use std::rc::Rc;
use std::str;

use anyhow::{anyhow, bail, Context, Result};
use bstr::BString;
use gix::ObjectId;
use relative_path::RelativePathBuf;
use serde::{Deserialize, Serialize};
use tracing::Level;

use crate::ctxt::Ctxt;
use crate::rstr::RStr;
use crate::workflows::Eval;

use super::{ActionRunner, ActionRunners};

const GITHUB_BASE: &str = "https://github.com";
const WORKDIR: &str = "workdir";
const GIT: &str = "git";
const KICK_META_JSON: &str = ".kick-meta.json";
const CURRENT_VERSION: &str = "v1";

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
}

impl Actions {
    /// Add an action by id.
    pub(super) fn insert_action<S>(&mut self, id: S) -> Result<()>
    where
        S: AsRef<RStr>,
    {
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
    pub(super) fn synchronize(
        &mut self,
        runners: &mut ActionRunners,
        cx: &Ctxt<'_>,
        eval: &Eval,
    ) -> Result<()> {
        for (repo, name, version) in self.changed.drain(..) {
            sync_action(runners, cx, eval, &repo, &name, &version)
                .with_context(|| anyhow!("Failed to sync GitHub action {repo}/{name}@{version}"))?;
        }

        Ok(())
    }
}

fn sync_action(
    runners: &mut ActionRunners,
    cx: &Ctxt<'_>,
    eval: &Eval,
    repo: &str,
    name: &str,
    version: &str,
) -> Result<()> {
    let mut refspecs = Vec::new();
    let key = format!("{repo}/{name}@{version}");

    if runners.contains(&key) {
        return Ok(());
    }

    let mut expected = HashSet::new();

    for remote_name in [
        BString::from(format!("refs/heads/{version}")),
        BString::from(format!("refs/tags/{version}")),
    ] {
        refspecs.push(remote_name.clone());
        expected.insert(remote_name);
    }

    let cache_dir = cx
        .paths
        .cache
        .context("Kick does not have project directories")?;

    let actions_dir = cache_dir.join("actions");
    let repo_dir = actions_dir.join(repo).join(name);

    let git_dir = repo_dir.join(GIT);
    let work_dir = repo_dir.join(WORKDIR).join(version);
    let meta_path = work_dir.join(KICK_META_JSON);

    let span = tracing::span!(Level::DEBUG, "sync_action", ?key, ?repo_dir);
    let _enter = span.enter();

    if !git_dir.is_dir() {
        fs::create_dir_all(&git_dir)
            .with_context(|| anyhow!("Failed to create repo directory: {}", git_dir.display()))?;
    }

    let (r, open) = match gix::open(&git_dir) {
        Ok(r) => (r, true),
        Err(gix::open::Error::NotARepository { .. }) => (gix::init_bare(&git_dir)?, false),
        Err(error) => return Err(error).context("Failed to open or initialize cache repository"),
    };

    let url = format!("{GITHUB_BASE}/{repo}/{name}");

    let mut found = None;

    tracing::debug!(?git_dir, ?url, "Syncing");

    match crate::gix::sync(&r, &url, &refspecs, open) {
        Ok(remotes) => {
            tracing::debug!(?url, ?remotes, "Found remotes");

            for (remote_name, id) in remotes {
                if !expected.remove(&remote_name) || found.is_some() {
                    continue;
                };

                let mut files = Vec::new();

                let (kind, action) = match crate::action::load(&r, eval, id, &mut files) {
                    Ok(found) => found,
                    Err(error) => {
                        tracing::debug!(?remote_name, ?id, ?error, "Not an action");
                        continue;
                    }
                };

                tracing::debug!(?remote_name, ?id, ?kind, "Found action");

                fs::create_dir_all(&work_dir).with_context(|| {
                    anyhow!("Failed to create work directory: {}", work_dir.display())
                })?;

                let existing_meta = load_meta(&meta_path)?;

                let meta = existing_meta.unwrap_or_else(|| KickMeta {
                    id: StringObjectId(id),
                    files: Vec::new(),
                });

                found = Some((kind, action, files, meta));
            }
        }
        Err(error) => {
            tracing::warn!(?error, "Failed to sync remote");
        }
    }

    // Try to read out remaining versions from the workdir cache.
    if found.is_none() {
        let Some(meta) = load_meta(&meta_path)? else {
            bail!("Could not find meta: {}", meta_path.display());
        };

        // Load an action runner directly out of a repository without checking it out.
        let mut files = Vec::new();
        let (kind, action) = crate::action::load(&r, eval, meta.id.0, &mut files)?;
        found = Some((kind, action, files, meta));
    }

    let (kind, action, repo_files, meta) = found.context("No action found")?;

    let mut current = meta
        .files
        .iter()
        .map(|(k, v)| (k, v))
        .collect::<HashMap<_, _>>();

    let export = 'export: {
        // TODO: Only look at files that we care about instead of every file.
        for (path, actual_hash) in &repo_files {
            let Some(hash) = current.remove(path) else {
                break 'export true;
            };

            if *hash != *actual_hash {
                break 'export true;
            }
        }

        false
    };

    tracing::debug!(export, "Loading runner");

    let action = action.load(kind, &work_dir, version, export)?;

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

    if version != CURRENT_VERSION {
        _ = fs::remove_file(path);
        return Ok(None);
    }

    match serde_json::from_value(value) {
        Ok(id) => Ok(Some(id)),
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
