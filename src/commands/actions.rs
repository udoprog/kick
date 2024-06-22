use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self};
use std::io::{self};
use std::path::Path;
use std::rc::Rc;
use std::str;

use anyhow::{anyhow, Context, Result};
use bstr::BString;
use gix::hash::Kind;
use gix::ObjectId;

use crate::ctxt::Ctxt;
use crate::rstr::RStr;

use super::{ActionRunner, ActionRunners};

const GITHUB_BASE: &str = "https://github.com";
const GIT_OBJECT_ID_FILE: &str = ".git-object-id";
const WORKDIR: &str = "workdir";
const STATE: &str = "state";

/// Loaded uses.
#[derive(Default)]
pub(crate) struct Actions {
    actions: BTreeMap<(String, String), BTreeSet<String>>,
}

impl Actions {
    /// Check if actions is empty.
    pub(super) fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    /// Add an action by id.
    pub(super) fn insert_action<S>(&mut self, id: S) -> Result<()>
    where
        S: AsRef<RStr>,
    {
        let id = id.as_ref().to_exposed();

        let u = Use::parse(id.as_ref()).with_context(|| anyhow!("Bad action `{id}`"))?;

        match u {
            Use::Github(repo, name, version) => {
                self.actions
                    .entry((repo.to_owned(), name.to_owned()))
                    .or_default()
                    .insert(version.to_owned());
            }
        }

        Ok(())
    }

    /// Synchronize github uses.
    pub(super) fn synchronize(&self, runners: &mut ActionRunners, cx: &Ctxt<'_>) -> Result<()> {
        for ((repo, name), versions) in &self.actions {
            sync_github_use(runners, cx, repo, name, versions)
                .with_context(|| anyhow!("Failed to sync GitHub use {repo}/{name}@{versions:?}"))?;
        }

        Ok(())
    }
}

fn sync_github_use(
    runners: &mut ActionRunners,
    cx: &Ctxt<'_>,
    repo: &str,
    name: &str,
    versions: &BTreeSet<String>,
) -> Result<()> {
    let mut refspecs = Vec::new();
    let mut reverse = HashMap::new();

    for version in versions {
        let key = format!("{repo}/{name}@{version}");

        if runners.contains(&key) {
            continue;
        }

        for remote_name in [
            BString::from(format!("refs/heads/{version}")),
            BString::from(format!("refs/tags/{version}")),
        ] {
            refspecs.push(remote_name.clone());
            reverse.insert(remote_name, version);
        }
    }

    if refspecs.is_empty() {
        return Ok(());
    }

    let project_dirs = cx
        .paths
        .project_dirs
        .context("Kick does not have project directories")?;

    let cache_dir = project_dirs.cache_dir();
    let actions_dir = cache_dir.join("actions");
    let state_dir = Rc::from(cache_dir.join(STATE));
    let repo_dir = actions_dir.join(repo).join(name);
    let git_dir = repo_dir.join("git");

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

    let mut out = Vec::new();

    tracing::debug!(?git_dir, ?url, "Syncing");

    let mut found = HashSet::new();

    match crate::gix::sync(&r, &url, &refspecs, open) {
        Ok(remotes) => {
            tracing::debug!(?url, ?repo, ?name, ?remotes, "Found remotes");

            for (remote_name, id) in remotes {
                let Some(version) = reverse.remove(&remote_name) else {
                    continue;
                };

                if found.contains(version) {
                    continue;
                }

                let (kind, action) = match crate::action::load(&r, &cx.eval, id) {
                    Ok(found) => found,
                    Err(error) => {
                        tracing::debug!(?remote_name, ?version, ?id, ?error, "Not an action");
                        continue;
                    }
                };

                found.insert(version);

                tracing::debug!(?remote_name, ?version, ?id, ?kind, "Found action");

                let work_dir = repo_dir.join(WORKDIR).join(version);

                fs::create_dir_all(&work_dir).with_context(|| {
                    anyhow!("Failed to create work directory: {}", work_dir.display())
                })?;

                let id_path = work_dir.join(GIT_OBJECT_ID_FILE);
                let existing = load_id(&id_path)?;

                let export = existing != Some(id);

                if export {
                    write_id(&id_path, id)?;
                }

                out.push((kind, action, work_dir, id, repo, name, version, export));
            }
        }
        Err(error) => {
            tracing::warn!("Failed to sync remote `{repo}/{name}` with remote `{url}`: {error}");
        }
    }

    // Try to read out remaining versions from the workdir cache.
    for version in versions {
        if !found.insert(version) {
            continue;
        }

        let work_dir = repo_dir.join(WORKDIR).join(version);
        let id_path = work_dir.join(GIT_OBJECT_ID_FILE);

        let Some(id) = load_id(&id_path)? else {
            continue;
        };

        // Load an action runner directly out of a repository without checking it out.
        let (kind, action) = crate::action::load(&r, &cx.eval, id)?;
        out.push((kind, action, work_dir, id, repo, name, version, false));
    }

    for (kind, action, work_dir, id, repo, name, version, export) in out {
        let key = format!("{repo}/{name}@{version}");
        tracing::debug!(?work_dir, key, export, "Loading runner");

        let action = action.load(kind, &work_dir, version, export)?;

        fs::create_dir_all(&state_dir)
            .with_context(|| anyhow!("Failed to create envs directory: {}", state_dir.display()))?;

        let runner = ActionRunner::new(
            id.to_string(),
            action.kind,
            action.defaults,
            work_dir.into(),
            state_dir.clone(),
        );

        runners.insert(key, runner);
    }

    Ok(())
}

fn load_id(path: &Path) -> Result<Option<ObjectId>> {
    use bstr::ByteSlice;

    match fs::read(path) {
        Ok(id) => match ObjectId::from_hex(id.trim()) {
            Ok(id) => Ok(Some(id)),
            Err(e) => {
                _ = fs::remove_file(path);
                Err(e).context(anyhow!("{}: Failed to parse Object ID", path.display()))
            }
        },
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).context(path.display().to_string()),
    }
}

fn write_id(path: &Path, id: ObjectId) -> Result<()> {
    let mut buf = [0u8; const { Kind::longest().len_in_hex() + 8 }];
    let n = id.hex_to_buf(&mut buf[..]);
    buf[n] = b'\n';

    fs::write(path, &buf[..n + 1])
        .with_context(|| anyhow!("Failed to write Object ID: {}", path.display()))?;

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
