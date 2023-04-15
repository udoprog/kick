use core::fmt;
use std::borrow::Cow;

use serde::de::Error;
use serde::{Deserialize, Serialize, Serializer};

/// First version to support 2018 edition.
pub(crate) const EDITION_2018: RustVersion = RustVersion::new(1, 31, None);
/// First version to support 2021 edition.
pub(crate) const EDITION_2021: RustVersion = RustVersion::new(1, 56, None);
/// Oldest version to support workspaces.
pub(crate) const WORKSPACE: RustVersion = RustVersion::new(1, 12, None);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[non_exhaustive]
pub(crate) struct RustVersion {
    pub(crate) major: u64,
    pub(crate) minor: u64,
    pub(crate) patch: Option<u64>,
}

impl fmt::Display for RustVersion {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(patch) = self.patch {
            write!(f, "{}.{}.{}", self.major, self.minor, patch)
        } else {
            write!(f, "{}.{}", self.major, self.minor)
        }
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

impl<'de> Deserialize<'de> for RustVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let string = Cow::<str>::deserialize(deserializer)?;
        Self::parse(string.as_ref()).ok_or_else(|| D::Error::custom("illegal rust version"))
    }
}

impl RustVersion {
    pub(crate) const fn new(major: u64, minor: u64, patch: Option<u64>) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub(crate) fn parse(string: &str) -> Option<Self> {
        let mut it = string.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next()?.parse().ok()?;
        let patch = it.next().and_then(|n| n.parse().ok());
        Some(RustVersion {
            major,
            minor,
            patch,
        })
    }
}
