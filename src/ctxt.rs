use std::cell::{Ref, RefCell};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context, Result};

use crate::actions::Actions;
use crate::config::Config;
use crate::git::Git;
use crate::model::{Module, ModuleParams};
use crate::rust_version::RustVersion;
use crate::validation::Validation;
use crate::workspace::Package;

pub(crate) struct Ctxt<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a Config,
    pub(crate) actions: &'a Actions<'a>,
    pub(crate) modules: &'a [Module],
    pub(crate) github_auth: Option<String>,
    pub(crate) rustc_version: Option<RustVersion>,
    pub(crate) git: Option<Git>,
    pub(crate) validation: RefCell<Vec<Validation>>,
}

impl<'a> Ctxt<'a> {
    /// Get module parameters for the given package.
    pub(crate) fn module_params<'m>(
        &'m self,
        package: &'m Package,
        module: &'m Module,
    ) -> Result<ModuleParams<'m>> {
        let variables = self.config.variables(module);
        let crate_params = package.crate_params(module)?;
        Ok(self
            .config
            .module_params(self, module, crate_params, variables))
    }

    /// Iterate over non-disabled modules.
    pub(crate) fn modules(&self) -> impl Iterator<Item = &Module> + '_ {
        self.modules.iter().filter(move |m| !m.is_disabled())
    }

    /// Require a working git command.
    pub(crate) fn require_git(&self) -> Result<&Git> {
        self.git.as_ref().context("no working git command")
    }

    /// Push a validation.
    pub(crate) fn validation(&self, validation: Validation) {
        self.validation.borrow_mut().push(validation);
    }

    /// Take all proposed validations.
    pub(crate) fn validations(&self) -> Ref<'_, [Validation]> {
        Ref::map(self.validation.borrow(), Vec::as_slice)
    }

    /// Check if there's a validation we can save.
    pub(crate) fn can_save(&self) -> bool {
        let mut can_save = false;

        for validation in self.validation.borrow().iter() {
            can_save |= validation.has_changes();
        }

        can_save
    }
}

/// Minor version from rustc.
pub(crate) fn rustc_version() -> Option<RustVersion> {
    let output = Command::new("rustc")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .ok()?;

    let output = String::from_utf8(output.stdout).ok()?;
    let output = output.trim();
    tracing::trace!("rustc --version: {output}");
    let version = output.split(' ').nth(1)?;
    RustVersion::parse(version)
}
