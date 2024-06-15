use std::collections::HashSet;
use std::fmt;
use std::mem::take;

use anyhow::{anyhow, Result};
use nondestructive::yaml::{self, Id};

use crate::actions::{self, Actions};
use crate::cargo::{Package, RustVersion};
use crate::changes::{Change, Warning, WorkflowError};
use crate::config::{WorkflowConfig, WorkflowFeature};
use crate::ctxt::Ctxt;
use crate::edits;
use crate::keys::Keys;
use crate::model::Repo;
use crate::redact::Redact;
use crate::workflows::{Job, Step, Workflow, Workflows};
use crate::workspace::Crates;

pub(crate) struct Ci<'a> {
    workflows: &'a Workflows<'a>,
    actions: Actions<'a>,
    repo: &'a Repo,
    package: &'a Package<'a>,
    crates: &'a Crates,
    edits: edits::Edits,
    errors: Vec<WorkflowError>,
}

pub(crate) enum ActionExpected {
    Mapping,
}

impl fmt::Display for ActionExpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionExpected::Mapping => write!(f, "mapping"),
        }
    }
}

enum CargoKind {
    Build,
    Test,
    None,
}

struct Cargo {
    #[allow(unused)]
    kind: CargoKind,
    features: CargoFeatures,
    missing_features: Vec<String>,
    features_list: Vec<String>,
}

enum RunIdentity {
    /// A cargo build command.
    Cargo(Cargo),
    /// Empty run identity.
    None,
}

enum CargoFeatures {
    Default,
    NoDefaultFeatures,
    AllFeatures,
}

/// Build ci change.
pub(crate) fn build(cx: &Ctxt<'_>, package: &Package, repo: &Repo, crates: &Crates) -> Result<()> {
    let mut actions = Actions::default();

    for latest in cx.config.action_latest(repo) {
        actions.latest(&latest.name, &latest.version);
    }

    for deny in cx.config.action_deny(repo) {
        actions.deny(&deny.name, deny.reason.as_deref());
    }

    actions.check(
        "actions-rs/toolchain",
        &actions::ActionsRsToolchainActionsCheck,
    );

    let workflows = Workflows::new(cx, repo)?;

    let mut ci = Ci {
        workflows: &workflows,
        actions,
        repo,
        package,
        crates,
        edits: edits::Edits::default(),
        errors: Vec::new(),
    };

    let mut configs = cx.config.workflows(ci.repo)?;

    for workflow in workflows.workflows() {
        let workflow = workflow?;
        let config = configs.remove(workflow.id()).unwrap_or_default();

        if config.disable {
            continue;
        }

        validate_workflow(cx, &mut ci, &workflow, &config)?;
    }

    for (id, _) in configs {
        cx.change(Change::MissingWorkflow {
            id: id.clone(),
            path: ci.workflows.path(&id),
            repo: (**ci.repo).clone(),
        });
    }

    Ok(())
}

/// Validate the current model.
fn validate_workflow(
    cx: &Ctxt<'_>,
    ci: &mut Ci<'_>,
    w: &Workflow<'_>,
    config: &WorkflowConfig,
) -> Result<()> {
    let name = w
        .doc
        .as_ref()
        .as_mapping()
        .and_then(|m| m.get("name")?.as_str())
        .ok_or_else(|| anyhow!("{}: missing .name", w.path))?;

    if let Some(expected) = &config.name {
        if name != expected {
            cx.warning(Warning::WrongWorkflowName {
                path: w.path.clone(),
                actual: name.to_owned(),
                expected: expected.clone(),
            });
        }
    }

    validate_jobs(cx, ci, w, config)?;

    if !ci.edits.is_empty() || !ci.errors.is_empty() {
        cx.change(Change::BadWorkflow {
            path: w.path.clone(),
            doc: w.doc.clone(),
            edits: take(&mut ci.edits),
            errors: take(&mut ci.errors),
        });
    }

    Ok(())
}

/// Validate that jobs are modern.
fn validate_jobs(
    cx: &Ctxt<'_>,
    ci: &mut Ci<'_>,
    w: &Workflow<'_>,
    config: &WorkflowConfig,
) -> Result<()> {
    let Some(table) = w.doc.as_ref().as_mapping() else {
        return Ok(());
    };

    if let Some(value) = table.get("on") {
        validate_on(cx, ci, w, config, value);
    }

    for job in w.jobs(&HashSet::new())? {
        check_strategy_rust_version(ci, &job);
        check_actions(ci, &job)?;

        if ci.crates.is_single_crate() {
            verify_single_project_build(cx, ci, w, &job)?;
        }
    }

    Ok(())
}

