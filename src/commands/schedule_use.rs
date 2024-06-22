use std::borrow::Cow;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::str;

use anyhow::Result;

use crate::rstr::{RStr, RString};
use crate::workflows::{Eval, Step, Tree};

use super::{ActionConfig, ActionRunners, BatchConfig, Env, Schedule, ScheduleStaticSetup};

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
    uses: RString,
    step: Step,
    tree: Rc<Tree>,
    env: Env,
}

impl ScheduleUse {
    pub(super) fn new(uses: RString, step: Step, tree: Rc<Tree>, env: Env) -> Self {
        Self {
            uses,
            step,
            tree,
            env,
        }
    }

    /// Get what this scheduled use is using.
    pub(super) fn uses(&self) -> &RStr {
        &self.uses
    }

    pub(super) fn build(
        self,
        batch: &BatchConfig<'_, '_>,
        parent: Option<&Tree>,
        runners: &ActionRunners,
    ) -> Result<RunGroup> {
        let mut tree = self.tree.as_ref().clone();

        if let Some(parent) = parent {
            tree.extend(parent);
        }

        let eval = Eval::new(&tree);
        let (_, tree_env) = self.env.build(Some((&eval, &self.step.env)))?;
        tree.insert_prefix(["env"], tree_env);
        let eval = Eval::new(&tree);

        let id = self.step.id.as_ref().map(|v| eval.eval(v)).transpose()?;

        let mut main = Vec::new();
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

        if let Some(rust_toolchain) = rust_toolchain(&self.uses, &with)? {
            main.push(Schedule::Push);

            if rust_toolchain.components.is_some() || rust_toolchain.targets.is_some() {
                let mut args = vec![
                    RString::from("toolchain"),
                    RString::from("install"),
                    RString::from(rust_toolchain.version),
                ];

                if let Some(c) = rust_toolchain.components {
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

                main.push(Schedule::StaticSetup(ScheduleStaticSetup {
                    command: "rustup",
                    args: args.clone(),
                    name: "install toolchain",
                    skipped: skipped.clone(),
                }));
            }

            main.push(Schedule::StaticSetup(ScheduleStaticSetup {
                command: "rustup",
                args: vec![RString::from("default"), rust_toolchain.version.to_owned()],
                name: "set default rust version",
                skipped: skipped.clone(),
            }));

            main.push(Schedule::Pop);
        }

        let uses_exposed = self.uses.to_exposed();

        if !should_skip_use(uses_exposed.as_ref()) {
            let c = ActionConfig::default()
                .with_id(id.map(Cow::into_owned))
                .with_skipped(skipped.as_ref())
                .with_inputs(with);

            let (runner_main, runner_post) = runners.build(batch, &c, &self.uses)?;

            main.push(Schedule::Push);
            main.extend(runner_main);
            main.push(Schedule::Pop);

            post.push(Schedule::Push);
            post.extend(runner_post);
            post.push(Schedule::Pop);
        }

        Ok(RunGroup { main, post })
    }
}

pub(super) struct RunGroup {
    pub(super) main: Vec<Schedule>,
    pub(super) post: Vec<Schedule>,
}

struct RustToolchain<'a> {
    version: &'a RStr,
    components: Option<&'a RStr>,
    targets: Option<&'a RStr>,
}

/// Extract a rust version from a `rust-toolchain` job.
fn rust_toolchain<'a>(
    uses: &'a RStr,
    with: &'a BTreeMap<String, RString>,
) -> Result<Option<RustToolchain<'a>>> {
    let Some((head, version)) = uses.split_once('@') else {
        return Ok(None);
    };

    let Some((_, what)) = head.split_once('/') else {
        return Ok(None);
    };

    if what != "rust-toolchain" {
        return Ok(None);
    }

    let version = with
        .get("toolchain")
        .map(RString::as_rstr)
        .unwrap_or(version);

    let components = with.get("components").map(RString::as_rstr);
    let targets = with.get("targets").map(RString::as_rstr);

    Ok(Some(RustToolchain {
        version,
        components,
        targets,
    }))
}
