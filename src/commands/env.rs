use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use anyhow::{Context, Result};

use crate::process::OsArg;
use crate::rstr::{RStr, RString};
use crate::workflows::{Eval, Tree};

use super::{ActionConfig, ActionRunner, BatchConfig, Run};

#[derive(Clone)]
pub(super) struct Env {
    pub(super) env: Rc<BTreeMap<String, RString>>,
    pub(super) tree: Rc<Tree>,
    file_env: Rc<BTreeMap<&'static str, Rc<Path>>>,
    env_file: Rc<Path>,
    path_file: Rc<Path>,
    output_file: Rc<Path>,
    tools_path: Rc<Path>,
    temp_path: Rc<Path>,
}

impl Env {
    /// Construct a new environment from a specialized set of options.
    pub(super) fn new(
        batch: &BatchConfig<'_, '_>,
        runner: Option<&ActionRunner>,
        c: Option<&ActionConfig<'_>>,
    ) -> Result<Self> {
        let cache_dir = batch
            .cx
            .paths
            .project_dirs
            .context("Missing project dirs for Kick")?
            .cache_dir();

        let state_dir = cache_dir.join("state");

        let env_file = Rc::<Path>::from(state_dir.join("env"));
        let output_file = Rc::<Path>::from(state_dir.join("output"));
        let path_file = Rc::<Path>::from(state_dir.join("path"));
        let temp_path = Rc::<Path>::from(state_dir.join("temp"));

        let tools_path;
        let runner_os;

        let mut file_env = BTreeMap::new();

        if let Some(runner) = runner {
            file_env.insert("GITHUB_ACTION_PATH", Rc::<Path>::from(runner.action_path()));
            tools_path = Rc::<Path>::from(runner.repo_dir().join("tools"));
        } else {
            tools_path = Rc::<Path>::from(state_dir.join("tools"));
        }

        file_env.insert("GITHUB_ENV", env_file.clone());
        file_env.insert("GITHUB_PATH", path_file.clone());
        file_env.insert("GITHUB_OUTPUT", output_file.clone());
        file_env.insert("RUNNER_TOOL_CACHE", tools_path.clone());
        file_env.insert("RUNNER_TEMP", temp_path.clone());

        let mut tree = Tree::new();

        if let Some(c) = c {
            runner_os = c.os();
        } else {
            runner_os = &batch.cx.current_os;
        }

        tree.insert(["runner", "os"], runner_os.as_tree_value());

        let github_tree = [(String::from("server"), RStr::new(batch.github_server()))]
            .into_iter()
            .chain(batch.github_token().map(|t| (String::from("token"), t)));

        tree.insert_prefix(["github"], github_tree);

        let mut env = BTreeMap::new();

        env.insert(
            String::from("GITHUB_SERVER"),
            RString::from(batch.github_server()),
        );

        if let Some(c) = c {
            let mut inputs = BTreeMap::new();

            if let Some(runner) = runner {
                let eval = Eval::new(&tree);

                for (k, v) in runner.defaults() {
                    inputs.insert(k.to_owned(), eval.eval(v)?.into_owned());
                }
            }

            inputs.extend(
                c.inputs()
                    .map(|(key, value)| (key.to_owned(), value.to_owned())),
            );

            if !inputs.is_empty() {
                for (key, value) in &inputs {
                    let key = key.to_uppercase();
                    env.insert(format!("INPUT_{key}"), value.clone());
                }

                tree.insert_prefix(["inputs"], inputs.clone());
            }
        }

        let env_tree = env.iter().map(|(k, v)| (k.clone(), v.clone()));
        let env_tree = env_tree.chain(
            file_env
                .iter()
                .map(|(&k, v)| (k.to_owned(), RString::from(v.to_string_lossy()))),
        );

        tree.insert_prefix(["env"], env_tree);

        let env = Rc::new(env);
        let tree = Rc::new(tree);
        let file_env = Rc::new(file_env);

        Ok(Self {
            env,
            tree,
            file_env,
            env_file,
            path_file,
            output_file,
            tools_path,
            temp_path,
        })
    }

    /// Extend with a specified set of environments.
    pub(super) fn extend_with(
        mut self,
        parent: &Tree,
        raw_env: &BTreeMap<String, String>,
    ) -> Result<Self> {
        if parent.is_empty() && raw_env.is_empty() {
            return Ok(self);
        }

        let mut tree = self.tree.as_ref().clone();

        if !parent.is_empty() {
            tree.extend(parent);
        }

        if !raw_env.is_empty() {
            let eval = Eval::new(&tree);

            let mut new_env = BTreeMap::new();

            for (k, v) in raw_env {
                new_env.insert(k.clone(), eval.eval(v)?.into_owned());
            }

            tree.insert_prefix(["env"], new_env.clone());

            let mut env = self.env.as_ref().clone();
            env.extend(new_env);
            self.env = Rc::new(env);
        }

        self.tree = Rc::new(tree);
        Ok(self)
    }

    /// Modify the tree of the environment.
    pub(super) fn with_tree(self, tree: Rc<Tree>) -> Self {
        Self { tree, ..self }
    }

    /// Build an os environment.
    pub(super) fn build_os_env(&self) -> BTreeMap<String, OsArg> {
        let files = self
            .file_env
            .iter()
            .map(|(&key, value)| (key.to_owned(), OsArg::from(value)));

        let values = self.env.iter().map(|(k, v)| (k.clone(), OsArg::from(v)));

        files.chain(values).collect::<BTreeMap<_, _>>()
    }

    #[inline]
    pub(super) fn decorate(&self, run: Run) -> Run {
        run.with_env_is_file(self.file_env.keys().cloned())
            .with_env_file(Some(self.env_file.clone()))
            .with_path_file(Some(self.path_file.clone()))
            .with_output_file(Some(self.output_file.clone()))
            .with_tools_path(self.tools_path.clone())
            .with_temp_path(self.temp_path.clone())
    }
}
