use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs::{self, File};
use std::io;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;
use std::str::{self, FromStr};

use anyhow::{anyhow, bail, Context, Result};
use bstr::ByteSlice;
use gix::objs::tree::EntryMode;
use gix::objs::Kind;
use gix::{Id, ObjectId, Repository};
use nondestructive::yaml;
use relative_path::{RelativePath, RelativePathBuf};

use crate::workflows::{self, Eval, Step};

pub(crate) struct Action {
    pub(crate) kind: ActionKind,
    pub(crate) defaults: BTreeMap<String, String>,
    pub(crate) outputs: BTreeMap<String, String>,
}

#[derive(Debug)]
pub(crate) enum ActionKind {
    Node {
        main: Rc<Path>,
        pre: Option<Rc<Path>>,
        pre_if: Option<String>,
        post: Option<Rc<Path>>,
        post_if: Option<String>,
        node_version: u32,
    },
    Composite {
        steps: Vec<Rc<Step>>,
    },
}

/// Load action context from the given repository.
pub(crate) fn load<'repo>(
    repo: &'repo Repository,
    eval: &Eval,
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

                    if let (Some("action"), Some("yml" | "yaml")) =
                        (path.file_stem(), path.extension())
                    {
                        tracing::trace!(?path, "Processing action manifest");

                        if let Some(existing) = &cx.action_yml {
                            bail!("Multiple action yml files: {existing} and {path}");
                        }

                        let object = id.object()?;

                        let action_yml = yaml::from_slice(&object.data)
                            .with_context(|| anyhow!("Reading {path}"))?;

                        cx.process_actions_yml(&action_yml, eval)
                            .with_context(|| anyhow!("Processing {path}"))?;

                        cx.action_yml = Some(path.to_owned());
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
    action_yml: Option<RelativePathBuf>,
    main: Option<RelativePathBuf>,
    pre: Option<RelativePathBuf>,
    pre_if: Option<String>,
    post: Option<RelativePathBuf>,
    post_if: Option<String>,
    steps: Vec<Rc<Step>>,
    defaults: BTreeMap<String, String>,
    outputs: BTreeMap<String, String>,
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
            ActionRunnerKind::Node(node) => {
                let Ok(node) = u32::from_str(node.as_ref()) else {
                    return Err(anyhow!("Invalid node runner version `{node}`"));
                };

                let mut out = Vec::new();

                let main = self
                    .extract(version, node, dir, &mut out, self.main.as_deref(), "main")?
                    .with_context(|| anyhow!("Missing main script"))?;
                let pre = self.extract(version, node, dir, &mut out, self.pre.as_deref(), "pre")?;
                let post =
                    self.extract(version, node, dir, &mut out, self.post.as_deref(), "post")?;

                if let Some(path) = &self.action_yml {
                    let (action_yml, _) = self
                        .paths
                        .get(path)
                        .with_context(|| anyhow!("Missing {path}"))?;

                    let action_yml_path =
                        Rc::<Path>::from(dir.join(format!("action-{node}-{node}.yml")));

                    out.push((action_yml_path, action_yml));
                }

                if export {
                    for (path, id) in out {
                        let object = id.object()?;
                        tracing::debug!(?path, "Writing");

                        fs::write(&path, &object.data[..]).with_context(|| {
                            anyhow!("Failed to write main script to: {}", path.display())
                        })?;
                    }
                }

                ActionKind::Node {
                    main,
                    pre,
                    pre_if: self.pre_if.clone(),
                    post,
                    post_if: self.post_if.clone(),
                    node_version: node,
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
            outputs: self.outputs,
        })
    }

    fn extract(
        &'repo self,
        version: &str,
        node_version: u32,
        dir: &Path,
        exports: &mut Vec<(Rc<Path>, &Id<'repo>)>,
        relative_path: Option<&RelativePath>,
        name: &str,
    ) -> Result<Option<Rc<Path>>> {
        let Some(relative_path) = relative_path else {
            return Ok(None);
        };

        let path = Rc::<Path>::from(dir.join(format!("{name}-{version}-{node_version}.js")));

        let (id, _) = self
            .paths
            .get(relative_path)
            .with_context(|| anyhow!("Missing {name} script in repo: {relative_path}"))?;

        exports.push((path.clone(), id));
        Ok(Some(path))
    }

    fn process_actions_yml(&mut self, action_yml: &yaml::Document, eval: &Eval) -> Result<()> {
        let Some(action_yml) = action_yml.as_ref().as_mapping() else {
            bail!("Expected mapping");
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

            if let Some(s) = runs.get("pre").and_then(|v| v.as_str()) {
                self.pre = Some(RelativePathBuf::from(s.trim().to_owned()));
            }

            if let Some(s) = runs.get("pre-if").and_then(|v| v.as_str()) {
                self.pre_if = Some(s.to_owned());
            }

            if let Some(s) = runs.get("main").and_then(|v| v.as_str()) {
                self.main = Some(RelativePathBuf::from(s.trim().to_owned()));
            }

            if let Some(s) = runs.get("post").and_then(|v| v.as_str()) {
                self.post = Some(RelativePathBuf::from(s.trim().to_owned()));
            }

            if let Some(s) = runs.get("post-if").and_then(|v| v.as_str()) {
                self.post_if = Some(s.to_owned());
            }
        }

        let inputs = action_yml
            .get("inputs")
            .and_then(|value| value.as_mapping());

        if let Some(inputs) = inputs {
            for (key, value) in inputs.iter() {
                let (Ok(key), Some(value)) = (str::from_utf8(key), value.as_mapping()) else {
                    continue;
                };

                if let Some(default) = value.get("default") {
                    let value = value_to_string(default)?;
                    self.defaults.insert(key.to_owned(), value);
                }

                if let Some(true) = value.get("required").and_then(|v| v.as_bool()) {
                    self.required.insert(key.to_owned());
                }
            }
        }

        let outputs = action_yml
            .get("outputs")
            .and_then(|value| value.as_mapping());

        if let Some(outputs) = outputs {
            for (key, value) in outputs.iter() {
                let (Ok(key), Some(value)) = (str::from_utf8(key), value.as_mapping()) else {
                    continue;
                };

                if let Some(value) = value.get("value") {
                    let value = value_to_string(value)?;
                    self.outputs.insert(key.to_owned(), value);
                }
            }
        }

        Ok(())
    }
}

fn value_to_string(default: yaml::Value<'_>) -> Result<String> {
    let string = match default.into_any() {
        yaml::Any::Null => "null".to_owned(),
        yaml::Any::Bool(b) => b.to_string(),
        yaml::Any::Number(n) => n.as_raw().to_string(),
        yaml::Any::String(s) => s.to_str()?.to_owned(),
        any => {
            bail!("Unsupported value: {any:?}")
        }
    };

    Ok(string)
}
