use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs::{self, File};
use std::io;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;
use std::str::{self, FromStr};

use anyhow::{anyhow, bail, Context, Result};
use gix::objs::tree::EntryMode;
use gix::objs::Kind;
use gix::{Id, ObjectId, Repository};
use nondestructive::yaml;
use relative_path::{RelativePath, RelativePathBuf};

use crate::workflows::{self, Eval, Step};

pub(crate) struct Action {
    pub(crate) kind: ActionKind,
    pub(crate) defaults: BTreeMap<String, String>,
}

#[derive(Debug)]
pub(crate) enum ActionKind {
    Node {
        main_path: Rc<Path>,
        post_path: Option<Rc<Path>>,
        node_version: u32,
    },
    Composite {
        steps: Vec<Rc<Step>>,
    },
}

/// Load action context from the given repository.
pub(crate) fn load<'repo>(
    repo: &'repo Repository,
    eval: &Eval<'_>,
    id: ObjectId,
) -> Result<(ActionRunnerKind, ActionContext<'repo>)> {
    let mut cx = ActionContext::default();

    let mut queue = VecDeque::new();

    let object = repo.find_object(id)?;
    queue.push_back((object.peel_to_tree()?, RelativePathBuf::default()));

    while let Some((tree, mut path)) = queue.pop_front() {
        for entry in tree.iter() {
            let entry = entry?;
            let id = entry.id();
            let header = id.header()?;

            let filename = str::from_utf8(entry.filename())?;
            path.push(filename);

            match header.kind() {
                Kind::Blob => {
                    tracing::trace!(?path, "blob");

                    if path == "action.yml" {
                        let object = id.object()?;

                        let action_yml =
                            yaml::from_slice(&object.data).context("Opening action.yml")?;

                        cx.process_actions_yml(&action_yml, eval)
                            .context("Processing action.yml")?;
                    }

                    cx.paths.insert(path.clone(), (id, entry.mode()));
                }
                Kind::Tree => {
                    tracing::trace!(?path, "tree");

                    cx.dirs.push((path.clone(), entry.mode()));
                    let object = id.object()?;
                    queue.push_back((object.peel_to_tree()?, path.clone()));
                }
                kind => {
                    bail!("Unsupported object: {kind}")
                }
            }

            path.pop();
        }
    }

    let kind = cx.kind.take().context("Could not determine runner kind")?;
    Ok((kind, cx))
}

/// A determined action runner kind.
#[derive(Debug)]
pub(crate) enum ActionRunnerKind {
    Node(Box<str>),
    Composite,
}

/// The context of an action loaded from a repo.
#[derive(Default)]
pub(crate) struct ActionContext<'repo> {
    kind: Option<ActionRunnerKind>,
    main: Option<RelativePathBuf>,
    post: Option<RelativePathBuf>,
    steps: Vec<Rc<Step>>,
    defaults: BTreeMap<String, String>,
    required: BTreeSet<String>,
    paths: HashMap<RelativePathBuf, (Id<'repo>, EntryMode)>,
    dirs: Vec<(RelativePathBuf, EntryMode)>,
}

