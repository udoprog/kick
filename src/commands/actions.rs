use std::collections::{BTreeSet, HashSet};
use std::fs::{self};
use std::io::{self};
use std::path::Path;
use std::rc::Rc;
use std::str;

use anyhow::{anyhow, bail, Context, Result};
use bstr::BString;
use gix::hash::Kind;
use gix::ObjectId;
use tracing::Level;

use crate::ctxt::Ctxt;
use crate::rstr::RStr;
use crate::workflows::Eval;

use super::{ActionRunner, ActionRunners};

const GITHUB_BASE: &str = "https://github.com";
const GIT_OBJECT_ID_FILE: &str = ".git-object-id";
const WORKDIR: &str = "workdir";
const GIT: &str = "git";

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

    let project_dirs = cx
        .paths
        .project_dirs
        .context("Kick does not have project directories")?;

    let actions_dir = project_dirs.cache_dir().join("actions");
    let repo_dir = actions_dir.join(repo).join(name);

    let git_dir = repo_dir.join(GIT);
    let work_dir = repo_dir.join(WORKDIR).join(version);
    let id_path = work_dir.join(GIT_OBJECT_ID_FILE);

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

                let (kind, action) = match crate::action::load(&r, eval, id) {
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

                let existing = load_id(&id_path)?;

                let export = existing != Some(id);

                if export {
                    write_id(&id_path, id)?;
                }

                found = Some((kind, action, export));
            }
        }
        Err(error) => {
            tracing::warn!(?error, "Failed to sync remote");
        }
    }

    // Try to read out remaining versions from the workdir cache.
    if found.is_none() {
        let Some(id) = load_id(&id_path)? else {
            bail!("Could not find Object ID: {}", id_path.display());
        };

        // Load an action runner directly out of a repository without checking it out.
        let (kind, action) = crate::action::load(&r, eval, id)?;
        found = Some((kind, action, false));
    }

    let (kind, action, export) = found.context("No action found")?;

    tracing::debug!(export, "Loading runner");

    let action = action.load(kind, &work_dir, version, export)?;

    let runner = ActionRunner::new(
        format!("{repo}-{name}-{version}").into(),
        action.kind,
        action.defaults,
        action.outputs,
        Rc::from(work_dir),
        Rc::from(repo_dir),
    );

    runners.insert(key, runner);
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
