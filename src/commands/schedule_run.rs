use std::borrow::Cow;
use std::rc::Rc;

use anyhow::{bail, Result};

use crate::rstr::{rformat, RStr};
use crate::shell::Shell;
use crate::workflows::{Eval, Step, Tree};

use super::{Env, Run};

#[derive(Clone)]
pub(crate) struct ScheduleRun {
    id: Box<str>,
    action_name: Option<Box<RStr>>,
    script: Box<str>,
    step: Rc<Step>,
    env: Env,
}

impl ScheduleRun {
    pub(super) fn new(
        id: Box<str>,
        action_name: Option<Box<RStr>>,
        script: Box<str>,
        step: Rc<Step>,
        env: Env,
    ) -> Self {
        Self {
            id,
            action_name,
            script,
            step,
            env,
        }
    }

    pub(super) fn build(self, parent: &Tree) -> Result<Run> {
        let env = self.env.extend_with(parent, &self.step.env)?;
        let eval = Eval::new(&env.tree);

        let mut skipped = None;

        if let Some(condition) = &self.step.condition {
            if !eval.test(condition)? {
                skipped = Some(condition.clone());
            }
        }

        let script = eval.eval(&self.script)?;

        let shell = self.step.shell.as_ref().map(|v| eval.eval(v)).transpose()?;
        let shell = to_shell(shell.as_deref())?;

        let id = self.step.id.as_ref().map(|v| eval.eval(v)).transpose()?;
        let name = self.step.name.as_ref().map(|v| eval.eval(v)).transpose()?;

        let name = match (self.action_name.as_deref(), name.as_deref()) {
            (Some(action_name), Some(name)) => Some(Cow::Owned(rformat!("{action_name} / {name}"))),
            (Some(action_name), None) => Some(Cow::Borrowed(action_name)),
            (None, Some(name)) => Some(Cow::Borrowed(name)),
            (None, None) => None,
        };

        let working_directory = self
            .step
            .working_directory
            .as_ref()
            .map(|v| Ok::<_, anyhow::Error>(eval.eval(v)?.into_owned()))
            .transpose()?;

        let run = Run::script(self.id, script.as_ref(), shell)
            .with_id(id.map(Cow::into_owned))
            .with_name(name.as_deref())
            .with_env(env.build_os_env())
            .with_skipped(skipped.clone())
            .with_working_directory(working_directory);

        Ok(env.decorate(run))
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
