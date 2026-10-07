use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::fs::{self, File};
use std::io;
use std::io::Write;
use std::path::Path;
use std::rc::Rc;
use std::str::{self, FromStr};

use anyhow::{Context, Result, anyhow, bail};
use bstr::ByteSlice;
use gix::objs::Kind;
use gix::objs::tree::EntryMode;
use gix::{Id, ObjectId, Repository};
use nondestructive::yaml;
use relative_path::{RelativePath, RelativePathBuf};

use crate::commands::StringObjectId;
use crate::workflows::{self, Eval, Step};

/// Configuration of an action.
pub(super) struct Action {
    pub(super) kind: ActionKind,
    pub(super) defaults: BTreeMap<String, String>,
    pub(super) outputs: BTreeMap<String, String>,
}

#[derive(Debug)]
pub(super) enum ActionKind {
    Node {
        main: Rc<Path>,
        pre: Option<Rc<Path>>,
        pre_if: Option<String>,
        post: Option<Rc<Path>>,
        post_if: Option<String>,
        node_version: u64,
    },
    Composite {
        steps: Vec<Rc<Step>>,
    },
}

/// Load action context from the given repository.
pub(super) fn load<'repo>(
    repo: &'repo Repository,
    eval: &Eval,
    id: ObjectId,
    files: &mut Vec<(RelativePathBuf, StringObjectId)>,
) -> Result<(ActionRunnerKind, ActionContext<'repo>)> {
    let mut cx = ActionContext::default();

    let mut queue = VecDeque::new();

    let object = repo.find_object(id)?;
    queue.push_back((object.peel_to_tree()?, RelativePathBuf::default()));

    while let Some((tree, mut path)) = queue.pop_front() {
        for entry in tree.iter() {
            let entry = entry.map_err(gix::Error::from)?;
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

                    files.push((path.clone(), StringObjectId(ObjectId::from(id))));
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
pub(super) enum ActionRunnerKind {
    Node(Box<str>),
    Composite,
}

/// The context of an action loaded from a repo.
#[derive(Default)]
pub(super) struct ActionContext<'repo> {
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
    /// Load the action, exporting its tree into `dir` if `export` is set.
    ///
    /// The whole tree is exported for every kind of action, so that scripts
    /// can resolve sibling modules (such as code-split chunks) and
    /// `package.json` relative to their original location, just like on a
    /// GitHub runner.
    pub(super) fn load(self, kind: ActionRunnerKind, dir: &Path, export: bool) -> Result<Action> {
        let kind = match kind {
            ActionRunnerKind::Node(node) => {
                let Ok(node_version) = u64::from_str(node.as_ref()) else {
                    return Err(anyhow!("Invalid node runner version `{node}`"));
                };

                let main = self
                    .script(dir, self.main.as_deref(), "main")?
                    .with_context(|| anyhow!("Missing main script"))?;
                let pre = self.script(dir, self.pre.as_deref(), "pre")?;
                let post = self.script(dir, self.post.as_deref(), "post")?;

                ActionKind::Node {
                    main,
                    pre,
                    pre_if: self.pre_if,
                    post,
                    post_if: self.post_if,
                    node_version,
                }
            }
            ActionRunnerKind::Composite => ActionKind::Composite { steps: self.steps },
        };

        if export {
            tracing::debug!(?dir, "Exporting action");
            export_tree(dir, &self.dirs, &self.paths)?;
        }

        Ok(Action {
            kind,
            defaults: self.defaults,
            outputs: self.outputs,
        })
    }

    /// Resolve a script of a node action to its path in the exported tree.
    fn script(
        &self,
        dir: &Path,
        relative_path: Option<&RelativePath>,
        name: &str,
    ) -> Result<Option<Rc<Path>>> {
        let Some(relative_path) = relative_path else {
            return Ok(None);
        };

        let relative_path = relative_path.normalize();

        if !self.paths.contains_key(&relative_path) {
            bail!("Missing {name} script in repo: {relative_path}");
        }

        Ok(Some(Rc::from(relative_path.to_path(dir))))
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

/// Export the given directories and files of an action into `dir`.
fn export_tree(
    dir: &Path,
    dirs: &[(RelativePathBuf, EntryMode)],
    paths: &HashMap<RelativePathBuf, (Id<'_>, EntryMode)>,
) -> Result<()> {
    // Directories are in breadth-first order, so parents come first.
    for (path, _) in dirs {
        let path = path.to_path(dir);

        match fs::create_dir(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => {
                return Err(e)
                    .with_context(|| anyhow!("Failed to create directory: {}", path.display()));
            }
        }
    }

    for (path, (id, mode)) in paths {
        let path = path.to_path(dir);
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
            perm.set_mode(mode.value() as u32);

            f.set_permissions(perm).with_context(|| {
                anyhow!("Failed to set permissions on file: {}", path.display())
            })?;
        }

        #[cfg(not(unix))]
        {
            _ = mode;
        }
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::process::Command;

    use anyhow::{Result, ensure};
    use gix::ObjectId;

    use super::ActionKind;
    use crate::workflows::Eval;

    fn git(dir: &Path, args: &[&str]) -> Result<String> {
        let output = Command::new("git")
            .args(["-c", "user.name=kick", "-c", "user.email=kick@example.com"])
            .args([
                "-c",
                "commit.gpgsign=false",
                "-c",
                "core.hooksPath=/dev/null",
            ])
            .args(args)
            .current_dir(dir)
            .output()?;
        ensure!(output.status.success(), "git {args:?} failed: {output:?}");
        Ok(String::from_utf8(output.stdout)?)
    }

    /// A node action whose main script imports a code-split sibling chunk, as
    /// produced by rollup, esbuild or vite.
    #[test]
    fn node_action_imports_sibling_module() -> Result<()> {
        let tmp = tempfile::tempdir()?;
        let src = tmp.path().join("src");
        let work = tmp.path().join("work");
        fs::create_dir_all(src.join("dist"))?;
        fs::create_dir_all(&work)?;

        fs::write(
            src.join("action.yml"),
            "name: test\nruns:\n  using: node24\n  main: ./dist/main.js\n  post: dist/post.js\n",
        )?;
        fs::write(src.join("package.json"), r#"{"type": "module"}"#)?;
        fs::write(
            src.join("dist/chunk-ABC123.js"),
            "export const greeting = 'hello from chunk';\n",
        )?;
        fs::write(
            src.join("dist/main.js"),
            "import { greeting } from './chunk-ABC123.js';\nconsole.log(greeting);\n",
        )?;
        fs::write(
            src.join("dist/post.js"),
            "import { greeting } from './chunk-ABC123.js';\nconsole.log('post: ' + greeting);\n",
        )?;

        git(&src, &["init", "-q"])?;
        git(&src, &["add", "."])?;
        git(&src, &["commit", "-q", "-m", "action"])?;
        let id = git(&src, &["rev-parse", "HEAD"])?;
        let id = ObjectId::from_hex(id.trim().as_bytes()).map_err(gix::Error::from)?;

        let repo = gix::open(&src)?;
        let mut files = Vec::new();
        let (kind, cx) = super::load(&repo, Eval::empty(), id, &mut files)?;
        let action = cx.load(kind, &work, true)?;

        let ActionKind::Node {
            main,
            pre,
            post,
            node_version,
            ..
        } = &action.kind
        else {
            panic!("expected a node action, got {:?}", action.kind);
        };

        assert_eq!(*node_version, 24);
        assert_eq!(&**main, work.join("dist/main.js"));
        assert!(pre.is_none());
        assert_eq!(post.as_deref(), Some(&*work.join("dist/post.js")));
        assert!(work.join("dist/chunk-ABC123.js").is_file());
        assert!(work.join("package.json").is_file());
        assert!(work.join("action.yml").is_file());

        // Run the exported script the way kick does, if node is available.
        if let Ok(output) = Command::new("node").arg(&**main).output() {
            assert!(output.status.success(), "node failed: {output:?}");
            assert_eq!(String::from_utf8(output.stdout)?, "hello from chunk\n");
        }

        Ok(())
    }
}
