use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use anyhow::{Context, Result};

use crate::ctxt::Ctxt;
use crate::process::OsArg;
use crate::rstr::RString;
use crate::workflows::{Eval, Tree};

use super::{ActionConfig, ActionRunner, Run};

#[derive(Clone)]
pub(super) struct Env {
    env: Rc<BTreeMap<String, RString>>,
    file_env: Rc<BTreeMap<String, Rc<Path>>>,
    env_file: Rc<Path>,
    output_file: Rc<Path>,
}

impl Env {
    pub(super) fn new(
        env: Rc<BTreeMap<String, RString>>,
        file_env: Rc<BTreeMap<String, Rc<Path>>>,
        env_file: Rc<Path>,
        output_file: Rc<Path>,
    ) -> Self {
        Self {
            env,
            file_env,
            env_file,
            output_file,
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

        for (key, value) in self.file_env.as_ref() {
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
            .with_output_file(Some(self.output_file.clone()))
    }
}

/// Construct a new environment from a specialized set of options.
pub(super) fn new_env(
    cx: &Ctxt<'_>,
    runner: Option<&ActionRunner>,
    c: Option<&ActionConfig>,
) -> Result<(Env, Tree)> {
    let cache_dir = cx
        .paths
        .project_dirs
        .context("Missing project dirs for Kick")?
        .cache_dir();

    let state_dir = cache_dir.join("state");
    let env_file;
    let output_file;

    let mut file_env = BTreeMap::new();

    if let Some(runner) = runner {
        env_file = Rc::<Path>::from(runner.state_dir().join(format!("env-{}", runner.id())));
        output_file = Rc::<Path>::from(runner.state_dir().join(format!("output-{}", runner.id())));
        file_env.insert(
            String::from("GITHUB_ACTION_PATH"),
            Rc::<Path>::from(runner.action_path()),
        );
    } else {
        env_file = Rc::<Path>::from(state_dir.join("env"));
        output_file = Rc::<Path>::from(state_dir.join("output"));
    }

    file_env.insert(String::from("GITHUB_ENV"), env_file.clone());
    file_env.insert(String::from("GITHUB_OUTPUT"), output_file.clone());

    let mut env = BTreeMap::new();
    let mut tree = Tree::new();

    if let Some(c) = c {
        let mut inputs = BTreeMap::new();

        if let Some(runner) = runner {
            for (k, v) in runner.defaults() {
                inputs.insert(k.to_owned(), RString::from(v));
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

    tree.insert(["runner", "os"], cx.os.as_tree_value());
    tree.insert_prefix(["env"], env.iter().map(|(k, v)| (k.clone(), v.clone())));
    tree.insert_prefix(
        ["env"],
        file_env
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string_lossy().into_owned())),
    );

    let env = Rc::new(env);
    let file_env = Rc::new(file_env);
    let env = Env::new(env, file_env, env_file, output_file);
    Ok((env, tree))
}
