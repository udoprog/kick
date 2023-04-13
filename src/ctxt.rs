use std::cell::{Ref, RefCell};
use std::path::Path;
use std::process::Stdio;

use anyhow::{Context, Result};

use crate::actions::Actions;
use crate::changes::Change;
use crate::config::Config;
use crate::git::Git;
use crate::model::{Module, ModuleParams};
use crate::process::Command;
use crate::rust_version::RustVersion;
use crate::sets::Sets;
use crate::workspace::Package;

pub(crate) struct Ctxt<'a> {
    pub(crate) root: &'a Path,
    pub(crate) config: &'a Config,
    pub(crate) actions: &'a Actions<'a>,
    pub(crate) modules: &'a [Module],
    pub(crate) github_auth: Option<String>,
    pub(crate) rustc_version: Option<RustVersion>,
    pub(crate) git: Option<Git>,
    pub(crate) changes: RefCell<Vec<Change>>,
    pub(crate) sets: &'a mut Sets,
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

    /// Push a change.
    pub(crate) fn change(&self, change: Change) {
        self.changes.borrow_mut().push(change);
    }

    /// Get a list of proposed changes.
    pub(crate) fn changes(&self) -> Ref<'_, [Change]> {
        Ref::map(self.changes.borrow(), Vec::as_slice)
    }

    /// Check if there's a change we can save.
    pub(crate) fn can_save(&self) -> bool {
        let mut can_save = false;

        for change in self.changes.borrow().iter() {
            can_save |= change.has_changes();
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
