use std::collections::HashSet;
use std::fmt;
use std::mem::take;

use anyhow::{anyhow, Context, Result};
use bstr::BStr;
use nondestructive::yaml::{self, Id, Mapping};
use relative_path::RelativePath;

use crate::changes::{Change, Warning, WorkflowChange};
use crate::config::{WorkflowConfig, WorkflowFeature};
use crate::ctxt::Ctxt;
use crate::manifest::Package;
use crate::model::Repo;
use crate::rust_version::RustVersion;
use crate::workspace::Crates;

pub(crate) struct Ci<'a> {
    path: &'a RelativePath,
    package: &'a Package<'a>,
    crates: &'a Crates,
    change: Vec<WorkflowChange>,
}

pub(crate) enum ActionExpected {
    Sequence,
    Mapping,
}

impl fmt::Display for ActionExpected {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ActionExpected::Sequence => write!(f, "sequence"),
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
    let path = repo.path().join(".github").join("workflows");

    let mut ci = Ci {
        path: &path,
        package,
        crates,
        change: Vec::new(),
    };

    for (id, config) in cx.config.workflows(repo)? {
        validate_workflow(cx, &mut ci, id, repo, config)?;
    }

    Ok(())
}

/// Validate the current model.
fn validate_workflow(
    cx: &Ctxt<'_>,
    ci: &mut Ci<'_>,
    id: String,
    repo: &Repo,
    config: WorkflowConfig,
) -> Result<()> {
    let path = ci.path.join(format!("{id}.yml"));

    if !path.to_path(cx.root).is_file() {
        cx.change(Change::MissingWorkflow {
            id,
            path,
            repo: (**repo).clone(),
        });

        return Ok(());
    }

    let bytes = std::fs::read(path.to_path(cx.root))?;
    let value = yaml::from_slice(bytes).with_context(|| anyhow!("{path}"))?;

    let name = value
        .as_ref()
        .as_mapping()
        .and_then(|m| m.get("name")?.as_str())
        .ok_or_else(|| anyhow!("{path}: missing .name"))?;

    if name != config.name {
        cx.warning(Warning::WrongWorkflowName {
            path: path.clone(),
            actual: name.to_owned(),
            expected: config.name.clone(),
        });
    }

    validate_jobs(cx, ci, &path, &value, &config)?;
    Ok(())
}

/// Validate that jobs are modern.
fn validate_jobs(
    cx: &Ctxt<'_>,
    ci: &mut Ci<'_>,
    path: &RelativePath,
    doc: &yaml::Document,
    config: &WorkflowConfig,
) -> Result<()> {
    let Some(table) = doc.as_ref().as_mapping() else {
        return Ok(());
    };

    if config.features.contains(&WorkflowFeature::Push) {
        if let Some(value) = table.get("on") {
            validate_on(cx, doc, value, path);
        }
    }

    if let Some(jobs) = table.get("jobs").and_then(|v| v.as_mapping()) {
        for (job_name, job) in jobs {
            let Some(job) = job.as_mapping() else {
                continue;
            };

            check_strategy_rust_version(ci, job_name, &job);
            check_actions(cx, ci, &job)?;

            if ci.crates.is_single_crate() {
                verify_single_project_build(cx, ci, path, job)?;
            }
        }
    }

    if !ci.change.is_empty() {
        cx.change(Change::BadWorkflow {
            path: path.to_owned(),
            doc: doc.clone(),
            change: take(&mut ci.change),
        });
    }

    Ok(())
}

fn check_actions(cx: &Ctxt, ci: &mut Ci<'_>, job: &yaml::Mapping) -> Result<()> {
    for action in job
        .get("steps")
        .and_then(|v| v.as_sequence())
        .into_iter()
        .flatten()
        .flat_map(|v| v.as_mapping())
    {
        if let Some((uses, value)) = action.get("uses").and_then(|v| Some((v.id(), v.as_str()?))) {
            check_action(cx, ci, &action, uses, value)?;
            check_uses_rust_version(ci, uses, value)?;
        }

        if let Some((if_id, value)) = action.get("if").and_then(|v| Some((v.id(), v.as_str()?))) {
            check_if_rust_version(ci, if_id, value)?;
        }
    }

    Ok(())
}

