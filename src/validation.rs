pub(crate) mod cargo;
mod ci;
mod readme;

use std::ops::Range;
use std::sync::Arc;

use anyhow::{anyhow, Context, Result};
use nondestructive::yaml;
use relative_path::RelativePathBuf;

use self::cargo::CargoIssue;
use self::ci::ActionExpected;
use crate::ctxt::Ctxt;
use crate::file::File;
use crate::manifest::Manifest;
use crate::model::{Module, ModuleParams, UpdateParams};
use crate::urls::Urls;
use crate::workspace::{Package, Workspace};

pub(crate) enum WorkflowValidation {
    /// Oudated version of an action.
    OutdatedAction {
        actual: String,
        expected: String,
        uses: yaml::ValueId,
    },
    /// Deny use of the specific action.
    DeniedAction { name: String, reason: String },
    /// Actions check failed.
    CustomActionsCheck { name: String, reason: String },
}

pub(crate) enum Validation {
    DeprecatedWorkflow {
        path: RelativePathBuf,
    },
    MissingWorkflow {
        path: RelativePathBuf,
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
        actual: Option<yaml::ValueId>,
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
#[tracing::instrument(skip_all)]
pub(crate) fn build(
    cx: &Ctxt<'_>,
    module: &Module,
    workspace: &Workspace,
    primary_crate: &Package,
    primary_crate_params: ModuleParams<'_>,
    validation: &mut Vec<Validation>,
    urls: &mut Urls,
) -> Result<()> {
    let documentation = match &cx.config.documentation(module) {
        Some(documentation) => Some(documentation.render(&primary_crate_params)?),
        None => None,
    };

    let module_url = module.url.to_string();

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
            cargo::work_cargo_toml(package, validation, &update_params)?;
        }
    }

    if cx.config.is_enabled(&module.path, "ci") {
        ci::build(cx, primary_crate, module, workspace, validation)
            .with_context(|| anyhow!("ci validation: {}", cx.config.job_name(module)))?;
    }

    if cx.config.is_enabled(&module.path, "readme") {
        readme::build(
            cx,
            &module.path,
            module,
            primary_crate,
            primary_crate_params,
            validation,
            urls,
            true,
            false,
        )?;

        for package in workspace.packages() {
            if !package.manifest.is_publish()? {
                continue;
            }

            let variables = cx.config.variables(module);

            let params =
                cx.config
                    .per_crate_render(cx, module, package.crate_params(module)?, &variables);

            readme::build(
                cx,
                &package.manifest_dir,
                module,
                package,
                params,
                validation,
                urls,
                package.manifest_dir != *module.path,
                true,
            )?;
        }
    }

    Ok(())
}
