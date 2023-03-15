pub(crate) mod cargo;
mod ci;
mod readme;

use std::ops::Range;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use relative_path::RelativePathBuf;

use self::cargo::CargoIssue;
use self::ci::ActionExpected;
use crate::ctxt::Ctxt;
use crate::file::File;
use crate::manifest::Manifest;
use crate::model::{Module, OwnedCrateParams, UpdateParams};
use crate::urls::Urls;
use crate::workspace;

pub(crate) enum Validation {
    DeprecatedWorkflow {
        path: RelativePathBuf,
    },
    MissingWorkflow {
        path: RelativePathBuf,
        candidates: Box<[RelativePathBuf]>,
        crate_params: OwnedCrateParams,
    },
    WrongWorkflowName {
        path: RelativePathBuf,
        actual: String,
        expected: String,
    },
    /// Oudated version of an action.
    OutdatedAction {
        path: RelativePathBuf,
        name: String,
        actual: String,
        expected: String,
    },
    /// Deny use of the specific action.
    DeniedAction {
        path: RelativePathBuf,
        name: String,
        reason: String,
    },
    /// Actions check failed.
    CustomActionsCheck {
        path: RelativePathBuf,
        name: String,
        reason: String,
    },
    MissingReadme {
        path: RelativePathBuf,
    },
    MismatchedLibRs {
        path: RelativePathBuf,
        new_file: Arc<File>,
    },
    BadReadme {
        path: RelativePathBuf,
        new_file: Arc<File>,
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
        actual: Option<serde_yaml::Value>,
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
}

/// Run a single module.
pub(crate) fn build(
    cx: &Ctxt<'_>,
    module: &Module<'_>,
    validation: &mut Vec<Validation>,
    urls: &mut Urls,
) -> Result<()> {
    let workspace = workspace::open(cx, module)?;

    let primary_crate = match workspace.primary_crate()? {
        Some(primary_crate) => primary_crate,
        None => {
            return Err(anyhow!(
                "{}: cannot determine primary crate",
                workspace.path()
            ))
        }
    };

    let params = cx
        .config
        .per_crate_render(cx, primary_crate.crate_params(module)?);

    let documentation = match &cx.config.documentation {
        Some(documentation) => Some(documentation.render(&params)?),
        None => None,
    };

    let module_url = module.url.as_ref().map(|url| url.to_string());

    let update_params = UpdateParams {
        license: Some(cx.config.license()),
        readme: Some(readme::README_MD),
        repository: module_url.as_deref(),
        homepage: module_url.as_deref(),
        documentation: documentation.as_deref(),
        authors: &cx.config.authors,
    };

    for package in workspace.packages() {
        if package.manifest.is_publish()? {
            cargo::work_cargo_toml(package, validation, &update_params)?;
        }
    }

    if cx.config.is_enabled(module.name, "ci") {
        ci::build(&cx, &primary_crate, module, &workspace, validation)
            .with_context(|| anyhow!("ci validation: {}", cx.config.job_name()))?;
    }

    if cx.config.is_enabled(module.name, "readme") {
        readme::build(
            cx,
            workspace.path(),
            module.name,
            primary_crate,
            params.crate_params,
            validation,
            urls,
        )?;

        for package in workspace.packages() {
            if package.manifest_dir != workspace.path() {
                if package.manifest.is_publish()? {
                    let crate_params = package.crate_params(module)?;

                    readme::build(
                        cx,
                        &package.manifest_dir,
                        module.name,
                        package,
                        crate_params,
                        validation,
                        urls,
                    )?;
                }
            }
        }
    }

    Ok(())
}
