use std::borrow::Cow;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::str;

use anyhow::Result;

use crate::config::Os;
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

        let id = self.step.id.as_ref().map(|s| Rc::<str>::from(s.as_str()));

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

        if builtin_action(
            &self.uses,
            id.as_ref(),
            &with,
            skipped.as_deref(),
            &mut main,
        )? {
            return Ok(RunGroup { main, pre, post });
        }

        let uses_exposed = self.uses.to_exposed();

        if !should_skip_use(uses_exposed.as_ref()) {
            let c = ActionConfig::new(os, self.uses.as_ref())
                .with_id(id)
                .with_skipped(skipped.as_ref())
                .with_inputs(with);

            let steps = runners.build(batch, &c)?;

            if !steps.main.is_empty() {
                main.extend(steps.main);
            }

            if !steps.pre.is_empty() {
                pre.extend(steps.pre);
            }

            if !steps.post.is_empty() {
                post.extend(steps.post);
            }
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
    id: Option<&Rc<str>>,
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
        main.push(Schedule::Push {
            name: Some(RStr::new("rust toolchain (builtin)").as_rc()),
            id: id.cloned(),
        });

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

            main.push(Schedule::StaticSetup(ScheduleStaticSetup::new(
                "rustup",
                "install toolchain",
                args.clone(),
                skipped.map(str::to_owned),
            )));
        }

        main.push(Schedule::StaticSetup(ScheduleStaticSetup::new(
            "rustup",
            "set default rust version",
            vec![RString::from("default"), rust_toolchain.version.to_owned()],
            skipped.map(str::to_owned),
        )));

        main.push(Schedule::Pop);
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
