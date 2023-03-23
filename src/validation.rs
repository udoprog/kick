pub(crate) mod cargo;
mod ci;
mod readme;

use std::ops::Range;
use std::sync::Arc;

use anyhow::{anyhow, bail, Context, Result};
use nondestructive::yaml;
use relative_path::RelativePathBuf;

use self::cargo::CargoIssue;
use self::ci::ActionExpected;
use crate::ctxt::Ctxt;
use crate::file::{File, LineColumn};
use crate::manifest::Manifest;
use crate::model::{Module, ModuleParams, UpdateParams};
use crate::rust_version::RustVersion;
use crate::urls::Urls;
use crate::workspace::{Package, Workspace};

pub(crate) enum WorkflowValidation {
    /// Oudated version of an action.
    ReplaceString {
        reason: String,
        string: String,
        value: yaml::Id,
        remove_keys: Vec<(yaml::Id, String)>,
        set_keys: Vec<(yaml::Id, String, String)>,
    },
    /// Deny use of the specific action.
    Error { name: String, reason: String },
}

pub(crate) enum Validation {
    DeprecatedWorkflow {
        path: RelativePathBuf,
    },
    MissingWorkflow {
        path: RelativePathBuf,
        module: Module,
        candidates: Box<[RelativePathBuf]>,
    },
    WrongWorkflowName {
        path: RelativePathBuf,
        actual: String,
        expected: String,
    },
    BadWorkflow {
        path: RelativePathBuf,
        doc: yaml::Document,
        validation: Vec<WorkflowValidation>,
    },
    MissingReadme {
        path: RelativePathBuf,
    },
    UpdateLib {
        path: RelativePathBuf,
        lib: Arc<File>,
    },
    UpdateReadme {
        path: RelativePathBuf,
        readme: Arc<File>,
    },
    ToplevelHeadings {
        path: RelativePathBuf,
        file: Arc<File>,
        range: Range<usize>,
        line_offset: usize,
    },
    MissingPreceedingBr {
        path: RelativePathBuf,
        file: Arc<File>,
        range: Range<usize>,
        line_offset: usize,
    },
    MissingFeature {
        path: RelativePathBuf,
        feature: String,
    },
    NoFeatures {
        path: RelativePathBuf,
    },
    MissingEmptyFeatures {
        path: RelativePathBuf,
    },
    MissingAllFeatures {
        path: RelativePathBuf,
    },
    CargoTomlIssues {
        path: RelativePathBuf,
        cargo: Option<Manifest>,
        issues: Vec<CargoIssue>,
    },
    ActionMissingKey {
        path: RelativePathBuf,
        key: Box<str>,
        expected: ActionExpected,
        doc: yaml::Document,
        actual: Option<yaml::Id>,
    },
    ActionOnMissingBranch {
        path: RelativePathBuf,
        key: Box<str>,
        branch: Box<str>,
    },
    ActionExpectedEmptyMapping {
        path: RelativePathBuf,
        key: Box<str>,
    },
    SetRustVersion {
        module: Module,
        version: RustVersion,
    },
    RemoveRustVersion {
        module: Module,
    },
}

/// Run a single module.
#[tracing::instrument(skip_all)]
pub(crate) fn build(
    cx: &Ctxt<'_>,
    module: &Module,
    workspace: &Workspace,
    primary_crate: &Package,
    primary_crate_params: ModuleParams<'_>,
    urls: &mut Urls,
) -> Result<()> {
    let documentation = match &cx.config.documentation(module) {
        Some(documentation) => Some(documentation.render(&primary_crate_params)?),
        None => None,
    };

    let module_url = module.url().to_string();

    let update_params = UpdateParams {
        license: Some(cx.config.license(module)),
        readme: Some(readme::README_MD),
        repository: Some(&module_url),
        homepage: Some(&module_url),
        documentation: documentation.as_deref(),
        authors: cx.config.authors(module),
    };

    for package in workspace.packages() {
        if package.manifest.is_publish()? {
            cargo::work_cargo_toml(cx, package, &update_params)?;
        }
    }

    if cx.config.is_enabled(module.path(), "ci") {
        ci::build(cx, primary_crate, module, workspace)
            .with_context(|| anyhow!("ci validation: {}", cx.config.job_name(module)))?;
    }

    if cx.config.is_enabled(module.path(), "readme") {
        readme::build(
            cx,
            module.path(),
            module,
            primary_crate,
            &primary_crate_params,
            urls,
            true,
            false,
        )?;

        for package in workspace.packages() {
            if !package.manifest.is_publish()? {
                continue;
            }

            let params = cx.module_params(package, module)?;

            readme::build(
                cx,
                &package.manifest_dir,
                module,
                package,
                &params,
                urls,
                package.manifest_dir != *module.path(),
                true,
            )?;
        }
    }

    Ok(())
}

