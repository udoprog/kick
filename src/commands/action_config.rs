use std::collections::BTreeMap;

use anyhow::Result;

use crate::config::Os;
use crate::rstr::{RStr, RString};

use super::{Batch, BatchConfig};

/// An actions configuration.
pub(crate) struct ActionConfig<'a> {
    os: &'a Os,
    id: Option<Box<RStr>>,
    action_name: Option<Box<RStr>>,
    skipped: Option<String>,
    inputs: BTreeMap<String, RString>,
}

impl<'a> ActionConfig<'a> {
    /// Construct a new action configuration.
    pub(crate) fn new(os: &'a Os) -> Self {
        Self {
            os,
            action_name: None,
            id: None,
            skipped: None,
            inputs: BTreeMap::new(),
        }
    }

    /// Get the os of the action.
    pub(super) fn os(&self) -> &Os {
        self.os
    }

    /// Get the id of the action.
    pub(crate) fn id(&self) -> Option<&RStr> {
        self.id.as_deref()
    }

    /// Get the name of the action being configured.
    pub(crate) fn action_name(&self) -> Option<&RStr> {
        self.action_name.as_deref()
    }

    /// Get the skipped config.
    pub(crate) fn skipped(&self) -> Option<&str> {
        self.skipped.as_deref()
    }

    /// Get the input variables for runner.
    pub(crate) fn inputs(&self) -> impl Iterator<Item = (&str, &RStr)> {
        self.inputs.iter().map(|(k, v)| (k.as_str(), v.as_rstr()))
    }

    /// Set the id of the action.
    pub(crate) fn with_id<S>(mut self, id: Option<S>) -> Self
    where
        S: AsRef<RStr>,
    {
        self.id = id.map(|s| s.as_ref().into());
        self
    }

    /// Set the name of the action.
    pub(crate) fn with_action_name<S>(mut self, action_name: S) -> Self
    where
        S: AsRef<RStr>,
    {
        self.action_name = Some(action_name.as_ref().into());
        self
    }

    /// Set the skipped status of the action.
    pub(crate) fn with_skipped<S>(mut self, skipped: Option<S>) -> Self
    where
        S: AsRef<RStr>,
    {
        self.skipped = skipped.map(|s| s.as_ref().to_string_lossy().into_owned());
        self
    }

    /// Set inputs variables for runner.
    pub(crate) fn with_inputs(mut self, inputs: BTreeMap<String, RString>) -> Self {
        self.inputs = inputs;
        self
    }

    /// Construct a new use batch.
    pub(crate) fn new_use_batch(
        &self,
        batch: &BatchConfig<'_, '_>,
        id: impl AsRef<RStr>,
    ) -> Result<Batch> {
        Batch::with_use(batch, self, id)
    }
}