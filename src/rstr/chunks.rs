use std::mem::take;

use super::{RStr, START, TAG_END, TAG_START};

/// An iterator over the chunks of a redacted string.
///
/// See [`Redact::chunks`][super::Redact::chunks].
pub(crate) struct Chunks<'a> {
    string: &'a str,
}

impl<'a> Chunks<'a> {
    pub(crate) fn new(string: &'a str) -> Self {
        Self { string }
    }

    /// Get the remaining redacted string.
    pub(crate) fn as_rstr(&self) -> &'a RStr {
        RStr::new(self.string)
    }
}

impl<'a> Iterator for Chunks<'a> {
    type Item = Chunk<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.string.is_empty() {
            return None;
        }

        let (public, redacted) = if let Some(at) = self.string.find(TAG_START) {
            let (public, rest) = self.string.split_at(at);
            let rest = &rest[TAG_START.len()..];

            if let Some(at) = rest.find(TAG_END) {
                let (redacted, string) = rest.split_at(at);
                self.string = &string[TAG_END.len()..];
                (public, redacted)
            } else {
                self.string = "";
                (public, rest)
            }
        } else {
            (take(&mut self.string), "")
        };

        Some(Chunk { public, redacted })
    }
}

/// A chunk of a redacted string.
///
/// See [`Redact::chunks`][super::Redact::chunks].
pub(crate) struct Chunk<'a> {
    public: &'a str,
    redacted: &'a str,
}

impl<'a> Chunk<'a> {
    /// Get the public part of the chunk.
    pub(crate) fn public(&self) -> &'a str {
        self.public
    }

    /// Get the redacted part of the chunk.
    #[inline]
    pub(crate) fn redacted(&self) -> Redacted<'a> {
        Redacted {
            string: self.redacted,
        }
    }
}

/// A redacted string.
pub(crate) struct Redacted<'a> {
    string: &'a str,
}

impl<'a> Redacted<'a> {
    /// Test if the redacted component is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.string.is_empty()
    }
}

impl Iterator for Redacted<'_> {
    type Item = char;

    fn next(&mut self) -> Option<Self::Item> {
        let mut it = self.string.chars();

        let Some(c) = it.next() else {
            self.string = "";
            return None;
        };

        self.string = it.as_str();

        // SAFETY: We know that `c` is an ASCII character in the tag range.
        Some(unsafe { char::from_u32_unchecked((c as u32) - START) })
    }
}
