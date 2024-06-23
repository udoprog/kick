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
        if batch.cx.current_os == *os && batch.cx.dist.matches(dist) {
            return Ok(RunOn::Same);
        }

        if batch.cx.current_os == Os::Windows
            && *os == Os::Linux
            && batch.cx.system.wsl.first().is_some()
        {
            return Ok(RunOn::Wsl(dist));
        }

        bail!(
            "No support for {os}/{dist} on current system {}/{}",
            batch.cx.current_os,
            batch.cx.dist
        );
    }
}
