use std::collections::HashMap;

use anyhow::Result;

use crate::changes::WorkflowError;
use crate::edits::{self, Edits};
use crate::workflows::Step;

/// A single actions check.
pub(crate) trait ActionsCheck {
    fn check(
        &self,
        name: &str,
        action: &Step,
        edits: &mut Edits,
        errors: &mut Vec<WorkflowError>,
    ) -> Result<()>;
}

/// A collection of supported uses.
#[derive(Default)]
pub(crate) struct Actions<'a> {
    latest: HashMap<String, String>,
    deny: HashMap<String, Option<Box<str>>>,
    checks: HashMap<String, &'a dyn ActionsCheck>,
}

impl<'a> Actions<'a> {
    /// Insert an expected use.
    pub(crate) fn latest(&mut self, name: &str, latest: &str) {
        self.latest.insert(name.into(), latest.into());
    }

    /// Deny the use of an action.
    pub(crate) fn deny(&mut self, name: &str, reason: Option<&str>) {
        self.deny.insert(name.into(), reason.map(Box::from));
    }

    /// Insert an actions check.
    pub(crate) fn check(&mut self, name: &str, check: &'a dyn ActionsCheck) {
        self.checks.insert(name.into(), check);
    }

    /// Get latest required.
    pub(crate) fn get_latest(&self, name: &str) -> Option<&str> {
        self.latest.get(name).map(|s| s.as_str())
    }

    /// Test if a crate is denied.
    pub(crate) fn is_denied(&self, name: &str) -> bool {
        self.deny.contains_key(name)
    }

    /// Get deny reason.
    pub(crate) fn get_deny_reason(&self, name: &str) -> Option<&str> {
        self.deny.get(name).and_then(Option::as_deref)
    }

    /// Get denied.
    pub(crate) fn get_check(&self, name: &str) -> Option<&dyn ActionsCheck> {
        self.checks.get(name).copied()
    }
}

pub(crate) struct ActionsRsToolchainActionsCheck;

impl ActionsCheck for ActionsRsToolchainActionsCheck {
    fn check(
        &self,
        name: &str,
        step: &Step,
        edits: &mut Edits,
        errors: &mut Vec<WorkflowError>,
    ) -> Result<()> {
        let Some((uses_id, _)) = &step.uses else {
            errors.push(WorkflowError::Error {
                name: name.to_string(),
                reason: String::from("there are better alternatives"),
            });
            return Ok(());
        };

        let toolchain = if let Some(toolchain) = step.with.get("toolchain") {
            toolchain.as_str()
        } else {
            "stable"
        };

        let toolchain = if !toolchain.starts_with("${{") {
            edits.remove_key(step.id, "With is incorrect", String::from("with"));
            toolchain
        } else {
            edits.insert(
                step.id,
                "Update toolchain",
                String::from("with"),
                edits::Value::Mapping(vec![(
                    String::from("toolchain"),
                    edits::Value::String(toolchain.to_string()),
                )]),
            );

            "master"
        };

        edits.set(
            *uses_id,
            "actions-rs/toolchain has better alternatives",
            edits::Value::String(format!("dtolnay/rust-toolchain@{toolchain}")),
        );

        Ok(())
    }
}
