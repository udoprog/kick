mod chunks;
pub(crate) use self::chunks::Chunks;

#[cfg(test)]
mod tests;

use core::cmp::Ordering;
use core::fmt;
use core::ops::Deref;
use std::borrow::{Borrow, Cow};
use std::hash::{Hash, Hasher};

// The start of the Unicode tag sequence.
//
// While the sequence is deprecated, it's frequently treated as markup and
// ignored when printed.
const START: u32 = 0xE0000;
const TAG_START: &str = "\u{E0001}";
const TAG_END: &str = "\u{E007F}";

/// A borrowed string which might contain redacted sequences.
///
/// Trying to format the string will result in those redacted sequences being
/// marked as `***`.
#[repr(transparent)]
pub(crate) struct Redact(str);

impl Redact {
    /// Construct a new redacted string wrapping the given string.
    pub(crate) fn new<S>(s: &S) -> &Self
    where
        S: ?Sized + AsRef<str>,
    {
        // This is safe because `Redacted` is a transparent wrapper around `str`.
        unsafe { &*(s.as_ref() as *const str as *const Redact) }
    }

    /// Check if the redacted string is empty.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Iterate over chunks of the redacted string.
    pub(crate) fn chunks(&self) -> Chunks<'_> {
        Chunks::new(&self.0)
    }

    /// Split the redacted string oncoe over the given string.
    pub(crate) fn split_once(&self, c: char) -> Option<(&Redact, &Redact)> {
        let (a, b) = self.0.split_once(c)?;
        Some((Redact::new(a), Redact::new(b)))
    }

    /// Get the raw underlying string.
    ///
    /// This should not be used to display the string, as it will contain the
    /// redacted string even though it is encoded.
    pub(crate) fn as_raw(&self) -> &str {
        &self.0
    }

    /// Coerce into the interior string, removing the redaction markup.
    pub(crate) fn to_redacted(&self) -> Cow<'_, str> {
        let Some(until) = self.0.find(TAG_START) else {
            return Cow::Borrowed(&self.0);
        };

        let mut out = String::with_capacity(self.0.len());
        let (head, tail) = self.0.split_at(until);
        out.push_str(head);

        for chunk in Redact::new(tail).chunks() {
            out.push_str(chunk.public());
            out.extend(chunk.redacted());
        }

        Cow::Owned(out)
    }
}

impl AsRef<Redact> for Redact {
    #[inline]
    fn as_ref(&self) -> &Redact {
        self
    }
}

impl fmt::Display for Redact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.chunks() {
            f.write_str(chunk.public())?;

            if chunk.redacted().next().is_some() {
                f.write_str("***")?;
            }
        }

        Ok(())
    }
}

impl fmt::Debug for Redact {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"")?;

        for chunk in self.chunks() {
            f.write_str(chunk.public())?;

            if chunk.redacted().next().is_some() {
                f.write_str("***")?;
            }
        }

        write!(f, "\"")?;
        Ok(())
    }
}

/// A string which might contain redacted components.
///
/// Redacted components are marked with special tags, any formatting of this
/// string will cause them to show up as `***`.
///
/// To access the raw underlying string, you should use [`to_redacted`].
/// Alternatively you can iterate over the chunks of the string using
/// [`chunks`].
///
/// [`to_redacted`]: Redact::to_redacted
/// [`chunks`]: Redact::chunks
#[derive(Clone)]
pub(crate) struct OwnedRedact(String);

