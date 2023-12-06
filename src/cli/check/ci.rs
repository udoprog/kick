use core::fmt;
use std::collections::HashSet;

use anyhow::{anyhow, Context, Result};
use bstr::ByteSlice;
use nondestructive::yaml;
use relative_path::{RelativePath, RelativePathBuf};

use crate::changes::{Change, Warning, WorkflowChange};
use crate::ctxt::Ctxt;
use crate::manifest::Package;
use crate::model::Repo;
use crate::rust_version::RustVersion;
use crate::workspace::Crates;

pub(crate) struct Ci<'a> {
    path: &'a RelativePath,
    package: &'a Package<'a>,
    crates: &'a Crates,
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
    };

    validate_weekly_yml(cx, &mut ci, repo)?;
    validate_ci_yml(cx, &mut ci, repo)?;
    Ok(())
}

/// Validate the current model.
fn validate_weekly_yml(cx: &Ctxt<'_>, ci: &mut Ci<'_>, repo: &Repo) -> Result<()> {
    let path = ci.path.join("weekly.yml");

    if !path.to_path(cx.root).is_file() {
        cx.change(Change::MissingWeeklyBuild {
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
    let weekly_name = cx.config.string_variable(repo, "weekly_name")?;

    if name != weekly_name {
        cx.warning(Warning::WrongWorkflowName {
            path: path.clone(),
            actual: name.to_owned(),
            expected: weekly_name.to_owned(),
        });
    }

    Ok(())
}

/// Validate the current model.
fn validate_ci_yml(cx: &Ctxt<'_>, ci: &mut Ci<'_>, repo: &Repo) -> Result<()> {
    let deprecated_yml = ci.path.join("rust.yml");
    let expected_path = ci.path.join("ci.yml");

    let candidates =
        candidates(cx, ci).with_context(|| anyhow!("list candidates: {path}", path = ci.path))?;

    let path = if !expected_path.to_path(cx.root).is_file() {
        let path = match &candidates[..] {
            [path] => Some(path.clone()),
            _ => None,
        };

        cx.change(Change::MissingWorkflow {
            path: expected_path,
            repo: (**repo).clone(),
            candidates: candidates.clone(),
        });

        match path {
            Some(path) => path,
            None => return Ok(()),
        }
    } else {
        expected_path
    };

    if deprecated_yml.to_path(cx.root).is_file() && candidates.len() > 1 {
        cx.warning(Warning::DeprecatedWorkflow {
            path: deprecated_yml,
        });
    }

    let bytes = std::fs::read(path.to_path(cx.root))?;
    let value = yaml::from_slice(bytes).with_context(|| anyhow!("{path}"))?;

    let name = value
        .as_ref()
        .as_mapping()
        .and_then(|m| m.get("name")?.as_str())
        .ok_or_else(|| anyhow!("{path}: missing .name"))?;

    let ci_name = cx.config.string_variable(repo, "ci_name")?;

    if name != ci_name {
        cx.warning(Warning::WrongWorkflowName {
            path: path.clone(),
            actual: name.to_owned(),
            expected: ci_name.to_owned(),
        });
    }

    validate_jobs(cx, ci, &path, &value)?;
    Ok(())
}

/// Get candidates.
fn candidates(cx: &Ctxt<'_>, ci: &Ci<'_>) -> std::io::Result<Box<[RelativePathBuf]>> {
    let dir = match std::fs::read_dir(ci.path.to_path(cx.root)) {
        Ok(dir) => dir,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Box::from([])),
        Err(e) => return Err(e),
    };

    let mut paths = Vec::new();

    for e in dir {
        let e = e?;

        if let Some(name) = e.file_name().to_str() {
            paths.push(ci.path.join(name));
        }
    }

    Ok(paths.into())
}

/// Validate that jobs are modern.
fn validate_jobs(
    cx: &Ctxt<'_>,
    ci: &mut Ci<'_>,
    path: &RelativePath,
    doc: &yaml::Document,
) -> Result<()> {
    let Some(table) = doc.as_ref().as_mapping() else {
        return Ok(());
    };

    if let Some(value) = table.get("on") {
        validate_on(cx, doc, value, path);
    }

    let mut change = Vec::new();

    if let Some(jobs) = table.get("jobs").and_then(|v| v.as_mapping()) {
        for (job_name, job) in jobs {
            let Some(job) = job.as_mapping() else {
                continue;
            };

            if matches!(job_name.to_str(), Ok("test" | "build")) {
                check_strategy_rust_version(ci, &job, &mut change);
            }

            check_actions(cx, &job, &mut change)?;

            if ci.crates.is_single_crate() {
                verify_single_project_build(cx, ci, path, job)?;
            }
        }
    }

    if !change.is_empty() {
        cx.change(Change::BadWorkflow {
            path: path.to_owned(),
            doc: doc.clone(),
            change,
        });
    }

    Ok(())
}

fn check_actions(cx: &Ctxt, job: &yaml::Mapping, change: &mut Vec<WorkflowChange>) -> Result<()> {
    for action in job
        .get("steps")
        .and_then(|v| v.as_sequence())
        .into_iter()
        .flatten()
        .flat_map(|v| v.as_mapping())
    {
        let Some((uses, name)) = action.get("uses").and_then(|v| Some((v.id(), v.as_str()?)))
        else {
            continue;
        };

        let Some((base, version)) = name.split_once('@') else {
            continue;
        };

        if let Some(expected) = cx.actions.get_latest(base) {
            if expected != version {
                change.push(WorkflowChange::ReplaceString {
                    reason: format!("Outdated action: got `{version}` but expected `{expected}`"),
                    string: format!("{base}@{expected}"),
                    value: uses,
                    remove_keys: vec![],
                    set_keys: vec![],
                });
            }
        }

        if let Some(reason) = cx.actions.get_deny(base) {
            change.push(WorkflowChange::Error {
                name: name.to_owned(),
                reason: reason.into(),
            });
        }

        if let Some(check) = cx.actions.get_check(base) {
            check.check(name, action, change)?;
        }
    }

    Ok(())
}

/// Check that the correct rust-version is used in a job.
fn check_strategy_rust_version(ci: &mut Ci, job: &yaml::Mapping, change: &mut Vec<WorkflowChange>) {
    let Some(rust_version) = ci.package.rust_version().and_then(RustVersion::parse) else {
        return;
    };

    if let Some(matrix) = job
        .get("strategy")
        .and_then(|v| v.as_mapping()?.get("matrix")?.as_mapping())
    {
        for value in matrix
            .get("rust")
            .and_then(|v| v.as_sequence())
            .into_iter()
            .flatten()
        {
            let Some(string) = value.as_str() else {
                continue;
            };

            let version = match string {
                "stable" => continue,
                "beta" => continue,
                "nightly" => continue,
                version => RustVersion::parse(version),
            };

            let Some(version) = version else {
                continue;
            };

            if rust_version != version {
                change.push(WorkflowChange::ReplaceString {
                    reason: format!(
                        "Wrong rust version: got `{version}` but expected `{rust_version}`"
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
