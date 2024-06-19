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
use relative_path::RelativePathBuf;
use serde_yaml::Value;

pub(crate) struct GithubAction {
    pub(crate) kind: GithubActionKind,
    pub(crate) defaults: BTreeMap<String, String>,
}

#[derive(Debug)]
pub(crate) enum GithubActionKind {
    Node {
        main: Rc<Path>,
        post: Option<Rc<Path>>,
        node_version: u32,
    },
    Composite {
        steps: Vec<GithubActionStep>,
    },
}

#[derive(Debug)]
pub(crate) struct GithubActionStep {
    pub(crate) run: Option<String>,
    pub(crate) shell: Option<String>,
    pub(crate) env: BTreeMap<String, String>,
}

/// Checkout the given object id.
pub(crate) fn load(
    repo: &Repository,
    id: ObjectId,
    work_dir: &Path,
    version: &str,
) -> Result<Option<GithubAction>> {
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

                        let action_yml = serde_yaml::from_slice::<serde_yaml::Value>(&object.data)
                            .context("Loading action.yml")?;

                        cx.process_actions_yml(&action_yml)?;
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

            GithubActionKind::Node {
                main: main_path,
                post: post_path,
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

            GithubActionKind::Composite { steps: cx.steps }
        }
    };

    Ok(Some(GithubAction {
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
    steps: Vec<GithubActionStep>,
    defaults: BTreeMap<String, String>,
    required: BTreeSet<String>,
}

impl Cx {
    fn process_actions_yml(&mut self, action_yml: &Value) -> Result<()> {
        let runs = action_yml.get("runs").and_then(|value| value.as_mapping());

        if let Some(runs) = runs {
            if let Some(using) = runs.get("using").and_then(|v| v.as_str()) {
                if let Some(version) = using.strip_prefix("node") {
                    self.kind = Some(RunnerKind::Node(version.into()));
                } else if using == "composite" {
                    self.kind = Some(RunnerKind::Composite);
                }
            }

            if let Some(values) = runs.get("steps").and_then(|v| v.as_sequence()) {
                for value in values {
                    let Some(value) = value.as_mapping() else {
                        continue;
                    };

                    let run = value.get("run").and_then(|v| v.as_str()).map(str::to_owned);

                    let shell = value
                        .get("shell")
                        .and_then(|v| v.as_str())
                        .map(str::to_owned);

                    let mut env = BTreeMap::new();

                    if let Some(e) = value.get("env").and_then(|v| v.as_mapping()) {
                        for (key, value) in e.iter() {
                            let (Some(key), Some(value)) = (key.as_str(), value.as_str()) else {
                                continue;
                            };

                            env.insert(key.to_owned(), value.to_owned());
                        }
                    }

                    self.steps.push(GithubActionStep { run, shell, env });
                }
            }

            if let Some(s) = runs.get("main").and_then(|v| v.as_str()) {
                self.main = Some(RelativePathBuf::from(s.to_owned()));
            }

            if let Some(s) = runs.get("post").and_then(|v| v.as_str()) {
                self.post = Some(RelativePathBuf::from(s.to_owned()));
            }
        }

        let data = action_yml
            .get("inputs")
            .and_then(|value| value.as_mapping());

        if let Some(data) = data {
            for (key, value) in data.iter() {
                let (Some(key), Some(value)) = (key.as_str(), value.as_mapping()) else {
                    continue;
                };

                if let Some(default) = value.get("default").and_then(|v| v.as_str()) {
                    self.defaults.insert(key.to_owned(), default.to_owned());
                }

                if let Some(true) = value.get("required").and_then(|v| v.as_bool()) {
                    self.required.insert(key.to_owned());
                }
            }
        }

        Ok(())
    }
}