impl OwnedRedact {
    /// Construct a new empty redacted string.
    pub(crate) const fn new() -> Self {
        OwnedRedact(String::new())
    }
    /// Construct a new empty redacted string with the given `capacity`.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        OwnedRedact(String::with_capacity(capacity))
    }

    /// Get a reference to the [`Redact`] value corresponding to this instance.
    pub(crate) fn as_redact(&self) -> &Redact {
        self
    }

    /// Construct a new redacted string. This can only contain ascii characters.
    pub(crate) fn redacted<S>(s: S) -> Option<Self>
    where
        S: AsRef<str>,
    {
        let s = s.as_ref();
        let mut out = Self::with_capacity(s.len() + 2);

        if !out.push_redacted(s.as_ref()) {
            return None;
        }

        Some(out)
    }

    /// Push another raw non-redacted char.
    pub(crate) fn push(&mut self, c: char) {
        self.0.push(c);
    }

    /// Push another raw non-redacted string.
    pub(crate) fn push_str(&mut self, s: &str) {
        self.0.push_str(s);
    }

    /// Push a redacted string.
    pub(crate) fn push_redacted(&mut self, s: &str) -> bool {
        self.0.push_str(TAG_START);

        for c in s.chars() {
            if !c.is_ascii() || c.is_ascii_control() {
                return false;
            }

            // SAFETY: We know that `c` is an ASCII character.
            self.0
                .push(unsafe { char::from_u32_unchecked(c as u32 + START) });
        }

        self.0.push_str(TAG_END);
        true
    }
}

impl From<String> for OwnedRedact {
    #[inline]
    fn from(value: String) -> Self {
        OwnedRedact(value)
    }
}

impl From<&String> for OwnedRedact {
    #[inline]
    fn from(value: &String) -> Self {
        OwnedRedact(value.clone())
    }
}

impl From<&str> for OwnedRedact {
    #[inline]
    fn from(value: &str) -> Self {
        OwnedRedact(value.into())
    }
}

impl From<&Redact> for OwnedRedact {
    #[inline]
    fn from(value: &Redact) -> Self {
        value.to_owned()
    }
}

impl Deref for OwnedRedact {
    type Target = Redact;

    #[inline]
    fn deref(&self) -> &Self::Target {
        Redact::new(&self.0)
    }
}

impl ToOwned for Redact {
    type Owned = OwnedRedact;

    #[inline]
    fn to_owned(&self) -> Self::Owned {
        OwnedRedact(self.0.to_owned())
    }
}

impl Borrow<Redact> for OwnedRedact {
    #[inline]
    fn borrow(&self) -> &Redact {
        self
    }
}

impl AsRef<Redact> for OwnedRedact {
    #[inline]
    fn as_ref(&self) -> &Redact {
        self
    }
}

impl fmt::Display for OwnedRedact {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl fmt::Debug for OwnedRedact {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl fmt::Write for OwnedRedact {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s);
        Ok(())
    }
}

impl AsRef<Redact> for str {
    #[inline]
    fn as_ref(&self) -> &Redact {
        Redact::new(self)
    }
}

impl AsRef<Redact> for String {
    #[inline]
    fn as_ref(&self) -> &Redact {
        Redact::new(self.as_str())
    }
}

macro_rules! cmp {
    ($ty:ty) => {
        impl Hash for $ty {
            fn hash<H>(&self, state: &mut H)
            where
                H: Hasher,
            {
                self.0.hash(state);
            }
        }

        impl PartialEq for $ty {
            #[inline]
            fn eq(&self, other: &Self) -> bool {
                self.0 == other.0
            }
        }

        impl PartialEq<str> for $ty {
            #[inline]
            fn eq(&self, other: &str) -> bool {
                self.0 == *other
            }
        }

        impl PartialEq<$ty> for str {
            #[inline]
            fn eq(&self, other: &$ty) -> bool {
                *self == other.0
            }
        }

        impl PartialEq<&str> for $ty {
            #[inline]
            fn eq(&self, other: &&str) -> bool {
                self.0 == **other
            }
        }

        impl PartialEq<$ty> for &str {
            #[inline]
            fn eq(&self, other: &$ty) -> bool {
                **self == other.0
            }
        }

        impl Eq for $ty {}

        impl PartialOrd for $ty {
            #[inline]
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.0.cmp(&other.0))
            }
        }

        impl Ord for $ty {
            #[inline]
            fn cmp(&self, other: &Self) -> Ordering {
                self.0.cmp(&other.0)
            }
        }
    };
}

cmp!(Redact);
cmp!(OwnedRedact);