fn check_actions(ci: &mut Ci<'_>, job: &Job) -> Result<()> {
    let policy = if job.name == "clippy" {
        RustVersionPolicy::Named("stable")
    } else {
        RustVersionPolicy::MinimumSupported
    };

    for (_, steps) in &job.matrices {
        for step in &steps.steps {
            if let Some((uses, value)) = &step.uses {
                check_action(ci, step, *uses, value)?;
                check_uses_rust_version(ci, *uses, value, policy)?;
            }

            if let Some((if_id, value)) = &step.condition {
                check_if_rust_version(ci, *if_id, value)?;
            }
        }
    }

    Ok(())
}

fn check_action(ci: &mut Ci<'_>, step: &Step, at: Id, name: &Redact) -> Result<()> {
    let name = name.to_redacted();

    let Some((base, version)) = name.split_once('@') else {
        return Ok(());
    };

    if let Some(expected) = ci.actions.get_latest(base) {
        if expected != version {
            ci.edits.set(
                at,
                format!("Outdated action: got `{version}` but expected `{expected}`"),
                format_args!("{base}@{expected}"),
            );
        }
    }

    if ci.actions.is_denied(base) {
        let reason = match ci.actions.get_deny_reason(base) {
            Some(reason) => reason.to_string(),
            None => String::from("Action is denied"),
        };

        ci.errors.push(WorkflowError::Error {
            name: name.as_ref().to_owned(),
            reason,
        });
    }

    if let Some(check) = ci.actions.get_check(base) {
        check.check(name.as_ref(), step, &mut ci.edits, &mut ci.errors)?;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy)]
enum RustVersionPolicy<'a> {
    Named(&'a str),
    MinimumSupported,
}

fn check_uses_rust_version(
    ci: &mut Ci<'_>,
    at: Id,
    name: &Redact,
    policy: RustVersionPolicy,
) -> Result<()> {
    let name = name.to_redacted();

    let Some((name, version)) = name.split_once('@') else {
        return Ok(());
    };

    let Some((author, "rust-toolchain")) = name.split_once('/') else {
        return Ok(());
    };

    match policy {
        RustVersionPolicy::Named(name) => {
            if version != name {
                ci.edits.set(
                    at,
                    "Expected stable rust toolchain",
                    format_args!("{author}/rust-toolchain@{name}"),
                );
            }
        }
        RustVersionPolicy::MinimumSupported => {
            let Some(version) = RustVersion::parse(version) else {
                return Ok(());
            };

            let Some(rust_version) = ci.package.rust_version() else {
                return Ok(());
            };

            if rust_version > version {
                ci.edits.set(
                    at,
                    format_args!(
                        "Outdated rust version in rust-toolchain action: got `{version}` but expected `{rust_version}`"
                    ),
                    format_args!("{author}/rust-toolchain@{rust_version}"),
                );
            }
        }
    }

    Ok(())
}

fn check_if_rust_version(ci: &mut Ci<'_>, at: Id, value: &str) -> Result<()> {
    let Some(rust_version) = ci.package.rust_version() else {
        return Ok(());
    };

    let Some((head, value)) = value.split_once("==") else {
        return Ok(());
    };

    if head.trim() != "matrix.rust" {
        return Ok(());
    }

    let Some(version) = RustVersion::parse(value.trim().trim_matches(|c| matches!(c, '\'' | '"')))
    else {
        return Ok(());
    };

    if rust_version > version {
        ci.edits.set(
            at,
            format_args!(
                "Outdated matrix.rust condition: got `{version}` but expected `{rust_version}`"
            ),
            format_args!("matrix.rust == '{rust_version}'"),
        );
    }

    Ok(())
}

/// Check that the correct rust-version is used in a job.
fn check_strategy_rust_version(ci: &mut Ci, job: &Job) {
    let Some(rust_version) = ci.package.rust_version() else {
        return;
    };

    for (matrix, _) in &job.matrices {
        let Some((version, id)) = matrix.get_with_id("rust") else {
            continue;
        };

        let version = &version.to_redacted();

        let Some(version) = RustVersion::parse(version.as_ref()) else {
            continue;
        };

        if rust_version > version {
            ci.edits.set(
                id,
                format_args!(
                    "build.{name}.strategy.matrix.rust: Found rust version `{version}` but expected `{rust_version}`",
                    name = job.name,
                ),
                rust_version.to_string(),
            );
        }
    }
}

