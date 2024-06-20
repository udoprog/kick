use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;
use std::str::{self, FromStr};

use anyhow::{anyhow, bail, Context, Result};
use gix::objs::Kind;
use gix::{ObjectId, Repository};
use nondestructive::yaml;
use relative_path::RelativePathBuf;

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
        steps: Vec<Step>,
    },
}

/// Checkout the given object id.
pub(crate) fn load(
    repo: &Repository,
    id: ObjectId,
    work_dir: &Path,
    version: &str,
) -> Result<Option<Action>> {
    let mut cx = Cx::default();

    let mut paths = HashMap::new();
    let mut dirs = Vec::new();

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
                    if path == "action.yml" {
                        let object = id.object()?;

                        let action_yml =
                            yaml::from_slice(&object.data).context("Opening action.yml")?;

                        cx.process_actions_yml(&action_yml)
                            .context("Processing action.yml")?;
                    }

                    paths.insert(path.clone(), (id, entry.mode()));
                }
                Kind::Tree => {
                    dirs.push((path.clone(), entry.mode()));
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

    let Some(kind) = cx.kind else {
        return Ok(None);
    };

    let kind = match kind {
        RunnerKind::Node(node_version) => {
            let Ok(node_version) = u32::from_str(node_version.as_ref()) else {
                return Err(anyhow!("Invalid node runner version `{node_version}`"));
            };

            let Some((main, _)) = cx.main.and_then(|p| paths.remove(&p)) else {
                return Ok(None);
            };

            let main = main.object()?;

            let post = 'post: {
                let Some((post, _)) = cx.post.and_then(|p| paths.remove(&p)) else {
                    break 'post None;
                };

                Some(post.object()?.data.clone())
            };

            let main_path = work_dir.join(format!("main-{node_version}-{version}.js"));

            fs::write(&main_path, main).with_context(|| {
                anyhow!("Failed to write main script to: {}", main_path.display())
            })?;

            let main_path = Rc::from(main_path);

            let post_path = if let Some(post) = post {
                let post_path = work_dir.join(format!("post-{node_version}-{version}.js"));

                fs::write(&post_path, post).with_context(|| {
                    anyhow!("Failed to write post script to: {}", post_path.display())
                })?;

                Some(Rc::from(post_path))
            } else {
                None
            };

            ActionKind::Node {
                main_path,
                post_path,
                node_version,
            }
        }
        RunnerKind::Composite => {
            for (dir, _) in dirs {
                let path = dir.to_path(work_dir);

                match fs::create_dir(&path) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(e) => {
                        return Err(e).with_context(|| {
                            anyhow!("Failed to create directory: {}", path.display())
                        });
                    }
                }
            }

            for (path, (id, mode)) in paths {
                let path = path.to_path(work_dir);
                let object = id.object()?;

                let mut f = File::create(&path)
                    .with_context(|| anyhow!("Failed to create file: {}", path.display()))?;

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

            ActionKind::Composite { steps: cx.steps }
        }
    };

    Ok(Some(Action {
        kind,
        defaults: cx.defaults,
    }))
}

enum RunnerKind {
    Node(Box<str>),
    Composite,
}

#[derive(Default)]
struct Cx {
    kind: Option<RunnerKind>,
    main: Option<RelativePathBuf>,
    post: Option<RelativePathBuf>,
    steps: Vec<Step>,
    defaults: BTreeMap<String, String>,
    required: BTreeSet<String>,
}

impl Cx {
    fn process_actions_yml(&mut self, action_yml: &yaml::Document) -> Result<()> {
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
                self.kind = Some(RunnerKind::Node(version.trim().into()));
            } else if using == "composite" {
                self.kind = Some(RunnerKind::Composite);
            } else {
                bail!("Unsupported .runs.using: {using}");
            }

            let eval = Eval::empty();
            let (steps, _) = workflows::load_steps(&runs, &eval)?;
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
