use std::borrow::Cow;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;

use anyhow::{bail, Result};

use crate::config::Os;
use crate::process::OsArg;
use crate::rstr::{RStr, RString};
use crate::shell::Shell;
use crate::workflows::{Eval, Step, Tree};

use super::{ActionConfig, ActionRunner, ActionRunners, BatchConfig, Env, Run, Session};

/// Schedule outputs.
#[derive(Clone)]
pub(super) struct ScheduleOutputs {
    pub(super) env: Env,
    pub(super) outputs: Rc<BTreeMap<String, String>>,
}

#[derive(Clone)]
pub(super) struct ScheduleGroup {
    pub(super) name: Option<Rc<RStr>>,
    pub(super) id: Option<Rc<RStr>>,
    pub(super) steps: Rc<[Schedule]>,
    pub(super) outputs: Option<ScheduleOutputs>,
}

impl ScheduleGroup {
    /// Construct a schedule group.
    pub(super) fn new(name: Option<Rc<RStr>>, id: Option<Rc<RStr>>, steps: Rc<[Schedule]>) -> Self {
        Self {
            name,
            id,
            steps,
            outputs: None,
        }
    }

    /// Modify outputs of the group.
    pub(super) fn with_outputs(mut self, outputs: ScheduleOutputs) -> Self {
        self.outputs = Some(outputs);
        self
    }
}

/// A scheduled action.
#[derive(Clone)]
pub(super) enum Schedule {
    Group(ScheduleGroup),
    BasicCommand(ScheduleBasicCommand),
    StaticSetup(ScheduleStaticSetup),
    NodeAction(ScheduleNodeAction),
    Run(ScheduleRun),
    Use(ScheduleUse),
}