fn validate_on(
    cx: &Ctxt<'_>,
    ci: &mut Ci<'_>,
    w: &Workflow<'_>,
    config: &WorkflowConfig,
    value: yaml::Value<'_>,
) {
    let mut keys = Keys::default();

    let Some(m) = value.as_mapping() else {
        cx.warning(Warning::ActionMissingKey {
            path: w.path.clone(),
            key: Box::from("on"),
            expected: ActionExpected::Mapping,
            doc: w.doc.clone(),
            actual: Some(value.id()),
        });

        return;
    };

    keys.field("on");

    let mut edits = Vec::new();

    if let Some(branch) = &config.branch {
        edits.push((
            String::from("push"),
            edits::Value::Mapping(vec![(
                String::from("branches"),
                edits::Value::Array(vec![edits::Value::String(branch.clone())]),
            )]),
        ));
    }

    if config
        .features
        .contains(&WorkflowFeature::ScheduleRandomWeekly)
    {
        let random = ci.repo.random();
        let string = format!("{} {} * * {}", random.minute, random.hour, random.day);

        edits.push((
            String::from("schedule"),
            edits::Value::Array(vec![edits::Value::Mapping(vec![(
                String::from("cron"),
                edits::Value::String(string),
            )])]),
        ));
    }

    if let Some(m) = m.get("pull_request").and_then(|v| v.as_mapping()) {
        if !m.is_empty() {
            cx.warning(Warning::ActionExpectedEmptyMapping {
                path: w.path.clone(),
                key: Box::from("on.pull_request"),
            });
        }
    }

    if !edits.is_empty() {
        ci.edits.edit_mapping(&mut keys, m, edits);
    }
}

fn verify_single_project_build(
    cx: &Ctxt<'_>,
    ci: &mut Ci<'_>,
    w: &Workflow<'_>,
    job: &Job,
) -> Result<()> {
    let mut cargo_combos = Vec::new();
    let features = ci.package.manifest().features(ci.crates)?;

    for (_, step) in &job.matrices {
        for step in &step.steps {
            if let Some(command) = &step.run {
                let identity = identify_command(command, &features);

                if let RunIdentity::Cargo(cargo) = identity {
                    for feature in &cargo.missing_features {
                        cx.warning(Warning::MissingFeature {
                            path: w.path.clone(),
                            feature: feature.clone(),
                        });
                    }

                    if matches!(cargo.kind, CargoKind::Build) {
                        cargo_combos.push(cargo);
                    }
                }
            }
        }
    }

    if !cargo_combos.is_empty() {
        if features.is_empty() {
            for build in &cargo_combos {
                if !matches!(build.features, CargoFeatures::Default) {
                    cx.warning(Warning::NoFeatures {
                        path: w.path.clone(),
                    });
                }
            }
        } else {
            ensure_feature_combo(cx, w, &cargo_combos);
        }
    }

    Ok(())
}

/// Ensure that feature combination is valid.
fn ensure_feature_combo(cx: &Ctxt<'_>, w: &Workflow<'_>, cargos: &[Cargo]) -> bool {
    let mut all_features = false;
    let mut empty_features = false;

    for cargo in cargos {
        match cargo.features {
            CargoFeatures::Default => {
                return false;
            }
            CargoFeatures::NoDefaultFeatures => {
                empty_features = empty_features || cargo.features_list.is_empty();
            }
            CargoFeatures::AllFeatures => {
                all_features = true;
            }
        }
    }

    if !empty_features {
        cx.warning(Warning::MissingEmptyFeatures {
            path: w.path.clone(),
        });
    }

    if !all_features {
        cx.warning(Warning::MissingAllFeatures {
            path: w.path.clone(),
        });
    }

    false
}

fn identify_command(command: &Redact, features: &HashSet<String>) -> RunIdentity {
    let command = command.to_redacted();
    let mut it = command.split(' ').peekable();

    if matches!(it.next(), Some("cargo")) {
        // Consume arguments.
        while it
            .peek()
            .filter(|p| p.starts_with('+') || p.starts_with('-'))
            .is_some()
        {
            it.next();
        }

        let kind = match it.next() {
            Some("build") => CargoKind::Build,
            Some("test") => CargoKind::Test,
            _ => CargoKind::None,
        };

        let (cargo_features, missing_features, features_list) = process_features(it, features);

        return RunIdentity::Cargo(Cargo {
            kind,
            features: cargo_features,
            missing_features,
            features_list,
        });
    }

    RunIdentity::None
}

fn process_features<'a>(
    mut it: std::iter::Peekable<impl Iterator<Item = &'a str>>,
    features: &HashSet<String>,
) -> (CargoFeatures, Vec<String>, Vec<String>) {
    let mut cargo_features = CargoFeatures::Default;
    let mut missing_features = Vec::new();
    let mut features_list = Vec::new();

    while let Some(arg) = it.next() {
        match arg {
            "--no-default-features" => {
                cargo_features = CargoFeatures::NoDefaultFeatures;
            }
            "--all-features" => {
                cargo_features = CargoFeatures::AllFeatures;
            }
            "--features" | "-F" => {
                if let Some(arg) = it.next() {
                    for feature in arg.split(',').map(|s| s.trim()) {
                        if !features.contains(feature) {
                            missing_features.push(feature.into());
                        }

                        features_list.push(feature.into());
                    }
                }
            }
            _ => {}
        }
    }

    (cargo_features, missing_features, features_list)
}