impl<'repo> ActionContext<'repo> {
    /// Checkout the given object id.
    pub(crate) fn load(
        self,
        kind: ActionRunnerKind,
        dir: &Path,
        version: &str,
        export: bool,
    ) -> Result<Action> {
        let kind = match kind {
            ActionRunnerKind::Node(node_version) => {
                let Ok(node_version) = u32::from_str(node_version.as_ref()) else {
                    return Err(anyhow!("Invalid node runner version `{node_version}`"));
                };

                let mut exports = Vec::new();

                let main_path =
                    Rc::<Path>::from(dir.join(format!("main-{node_version}-{version}.js")));
                let main = self.main.with_context(|| anyhow!("Missing main script"))?;

                let (main, _) = self
                    .paths
                    .get(&main)
                    .with_context(|| anyhow!("Missing main script in repo: {main}"))?;

                exports.push((main_path.clone(), main));

                let post_path = 'post: {
                    let Some(post) = self.post else {
                        break 'post None;
                    };

                    let post_path =
                        Rc::<Path>::from(dir.join(format!("post-{node_version}-{version}.js")));

                    let (id, _) = self
                        .paths
                        .get(&post)
                        .with_context(|| anyhow!("Missing post script in repo: {post}"))?;

                    exports.push((post_path.clone(), id));
                    Some(post_path)
                };

                let (action_yml, _) = self
                    .paths
                    .get(RelativePath::new("action.yml"))
                    .context("Missing action.yml")?;
                let action_yml_path =
                    Rc::<Path>::from(dir.join(format!("action-{node_version}-{version}.yml")));

                exports.push((action_yml_path, action_yml));

                if export {
                    for (path, id) in exports {
                        let object = id.object()?;
                        tracing::debug!(?path, "Writing");

                        fs::write(&path, &object.data[..]).with_context(|| {
                            anyhow!("Failed to write main script to: {}", path.display())
                        })?;
                    }
                }

                ActionKind::Node {
                    main_path,
                    post_path,
                    node_version,
                }
            }
            ActionRunnerKind::Composite => {
                if export {
                    tracing::debug!(?dir, "Exporting composite action");

                    for (path, _) in self.dirs {
                        let path = path.to_path(dir);

                        match fs::create_dir(&path) {
                            Ok(()) => {}
                            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                            Err(e) => {
                                return Err(e).with_context(|| {
                                    anyhow!("Failed to create directory: {}", path.display())
                                });
                            }
                        }
                    }

                    for (path, (id, mode)) in self.paths {
                        let path = path.to_path(dir);
                        let object = id.object()?;

                        let mut f = File::create(&path).with_context(|| {
                            anyhow!("Failed to create file: {}", path.display())
                        })?;

                        f.write_all(&object.data[..])
                            .with_context(|| anyhow!("Failed to write file: {}", path.display()))?;

                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::PermissionsExt;

                            let meta = f.metadata()?;
                            let mut perm = meta.permissions();
                            perm.set_mode(mode.0 as u32);

                            f.set_permissions(perm).with_context(|| {
                                anyhow!("Failed to set permissions on file: {}", path.display())
                            })?;
                        }

                        #[cfg(not(unix))]
                        {
                            _ = mode;
                        }
                    }
                }

                ActionKind::Composite { steps: self.steps }
            }
        };

        Ok(Action {
            kind,
            defaults: self.defaults,
        })
    }

    fn process_actions_yml(&mut self, action_yml: &yaml::Document, eval: &Eval<'_>) -> Result<()> {
        let Some(action_yml) = action_yml.as_ref().as_mapping() else {
            bail!("Expected mapping in action.yml");
        };

        let runs = action_yml.get("runs").and_then(|v| v.as_mapping());

        if let Some(runs) = runs {
            let using = runs
                .get("using")
                .and_then(|v| v.as_str())
                .context("Missing .runs.using")?;

            if let Some(version) = using.strip_prefix("node") {
                self.kind = Some(ActionRunnerKind::Node(version.trim().into()));
            } else if using == "composite" {
                self.kind = Some(ActionRunnerKind::Composite);
            } else {
                bail!("Unsupported .runs.using: {using}");
            }

            let (steps, _, _) = workflows::load_steps(&runs, eval)?;
            self.steps = steps;

            if let Some(s) = runs.get("main").and_then(|v| v.as_str()) {
                self.main = Some(RelativePathBuf::from(s.trim().to_owned()));
            }

            if let Some(s) = runs.get("post").and_then(|v| v.as_str()) {
                self.post = Some(RelativePathBuf::from(s.trim().to_owned()));
            }
        }

        let data = action_yml
            .get("inputs")
            .and_then(|value| value.as_mapping());

        if let Some(data) = data {
            for (key, value) in data.iter() {
                let (Ok(key), Some(value)) = (str::from_utf8(key), value.as_mapping()) else {
                    continue;
                };

                if let Some(default) = value.get("default").and_then(|v| v.as_str()) {
                    self.defaults
                        .insert(key.to_owned(), default.trim().to_owned());
                }

                if let Some(true) = value.get("required").and_then(|v| v.as_bool()) {
                    self.required.insert(key.to_owned());
                }
            }
        }

        Ok(())
    }
}
