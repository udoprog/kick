use std::fmt;

/// Architecture of a debian archive.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Architecture {
    /// The 64-bit x86 architecture..
    Amd64,
}

impl fmt::Display for Architecture {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Architecture::Amd64 => write!(f, "amd64"),
        }
    }
}
