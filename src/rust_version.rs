use core::fmt;

use serde::{Serialize, Serializer};

/// First version to support 2018 edition.
pub(crate) const EDITION_2018: RustVersion = RustVersion::new(1, 31);
/// First version to support 2021 edition.
pub(crate) const EDITION_2021: RustVersion = RustVersion::new(1, 56);
/// Oldest version to support workspaces.
pub(crate) const WORKSPACE: RustVersion = RustVersion::new(1, 12);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub(crate) struct RustVersion {
    pub(crate) major: u64,
    pub(crate) minor: u64,
}

impl fmt::Display for RustVersion {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl Serialize for RustVersion {
    #[inline]
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.collect_str(self)
    }
}

impl RustVersion {
    pub(crate) const fn new(major: u64, minor: u64) -> Self {
        Self { major, minor }
    }

    pub(crate) fn parse(string: &str) -> Option<Self> {
        let mut it = string.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next()?.parse().ok()?;
        Some(RustVersion { major, minor })
    }
}
