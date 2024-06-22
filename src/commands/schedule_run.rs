use std::borrow::Cow;
use std::rc::Rc;

use anyhow::{bail, Result};

use crate::rstr::RStr;
use crate::shell::Shell;
use crate::workflows::{Eval, Step, Tree};

use super::{Env, Run};

#[derive(Clone)]
pub(crate) struct ScheduleRun {
    run: String,
    step: Step,
    tree: Rc<Tree>,
    env: Env,
}

impl ScheduleRun {
    pub(super) fn new(run: String, step: Step, tree: Rc<Tree>, env: Env) -> Self {
        Self {
            run,
            step,
            tree,
            env,
        }
    }

    pub(super) fn build(self, parent: Option<&Tree>) -> Result<Run> {
        let mut tree = self.tree.as_ref().clone();

        if let Some(parent) = parent {
            tree.extend(parent);
        }

        let eval = Eval::new(&tree);
        let (env, tree_env) = self.env.build(Some((&eval, &self.step.env)))?;

        tree.insert_prefix(["env"], tree_env);
        let eval = Eval::new(&tree);

        let mut skipped = None;

        if let Some(condition) = &self.step.condition {
            if eval.test(condition)? {
                skipped = Some(condition.clone());
            }
        }

        let script = eval.eval(&self.run)?;

        let shell = self.step.shell.as_ref().map(|v| eval.eval(v)).transpose()?;
        let shell = to_shell(shell.as_deref())?;

        let id = self.step.id.as_ref().map(|v| eval.eval(v)).transpose()?;
        let name = self.step.name.as_ref().map(|v| eval.eval(v)).transpose()?;

        let working_directory = self
            .step
            .working_directory
            .as_ref()
            .map(|v| Ok::<_, anyhow::Error>(eval.eval(v)?.into_owned()))
            .transpose()?;

        let run = Run::script(script.as_ref(), shell)
            .with_id(id.map(Cow::into_owned))
            .with_name(name.as_deref())
            .with_env(env)
            .with_skipped(skipped.clone())
            .with_working_directory(working_directory);

        Ok(self.env.decorate(run))
    }
}

fn to_shell(shell: Option<&RStr>) -> Result<Shell> {
    let Some(shell) = shell else {
        return Ok(Shell::Bash);
    };

    match shell.to_exposed().as_ref() {
        "bash" => Ok(Shell::Bash),
        "powershell" => Ok(Shell::Powershell),
        _ => bail!("Unsupported shell: {shell}"),
    }
}
