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
    env: Rc<BTreeMap<String, RString>>,
    file_env: Rc<BTreeMap<String, Rc<Path>>>,
    env_file: Rc<Path>,
    path_file: Rc<Path>,
    output_file: Rc<Path>,
    tools_path: Rc<Path>,
    temp_path: Rc<Path>,
}

impl Env {
    pub(super) fn new(
        env: Rc<BTreeMap<String, RString>>,
        file_env: Rc<BTreeMap<String, Rc<Path>>>,
        env_file: Rc<Path>,
        path_file: Rc<Path>,
        output_file: Rc<Path>,
        tools_path: Rc<Path>,
        temp_path: Rc<Path>,
    ) -> Self {
        Self {
            env,
            file_env,
            env_file,
            path_file,
            output_file,
            tools_path,
            temp_path,
        }
    }

    pub(super) fn build(
        &self,
        extra: Option<(&Eval<'_>, &BTreeMap<String, String>)>,
    ) -> Result<(BTreeMap<String, OsArg>, BTreeMap<String, RString>)> {
        let mut env = self
            .env
            .iter()
            .map(|(k, v)| (k.clone(), OsArg::from(v)))
            .collect::<BTreeMap<_, _>>();

        let mut tree_env = BTreeMap::new();

        for (key, value) in &*self.file_env {
            tree_env.insert(
                key.clone(),
                RString::from(value.to_string_lossy().into_owned()),
            );
            env.insert(key.clone(), OsArg::from(value));
        }

        if let Some((eval, input)) = extra {
            for (key, value) in input {
                let value = eval.eval(value)?.into_owned();
                tree_env.insert(key.clone(), value.clone());
                env.insert(key.clone(), OsArg::from(value));
            }
        }

        Ok((env, tree_env))
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

/// Construct a new environment from a specialized set of options.
pub(super) fn new_env(
    batch: &BatchConfig<'_, '_>,
    runner: Option<&ActionRunner>,
    c: Option<&ActionConfig>,
) -> Result<(Env, Tree)> {
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
        let base = runner.state_dir();
        let id = runner.id();

        file_env.insert(
            String::from("GITHUB_ACTION_PATH"),
            Rc::<Path>::from(runner.action_path()),
        );

        tools_path = Rc::<Path>::from(base.join(format!("{id}-tools")));
    } else {
        tools_path = Rc::<Path>::from(state_dir.join("tools"));
    }

    file_env.insert(String::from("GITHUB_ENV"), env_file.clone());
    file_env.insert(String::from("GITHUB_PATH"), path_file.clone());
    file_env.insert(String::from("GITHUB_OUTPUT"), output_file.clone());

    file_env.insert(String::from("RUNNER_TOOL_CACHE"), tools_path.clone());
    file_env.insert(String::from("RUNNER_TEMP"), temp_path.clone());

    let mut tree = Tree::new();

    if let Some(c) = c {
        runner_os = c.os();
    } else {
        runner_os = &batch.cx.current_os;
    }

    tree.insert_prefix(
        ["env"],
        file_env
            .iter()
            .map(|(k, v)| (k.clone(), RString::from(v.to_string_lossy().as_ref()))),
    );
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
            tree.insert_prefix(["env"], env.clone());
        }
    }

    let env = Rc::new(env);
    let file_env = Rc::new(file_env);

    let env = Env::new(
        env,
        file_env,
        env_file,
        path_file,
        output_file,
        tools_path,
        temp_path,
    );
    Ok((env, tree))
}