/// Report and apply a asingle validation.
pub(crate) fn validate(cx: &Ctxt<'_>, validation: &Validation, fix: bool) -> Result<()> {
    match validation {
        Validation::MissingWorkflow {
            path,
            module,
            candidates,
        } => {
            println!("{path}: Missing workflow");

            for candidate in candidates.iter() {
                println!("  Candidate: {candidate}");
            }

            if fix {
                if let [from] = candidates.as_ref() {
                    println!("{path}: Rename from {from}",);
                    std::fs::rename(from.to_path(cx.root), path.to_path(cx.root))?;
                } else {
                    let path = path.to_path(cx.root);

                    if let Some(parent) = path.parent() {
                        if !parent.is_dir() {
                            std::fs::create_dir_all(parent)?;
                        }
                    }

                    let workspace = module.workspace(cx)?;
                    let primary_crate = workspace.primary_crate()?;
                    let params = cx.module_params(primary_crate, module)?;

                    let Some(string) = cx.config.workflow(module, params)? else {
                        println!("  Missing default workflow!");
                        return Ok(());
                    };

                    std::fs::write(path, string)?;
                }
            }
        }
        Validation::DeprecatedWorkflow { path } => {
            println!("{path}: Reprecated Workflow");
        }
        Validation::WrongWorkflowName {
            path,
            actual,
            expected,
        } => {
            println!("{path}: Wrong workflow name: {actual} (actual) != {expected} (expected)");
        }
        Validation::BadWorkflow {
            path,
            doc,
            validation,
        } => {
            let mut doc = doc.clone();
            let mut edited = false;

            for validation in validation {
                match validation {
                    WorkflowValidation::ReplaceString {
                        reason,
                        string,
                        value: uses,
                        remove_keys,
                        set_keys,
                    } => {
                        println!("{path}: {reason}");

                        if fix {
                            doc.value_mut(*uses).set_string(string);

                            for (id, key) in remove_keys {
                                if let Some(mut m) = doc.value_mut(*id).into_mapping_mut() {
                                    if !m.remove(key) {
                                        bail!("{path}: failed to remove key `{key}`");
                                    }
                                }
                            }

                            for (id, key, value) in set_keys {
                                let mut m = doc.value_mut(*id);

                                for step in key.split('.') {
                                    let Some(next) = m.into_mapping_mut().and_then(|m| m.get_into_mut(step)) else {
                                        bail!("{path}: missing step `{step}` in key `{key}`");
                                    };

                                    m = next;
                                }

                                m.set_string(value);
                            }

                            edited = true;
                        }
                    }
                    WorkflowValidation::Error { name, reason } => {
                        println!("{path}: {name}: {reason}");
                    }
                }
            }

            if edited {
                println!("{path}: Fixing");
                std::fs::write(path.to_path(cx.root), doc.to_string())?;
            }
        }
        Validation::MissingReadme { path } => {
            println!("{path}: Missing README");
        }
        Validation::UpdateLib {
            path,
            lib: new_file,
        } => {
            if fix {
                println!("{path}: Fixing");
                std::fs::write(path.to_path(cx.root), new_file.as_str())?;
            } else {
                println!("{path}: Needs update");
            }
        }
        Validation::UpdateReadme {
            path,
            readme: new_file,
        } => {
            if fix {
                println!("{path}: Fixing");
                std::fs::write(path.to_path(cx.root), new_file.as_str())?;
            } else {
                println!("{path}: Needs update");
            }
        }
        Validation::ToplevelHeadings {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(file, range.start, *line_offset)?;
            println!("{path}:{line}:{column}: doc comment has toplevel headings");
            println!("{string}");
        }
        Validation::MissingPreceedingBr {
            path,
            file,
            range,
            line_offset,
        } => {
            let (line, column, string) = temporary_line_fix(file, range.start, *line_offset)?;
            println!("{path}:{line}:{column}: missing preceeding <br>");
            println!("{string}");
        }
        Validation::MissingFeature { path, feature } => {
            println!("{path}: missing features `{feature}`");
        }
        Validation::NoFeatures { path } => {
            println!("{path}: trying featured build (--all-features, --no-default-features), but no features present");
        }
        Validation::MissingEmptyFeatures { path } => {
            println!("{path}: missing empty features build");
        }
        Validation::MissingAllFeatures { path } => {
            println!("{path}: missing all features build");
        }
        Validation::CargoTomlIssues {
            path,
            cargo: modified_cargo,
            issues,
        } => {
            println!("{path}:");

            for issue in issues {
                println!("  {issue}");
            }

            if fix {
                if let Some(modified_cargo) = modified_cargo {
                    modified_cargo.save_to(path.to_path(cx.root))?;
                }
            }
        }
        Validation::ActionMissingKey {
            path,
            key,
            expected,
            doc,
            actual,
        } => {
            println!("{path}: {key}: action missing key, expected {expected}");

            match actual {
                Some(value) => {
                    println!("  actual:");
                    let value = doc.value(*value);
                    print!("{value}");
                }
                None => {
                    println!("  actual: *missing value*");
                }
            }
        }
        Validation::ActionOnMissingBranch { path, key, branch } => {
            println!("{path}: {key}: action missing branch `{branch}`");
        }
        Validation::ActionExpectedEmptyMapping { path, key } => {
            println!("{path}: {key}: action expected empty mapping");
        }
        Validation::SetRustVersion { module, version } => {
            let workspace = module.workspace(cx)?;

            for p in workspace.packages() {
                if p.manifest.is_publish()? {
                    tracing::info!(
                        "Saving {} with rust-version = \"{version}\"",
                        p.manifest_path
                    );
                    let mut p = p.clone();
                    p.manifest.set_rust_version(&version.to_string())?;
                    p.manifest.sort_package_keys()?;
                    p.manifest.save_to(p.manifest_path.to_path(cx.root))?;
                }
            }
        }
        Validation::RemoveRustVersion { module } => {
            let workspace = module.workspace(cx)?;

            for p in workspace.packages() {
                let mut p = p.clone();

                if p.manifest.remove_rust_version() {
                    tracing::info!(
                        "Saving {} without rust-version (target version outdates rust-version)",
                        p.manifest_path
                    );
                    p.manifest.save_to(p.manifest_path.to_path(cx.root))?;
                }
            }
        }
    };
    Ok(())
}

/// Temporary line comment fix which adjusts the line and column.
pub(crate) fn temporary_line_fix(
    file: &File,
    pos: usize,
    line_offset: usize,
) -> Result<(usize, usize, &str)> {
    let (LineColumn { line, column }, string) = file.line_column(pos)?;
    let line = line_offset + line;
    let column = column + 4;
    Ok((line, column, string))
}
