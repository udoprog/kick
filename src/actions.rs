use std::collections::HashMap;

/// A single actions check.
pub(crate) trait ActionsCheck {
    fn check(&self, action: &serde_yaml::Mapping) -> Result<(), String>;
}

/// A collection of supported uses.
#[derive(Default)]
pub(crate) struct Actions<'a> {
    latest: HashMap<String, String>,
    deny: HashMap<String, String>,
    checks: HashMap<String, &'a dyn ActionsCheck>,
}

impl<'a> Actions<'a> {
    /// Insert an expected use.
    pub(crate) fn latest(&mut self, name: &str, latest: &str) {
        self.latest.insert(name.into(), latest.into());
    }

    /// Deny the use of an action.
    pub(crate) fn deny(&mut self, name: &str, reason: &str) {
        self.deny.insert(name.into(), reason.into());
    }

    /// Insert an actions check.
    pub(crate) fn check(&mut self, name: &str, check: &'a dyn ActionsCheck) {
        self.checks.insert(name.into(), check);
    }

    /// Get latest required.
    pub(crate) fn get_latest(&self, name: &str) -> Option<&str> {
        self.latest.get(name).map(|s| s.as_str())
    }

    /// Get denied.
    pub(crate) fn get_deny(&self, name: &str) -> Option<&str> {
        self.deny.get(name).map(|s| s.as_str())
    }

    /// Get denied.
    pub(crate) fn get_check(&self, name: &str) -> Option<&dyn ActionsCheck> {
        self.checks.get(name).copied()
    }
}

pub(crate) struct ActionsRsToolchainActionsCheck;

impl ActionsCheck for ActionsRsToolchainActionsCheck {
    fn check(&self, mapping: &serde_yaml::Mapping) -> Result<(), String> {
        let with = match mapping.get("with").and_then(|v| v.as_mapping()) {
            Some(with) => with,
            None => return Err(String::from("missing with")),
        };

        if with.get("toolchain").and_then(|v| v.as_str()).is_none() {
            return Err(String::from("missing toolchain"));
        }

        Ok(())
    }
}
