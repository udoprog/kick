use crate::config::{Distribution, Os};

use anyhow::{bail, Result};

use super::BatchConfig;

/// A run on configuration.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum RunOn {
    /// Run on the same system (default).
    #[default]
    Same,
    /// Run over WSL with the specified distribution.
    Wsl(Distribution),
}

impl RunOn {
    /// Construct a [`RunOn`] from the specified system.
    pub(super) fn from_os(
        batch: &BatchConfig<'_, '_>,
        os: &Os,
        dist: Distribution,
    ) -> Result<RunOn> {
        if batch.cx.os == *os {
            return Ok(RunOn::Same);
        }

        if batch.cx.os == Os::Windows && *os == Os::Linux && !batch.cx.system.wsl.is_empty() {
            return Ok(RunOn::Wsl(dist));
        }

        bail!(
            "No support for {os}/{dist} on current system {}/{}",
            batch.cx.os,
            batch.cx.dist
        );
    }
}