fn check_action(
    cx: &Ctxt,
    ci: &mut Ci<'_>,
    action: &Mapping<'_>,
    uses: Id,
    name: &str,
) -> Result<()> {
    let Some((base, version)) = name.split_once('@') else {
        return Ok(());
    };

    if let Some(expected) = cx.actions.get_latest(base) {
        if expected != version {
            ci.change.push(WorkflowChange::ReplaceString {
                reason: format!("Outdated action: got `{version}` but expected `{expected}`"),
                string: format!("{base}@{expected}"),
                value: uses,
                remove_keys: vec![],
                set_keys: vec![],
            });
        }
    }

    if let Some(reason) = cx.actions.get_deny(base) {
        ci.change.push(WorkflowChange::Error {
            name: name.to_owned(),
            reason: reason.into(),
        });
    }

    if let Some(check) = cx.actions.get_check(base) {
        check.check(name, action, &mut ci.change)?;
    }

    Ok(())
}

fn check_uses_rust_version(ci: &mut Ci<'_>, uses_id: Id, name: &str) -> Result<()> {
    let Some(rust_version) = ci.package.rust_version() else {
        return Ok(());
    };

    let Some((name, version)) = name.split_once('@') else {
        return Ok(());
    };

    let Some((author, "rust-toolchain")) = name.split_once('/') else {
        return Ok(());
    };

    let Some(version) = RustVersion::parse(version) else {
        return Ok(());
    };

    if rust_version > version {
        ci.change.push(WorkflowChange::ReplaceString {
            reason: format!(
                "Outdated rust version in rust-toolchain action: got `{version}` but expected `{rust_version}`"
            ),
            string: format!("{author}/rust-toolchain@{rust_version}"),
            value: uses_id,
            remove_keys: vec![],
            set_keys: vec![],
        });
    }

    Ok(())
}

fn check_if_rust_version(ci: &mut Ci<'_>, if_id: Id, value: &str) -> Result<()> {
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
        ci.change.push(WorkflowChange::ReplaceString {
            reason: format!(
                "Outdated matrix.rust condition: got `{version}` but expected `{rust_version}`"
            ),
            string: format!("matrix.rust == '{rust_version}'"),
            value: if_id,
            remove_keys: vec![],
            set_keys: vec![],
        });
    }

    Ok(())
}

/// Check that the correct rust-version is used in a job.
fn check_strategy_rust_version(ci: &mut Ci, job_name: &BStr, job: &yaml::Mapping) {
    let Some(rust_version) = ci.package.rust_version() else {
        return;
    };

    if let Some(matrix) = job
        .get("strategy")
        .and_then(|v| v.as_mapping()?.get("matrix")?.as_mapping())
    {
        for (index, value) in matrix
            .get("rust")
            .and_then(|v| v.as_sequence())
            .into_iter()
            .flatten()
            .enumerate()
        {
            let Some(version) = value.as_str().and_then(RustVersion::parse) else {
                continue;
            };

            if rust_version > version {
                ci.change.push(WorkflowChange::ReplaceString {
                    reason: format!(
                        "build.{job_name}.strategy.matrix.rust[{index}]: Found rust version `{version}` but expected `{rust_version}`"
                    ),
                    string: rust_version.to_string(),
                    value: value.id(),
                    remove_keys: vec![],
                    set_keys: vec![],
                });
            }
        }
    }
}

