use core::fmt;
use std::collections::HashSet;

use anyhow::{anyhow, Context, Result};
use nondestructive::yaml;
use relative_path::{RelativePath, RelativePathBuf};

use crate::ctxt::Ctxt;
use crate::manifest::Manifest;
use crate::model::Module;
use crate::validation::Validation;
use crate::workspace::{Package, Workspace};

pub(crate) struct Ci<'a> {
    path: &'a RelativePath,
    manifest: &'a Manifest,
    workspace: bool,
    validation: &'a mut Vec<Validation>,
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

/// Build ci validation.
pub(crate) fn build(
    cx: &Ctxt<'_>,
    primary_crate: &Package,
    module: &Module,
    workspace: &Workspace,
    validation: &mut Vec<Validation>,
) -> Result<()> {
    let path = module.path.join(".github").join("workflows");

    let mut ci = Ci {
        path: &path,
        manifest: &primary_crate.manifest,
        workspace: !workspace.is_single_crate(),
        validation,
    };

    validate(cx, &mut ci, module)
}

/// Validate the current model.
fn validate(cx: &Ctxt<'_>, ci: &mut Ci<'_>, module: &Module) -> Result<()> {
    let deprecated_yml = ci.path.join("rust.yml");
    let expected_path = ci.path.join("ci.yml");

    let candidates =
        candidates(cx, ci).with_context(|| anyhow!("list candidates: {path}", path = ci.path))?;

    let path = if !expected_path.to_path(cx.root).is_file() {
        let path = match &candidates[..] {
            [path] => Some(path.clone()),
            _ => None,
        };

        ci.validation.push(Validation::MissingWorkflow {
            path: expected_path,
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
        ci.validation.push(Validation::DeprecatedWorkflow {
            path: deprecated_yml,
        });
    }

    let bytes = std::fs::read(path.to_path(cx.root))?;
    let value = yaml::from_bytes(&bytes).with_context(|| anyhow!("{path}"))?;

    let name = value
        .root()
        .as_mapping()
        .and_then(|m| m.get("name")?.as_str())
        .ok_or_else(|| anyhow!("{path}: missing .name"))?;

    if name != cx.config.job_name(module) {
        ci.validation.push(Validation::WrongWorkflowName {
            path: path.clone(),
            actual: name.to_owned(),
            expected: cx.config.job_name(module).to_owned(),
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
    let Some(table) = doc.root().as_mapping() else {
        return Ok(());
    };

    if let Some(value) = table.get("on") {
        validate_on(ci, doc, value, path);
    }

    if let Some(jobs) = table.get("jobs").and_then(|v| v.as_mapping()) {
        for (_, job) in jobs {
            for action in job
                .as_mapping()
                .and_then(|m| m.get("steps")?.as_sequence())
                .into_iter()
                .flatten()
                .flat_map(|v| v.as_mapping())
            {
                if let Some(uses) = action.get("uses").and_then(|v| v.as_str()) {
                    if let Some((name, version)) = uses.split_once('@') {
                        if let Some(expected) = cx.actions.get_latest(name) {
                            if expected != version {
                                ci.validation.push(Validation::OutdatedAction {
                                    path: path.to_owned(),
                                    name: name.into(),
                                    actual: version.into(),
                                    expected: expected.into(),
                                });
                            }
                        }

                        if let Some(reason) = cx.actions.get_deny(name) {
                            ci.validation.push(Validation::DeniedAction {
                                path: path.to_owned(),
                                name: name.into(),
                                reason: reason.into(),
                            });
                        }

                        if let Some(check) = cx.actions.get_check(name) {
                            if let Err(reason) = check.check(action) {
                                ci.validation.push(Validation::CustomActionsCheck {
                                    path: path.to_owned(),
                                    name: name.into(),
                                    reason,
                                });
                            }
                        }
                    }
                }
            }

            if !ci.workspace {
                verify_single_project_build(ci, path, job);
            }
        }
    }

    Ok(())
}

fn validate_on(ci: &mut Ci<'_>, doc: &yaml::Document, value: yaml::Value<'_>, path: &RelativePath) {
    let Some(m) = value.as_mapping() else {
        ci.validation.push(Validation::ActionMissingKey {
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
                ci.validation.push(Validation::ActionExpectedEmptyMapping {
                    path: path.to_owned(),
                    key: Box::from("on.pull_request"),
                });
            }
        }
        value => {
            ci.validation.push(Validation::ActionMissingKey {
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
                    ci.validation.push(Validation::ActionOnMissingBranch {
                        path: path.to_owned(),
                        key: Box::from("on.push.branches"),
                        branch: Box::from("main"),
                    });
                }
            }
            value => {
                ci.validation.push(Validation::ActionMissingKey {
                    path: path.to_owned(),
                    key: Box::from("on.push.branches"),
                    expected: ActionExpected::Sequence,
                    doc: doc.clone(),
                    actual: value.map(|v| v.id()),
                });
            }
        },
        value => {
            ci.validation.push(Validation::ActionMissingKey {
                path: path.to_owned(),
                key: Box::from("on.push"),
                expected: ActionExpected::Mapping,
                doc: doc.clone(),
                actual: value.map(|v| v.id()),
            });
        }
    }
}

fn verify_single_project_build(ci: &mut Ci<'_>, path: &RelativePath, job: yaml::Value<'_>) {
    let mut cargo_combos = Vec::new();
    let features = ci.manifest.features();

    for step in job
        .as_mapping()
        .and_then(|v| v.get("steps")?.as_sequence())
        .into_iter()
        .flatten()
        .flat_map(|v| v.as_mapping())
    {
        if let Some(command) = step.get("run").and_then(|v| v.as_str()) {
            let identity = identify_command(command, &features);

            if let RunIdentity::Cargo(cargo) = identity {
                for feature in &cargo.missing_features {
                    ci.validation.push(Validation::MissingFeature {
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
                    ci.validation.push(Validation::NoFeatures {
                        path: path.to_owned(),
                    });
                }
            }
        } else {
            ensure_feature_combo(ci, path, &cargo_combos);
        }
    }
}

/// Ensure that feature combination is valid.
fn ensure_feature_combo(ci: &mut Ci<'_>, path: &RelativePath, cargos: &[Cargo]) -> bool {
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
        ci.validation.push(Validation::MissingEmptyFeatures {
            path: path.to_owned(),
        });
    }

    if !all_features {
        ci.validation.push(Validation::MissingAllFeatures {
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
