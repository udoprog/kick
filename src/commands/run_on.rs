use crate::config::{Distribution, Os};
use crate::ctxt::Ctxt;

use anyhow::{bail, Result};

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
    pub(super) fn from_os(cx: &Ctxt<'_>, os: &Os, dist: Distribution) -> Result<RunOn> {
        if cx.os == *os && cx.dist.matches(dist) {
            return Ok(RunOn::Same);
        }

        if cx.os == Os::Windows && *os == Os::Linux && cx.system.wsl.first().is_some() {
            return Ok(RunOn::Wsl(dist));
        }

        bail!("No support for {os:?} on current system {:?}", cx.os);
    }
}
