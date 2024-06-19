use std::fmt;
use std::path::PathBuf;
use std::str;

#[derive(Debug)]
#[cfg_attr(test, derive(PartialEq))]
pub(crate) struct NodeVersion {
    pub(crate) major: u32,
    pub(crate) minor: u32,
    pub(crate) patch: u32,
}

impl NodeVersion {
    pub(super) fn parse<S>(s: S) -> Option<NodeVersion>
    where
        S: AsRef<str>,
    {
        let s = s.as_ref();
        let s = s.trim();

        let mut it = s.strip_prefix('v').unwrap_or(s).split('.');

        let major = it.next()?.parse().ok()?;
        let minor = it.next()?.parse().ok()?;
        let patch = it.next()?.parse().ok()?;

        Some(NodeVersion {
            major,
            minor,
            patch,
        })
    }
}

impl fmt::Display for NodeVersion {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug)]
pub(crate) struct Node {
    pub(crate) path: PathBuf,
    pub(crate) version: NodeVersion,
}

impl Node {
    #[inline]
    pub(crate) fn new(path: PathBuf, version: NodeVersion) -> Self {
        Self { path, version }
    }
}

#[test]
fn node_version() {
    assert_eq!(
        NodeVersion::parse("v14.15.4"),
        Some(NodeVersion {
            major: 14,
            minor: 15,
            patch: 4
        })
    );

    assert_eq!(
        NodeVersion::parse("14.15.4"),
        Some(NodeVersion {
            major: 14,
            minor: 15,
            patch: 4
        })
    );
}