fn validate_on(cx: &Ctxt<'_>, doc: &yaml::Document, value: yaml::Value<'_>, path: &RelativePath) {
    let Some(m) = value.as_mapping() else {
        cx.warning(Warning::ActionMissingKey {
            path: path.to_owned(),
            key: Box::from("on"),
            expected: ActionExpected::Mapping,
            doc: doc.clone(),
            actual: Some(value.id()),
        });

        return;
    };

    match m.get("pull_request").map(yaml::Value::into_any) {
        Some(yaml::Any::Mapping(m)) => {
            if !m.is_empty() {
                cx.warning(Warning::ActionExpectedEmptyMapping {
                    path: path.to_owned(),
                    key: Box::from("on.pull_request"),
                });
            }
        }
        value => {
            cx.warning(Warning::ActionMissingKey {
                path: path.to_owned(),
                key: Box::from("on.pull_request"),
                expected: ActionExpected::Mapping,
                doc: doc.clone(),
                actual: value.map(|v| v.id()),
            });
        }
    }

    match m.get("push").map(yaml::Value::into_any) {
        Some(yaml::Any::Mapping(m)) => match m.get("branches").map(yaml::Value::into_any) {
            Some(yaml::Any::Sequence(s)) => {
                if !s.iter().flat_map(|v| v.as_str()).any(|b| b == "main") {
                    cx.warning(Warning::ActionOnMissingBranch {
                        path: path.to_owned(),
                        key: Box::from("on.push.branches"),
                        branch: Box::from("main"),
                    });
                }
            }
            value => {
                cx.warning(Warning::ActionMissingKey {
                    path: path.to_owned(),
                    key: Box::from("on.push.branches"),
                    expected: ActionExpected::Sequence,
                    doc: doc.clone(),
                    actual: value.map(|v| v.id()),
                });
            }
        },
        value => {
            cx.warning(Warning::ActionMissingKey {
                path: path.to_owned(),
                key: Box::from("on.push"),
                expected: ActionExpected::Mapping,
                doc: doc.clone(),
                actual: value.map(|v| v.id()),
            });
        }
    }
}

fn verify_single_project_build(
    cx: &Ctxt<'_>,
    ci: &mut Ci<'_>,
    path: &RelativePath,
    job: yaml::Mapping<'_>,
) -> Result<()> {
    let mut cargo_combos = Vec::new();
    let features = ci.package.manifest().features(ci.crates)?;

    for step in job
        .get("steps")
        .and_then(|v| v.as_sequence())
        .into_iter()
        .flatten()
        .flat_map(|v| v.as_mapping())
    {
        if let Some(command) = step.get("run").and_then(|v| v.as_str()) {
            let identity = identify_command(command, &features);

            if let RunIdentity::Cargo(cargo) = identity {
                for feature in &cargo.missing_features {
                    cx.warning(Warning::MissingFeature {
                        path: path.to_owned(),
                        feature: feature.clone(),
                    });
                }

                if matches!(cargo.kind, CargoKind::Build) {
                    cargo_combos.push(cargo);
                }
            }
        }
    }

    if !cargo_combos.is_empty() {
        if features.is_empty() {
            for build in &cargo_combos {
                if !matches!(build.features, CargoFeatures::Default) {
                    cx.warning(Warning::NoFeatures {
                        path: path.to_owned(),
                    });
                }
            }
        } else {
            ensure_feature_combo(cx, path, &cargo_combos);
        }
    }

    Ok(())
}

/// Ensure that feature combination is valid.
fn ensure_feature_combo(cx: &Ctxt<'_>, path: &RelativePath, cargos: &[Cargo]) -> bool {
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
            path: path.to_owned(),
        });
    }

    if !all_features {
        cx.warning(Warning::MissingAllFeatures {
            path: path.to_owned(),
        });
    }

    false
}

fn identify_command(command: &str, features: &HashSet<String>) -> RunIdentity {
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

fn process_features(
    mut it: std::iter::Peekable<std::str::Split<char>>,
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
                if let Some(args) = it.next() {
                    for feature in args.split(',').map(|s| s.trim()) {
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