impl Schedule {
    /// Add a preparation which matches the given schedule.
    pub(super) fn prepare(&self, session: &mut Session) -> Result<()> {
        match self {
            Schedule::Use(u) => {
                session.actions_mut().insert_action(u.uses())?;
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

/// Add jobs from a workflows, matrix, and associated steps.
pub(super) fn build_steps(
    batch: &BatchConfig<'_, '_>,
    c: Option<&ActionConfig<'_>>,
    id: Option<&Rc<RStr>>,
    name: Option<&RStr>,
    steps: &[Rc<Step>],
    runner: Option<&ActionRunner>,
) -> Result<Schedule> {
    let env = Env::new(batch, runner, c)?;

    let mut group = Vec::new();

    if !steps.is_empty() {
        for step in steps {
            let mut env = env.clone();

            if !step.tree.is_empty() {
                let tree = env.tree.with_extended(&step.tree);
                env = env.with_tree(Rc::new(tree));
            }

            if let Some(run) = &step.run {
                group.push(Schedule::Run(ScheduleRun::new(
                    Rc::from(run.as_str()),
                    step.clone(),
                    env.clone(),
                )));
            }

            if let Some(uses) = &step.uses {
                group.push(Schedule::Use(ScheduleUse::new(
                    uses.clone(),
                    step.clone(),
                    env.clone(),
                )));
            }
        }
    }

    Ok(Schedule::Group(ScheduleGroup::new(
        name.map(RStr::as_rc),
        id.cloned(),
        group.into(),
    )))
}

#[derive(Clone)]
pub(crate) struct ScheduleBasicCommand {
    command: OsArg,
    args: Vec<OsArg>,
}

impl ScheduleBasicCommand {
    pub(super) fn new<C, A>(command: C, args: A) -> Self
    where
        C: Into<OsArg>,
        A: IntoIterator<Item: Into<OsArg>>,
    {
        Self {
            command: command.into(),
            args: args.into_iter().map(Into::into).collect(),
        }
    }

    pub(super) fn build(self) -> Run {
        Run::command(self.command, self.args)
    }
}

#[derive(Clone)]
pub(crate) struct ScheduleNodeAction {
    path: Rc<Path>,
    node_version: u32,
    skipped: Option<String>,
    env: Env,
    condition: Option<String>,
}

impl ScheduleNodeAction {
    pub(crate) fn new(
        path: Rc<Path>,
        node_version: u32,
        skipped: Option<&str>,
        env: Env,
        condition: Option<String>,
    ) -> Self {
        Self {
            path,
            node_version,
            skipped: skipped.map(str::to_owned),
            env,
            condition,
        }
    }

    pub(super) fn build(self) -> Result<Run> {
        let skipped = 'skipped: {
            let Some(condition) = self.condition else {
                break 'skipped None;
            };

            let eval = Eval::new(&self.env.tree);

            if !eval.test(&condition)? {
                Some(condition)
            } else {
                None
            }
        };

        let run = Run::node(self.node_version, self.path)
            .with_skipped(self.skipped.or(skipped))
            .with_env(self.env.build_os_env());

        Ok(self.env.decorate(run))
    }
}

#[derive(Clone)]
pub(super) struct ScheduleStaticSetup {
    command: &'static str,
    name: &'static str,
    args: Vec<RString>,
    skipped: Option<String>,
}

impl ScheduleStaticSetup {
    pub(super) fn new(
        command: &'static str,
        name: &'static str,
        args: Vec<RString>,
        skipped: Option<String>,
    ) -> Self {
        Self {
            command,
            name,
            args,
            skipped,
        }
    }

    pub(super) fn build(self) -> Run {
        Run::command(self.command, self.args)
            .with_name(Some(self.name))
            .with_skipped(self.skipped)
    }
}

#[derive(Clone)]
pub(crate) struct ScheduleRun {
    script: Rc<str>,
    step: Rc<Step>,
    env: Env,
}

impl ScheduleRun {
    pub(super) fn new(script: Rc<str>, step: Rc<Step>, env: Env) -> Self {
        Self { script, step, env }
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

        let name = self.step.name.as_ref().map(|v| eval.eval(v)).transpose()?;

        let working_directory = self
            .step
            .working_directory
            .as_ref()
            .map(|v| Ok::<_, anyhow::Error>(eval.eval(v)?.into_owned()))
            .transpose()?;

        let run = Run::script(script.as_ref(), shell)
            .with_id(self.step.id.clone())
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

/// Check if a use should be skipped.
fn should_skip_use(uses: &str) -> bool {
    let Some((head, _)) = uses.split_once('@') else {
        return true;
    };

    let Some((_, what)) = head.split_once('/') else {
        return true;
    };

    matches!(what, "checkout" | "rust-toolchain")
}

#[derive(Clone)]
pub(crate) struct ScheduleUse {
    uses: Rc<RStr>,
    step: Rc<Step>,
    env: Env,
}

impl ScheduleUse {
    pub(super) fn new(uses: Rc<RStr>, step: Rc<Step>, env: Env) -> Self {
        Self { uses, step, env }
    }

    /// Get what this scheduled use is using.
    pub(super) fn uses(&self) -> &RStr {
        &self.uses
    }

    pub(super) fn build(
        self,
        batch: &BatchConfig<'_, '_>,
        parent: &Tree,
        runners: &ActionRunners,
        os: &Os,
    ) -> Result<RunGroup> {
        let env = self.env.extend_with(parent, &self.step.env)?;
        let eval = Eval::new(&env.tree);

        let id = self.step.id.as_ref();

        let mut main = Vec::new();
        let mut pre = Vec::new();
        let mut post = Vec::new();

        let mut skipped = None;

        if let Some(condition) = &self.step.condition {
            if eval.test(condition)? {
                skipped = Some(condition.clone());
            }
        }

        let with = self
            .step
            .with
            .iter()
            .map(|(k, v)| Ok((k.clone(), eval.eval(v)?.into_owned())))
            .collect::<Result<BTreeMap<_, _>>>()?;

        if builtin_action(&self.uses, id, &with, skipped.as_deref(), &mut main)? {
            return Ok(RunGroup { main, pre, post });
        }

        let uses_exposed = self.uses.to_exposed();

        if !should_skip_use(uses_exposed.as_ref()) {
            let c = ActionConfig::new(os, self.uses.as_ref())
                .repo_from_name()
                .with_id(id)
                .with_skipped(skipped.as_ref())
                .with_inputs(with);

            let steps = runners.build(batch, &c)?;

            main.push(steps.main);
            pre.extend(steps.pre);
            post.extend(steps.post);
        }

        Ok(RunGroup { main, pre, post })
    }
}

pub(super) struct RunGroup {
    pub(super) main: Vec<Schedule>,
    pub(super) pre: Vec<Schedule>,
    pub(super) post: Vec<Schedule>,
}

struct RustToolchain<'a> {
    version: &'a RStr,
    components: Option<Cow<'a, RStr>>,
    targets: Option<&'a RStr>,
}

fn builtin_action(
    uses: &RStr,
    id: Option<&Rc<RStr>>,
    with: &BTreeMap<String, RString>,
    skipped: Option<&str>,
    main: &mut Vec<Schedule>,
) -> Result<bool> {
    let Some((head, version)) = uses.split_once('@') else {
        return Ok(false);
    };

    let Some((user, repo)) = head.split_once('/') else {
        return Ok(false);
    };

    if let Some(rust_toolchain) = rust_toolchain(user, repo, version, with)? {
        let mut group = Vec::new();

        if rust_toolchain.components.is_some() || rust_toolchain.targets.is_some() {
            let mut args = vec![
                RString::from("toolchain"),
                RString::from("install"),
                RString::from(rust_toolchain.version),
            ];

            if let Some(c) = rust_toolchain.components.as_deref() {
                args.push(RString::from("-c"));
                args.push(RString::from(c));
            }

            if let Some(t) = rust_toolchain.targets {
                args.push(RString::from("-t"));
                args.push(RString::from(t));
            }

            args.extend([
                RString::from("--profile"),
                RString::from("minimal"),
                RString::from("--no-self-update"),
            ]);

            group.push(Schedule::StaticSetup(ScheduleStaticSetup::new(
                "rustup",
                "install toolchain",
                args.clone(),
                skipped.map(str::to_owned),
            )));
        }

        group.push(Schedule::StaticSetup(ScheduleStaticSetup::new(
            "rustup",
            "set default rust version",
            vec![RString::from("default"), rust_toolchain.version.to_owned()],
            skipped.map(str::to_owned),
        )));

        main.push(Schedule::Group(ScheduleGroup::new(
            Some(RStr::new("rust toolchain (builtin)").as_rc()),
            id.cloned(),
            Rc::from(group),
        )));

        return Ok(true);
    }

    Ok(false)
}

/// Extract a rust version from a `rust-toolchain` job.
fn rust_toolchain<'a>(
    user: &'a RStr,
    repo: &'a RStr,
    version: &'a RStr,
    with: &'a BTreeMap<String, RString>,
) -> Result<Option<RustToolchain<'a>>> {
    if user.str_eq("dtolnay") && repo.str_eq("rust-toolchain") {
        let version = with
            .get("toolchain")
            .map(RString::as_rstr)
            .unwrap_or(version);

        let components = with.get("components").map(|v| split_join(v, ',', ','));
        let targets = with.get("targets").map(RString::as_rstr);

        return Ok(Some(RustToolchain {
            version,
            components,
            targets,
        }));
    }

    if user.str_eq("actions-rs") && repo.str_eq("toolchain") {
        let version = with
            .get("toolchain")
            .map(RString::as_rstr)
            .unwrap_or(RStr::new("stable"));

        let components = with.get("components").map(|v| split_join(v, ',', ','));
        let target = with.get("target").map(RString::as_rstr);

        return Ok(Some(RustToolchain {
            version,
            components,
            targets: target,
        }));
    }

    Ok(None)
}

fn split_join(value: &RStr, split: char, join: char) -> Cow<'_, RStr> {
    let Some((head, tail)) = value.split_once(split) else {
        return Cow::Borrowed(value);
    };

    let mut out = RString::with_capacity(value.len());
    out.push_rstr(head.trim());
    let mut current = tail;

    while let Some((head, tail)) = current.split_once(split) {
        out.push(join);
        out.push_rstr(head.trim());
        current = tail;
    }

    out.push(join);
    out.push_rstr(current.trim());
    Cow::Owned(out)
}
