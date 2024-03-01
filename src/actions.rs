use std::collections::HashMap;

use anyhow::Result;
use nondestructive::yaml;

use crate::changes::WorkflowChange;

/// A single actions check.
pub(crate) trait ActionsCheck {
    fn check(
        &self,
        name: &str,
        action: &yaml::Mapping<'_>,
        change: &mut Vec<WorkflowChange>,
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
        mapping: &yaml::Mapping<'_>,
        change: &mut Vec<WorkflowChange>,
    ) -> Result<()> {
        let Some(uses) = mapping.get("uses") else {
            change.push(WorkflowChange::Error {
                name: name.to_string(),
                reason: String::from("there are better alternatives"),
            });
            return Ok(());
        };

        let toolchain = if let Some(toolchain) = mapping
            .get("with")
            .and_then(|v| v.as_mapping()?.get("toolchain")?.as_str())
        {
            toolchain
        } else {
            "stable"
        };

        let mut remove_keys = Vec::new();
        let mut set_keys = Vec::new();

        let toolchain = if !toolchain.starts_with("${{") {
            remove_keys.push((mapping.id(), String::from("with")));
            toolchain
        } else {
            set_keys.push((
                mapping.id(),
                String::from("with.toolchain"),
                toolchain.to_string(),
            ));
            "master"
        };

        change.push(WorkflowChange::ReplaceString {
            reason: String::from("actions-rs/toolchain has better alternatives"),
            string: format!("dtolnay/rust-toolchain@{toolchain}"),
            value: uses.id(),
            remove_keys,
            set_keys,
        });

        Ok(())
    }
}
