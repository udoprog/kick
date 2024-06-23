mod chunks;
pub(crate) use self::chunks::Chunks;

#[cfg(test)]
mod tests;

use std::borrow::{Borrow, Cow};
use std::cmp::Ordering;
use std::fmt;
#[cfg(test)]
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::rc::Rc;

/// The redaction string.
const REDACTION: &str = "***";

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
pub(crate) struct RStr(str);

impl RStr {
    /// Construct a new redacted string wrapping the given string.
    pub(crate) fn new<S>(s: &S) -> &Self
    where
        S: ?Sized + AsRef<str>,
    {
        // This is safe because `Redacted` is a transparent wrapper around `str`.
        unsafe { &*(s.as_ref() as *const str as *const RStr) }
    }

    /// Construct a reference counted container for an RStr.
    pub(crate) fn as_rc(&self) -> Rc<Self> {
        Self::from_rc(Rc::from(&self.0))
    }

    /// Construct a new redacted string wrapping the given string.
    pub(crate) fn from_rc(value: Rc<str>) -> Rc<Self> {
        // SAFETY: RStr is a transparent wrapper around str.
        unsafe { Rc::from_raw(Rc::into_raw(value) as *mut RStr) }
    }

    /// Get the string as a lossy string.
    pub(crate) fn to_string_lossy(&self) -> Cow<'_, str> {
        let mut chunks = self.chunks();

        let Some(chunk) = chunks.next() else {
            return Cow::Borrowed("");
        };

        if chunk.redacted().is_empty() {
            return Cow::Borrowed(chunk.public());
        };

        let mut out = String::with_capacity(self.0.len());

        out.push_str(chunk.public());
        out.push_str(REDACTION);

        for chunk in chunks {
            out.push_str(chunk.public());

            if !chunk.redacted().is_empty() {
                out.push_str(REDACTION);
            }
        }

        Cow::Owned(out)
    }

    /// Get the exposed string which contains any redacted components in clear
    /// text.
    pub(crate) fn to_exposed(&self) -> Cow<'_, str> {
        let mut chunks = self.chunks();

        let Some(chunk) = chunks.next() else {
            return Cow::Borrowed("");
        };

        let redacted = chunk.redacted();

        if redacted.is_empty() {
            return Cow::Borrowed(chunk.public());
        };

        let mut out = String::with_capacity(self.0.len());

        out.push_str(chunk.public());
        out.extend(redacted);

        for chunk in chunks {
            out.push_str(chunk.public());
            out.extend(chunk.redacted());
        }

        Cow::Owned(out)
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
    pub(crate) fn split_once(&self, c: char) -> Option<(&RStr, &RStr)> {
        let (a, b) = self.0.split_once(c)?;
        Some((RStr::new(a), RStr::new(b)))
    }

    /// Test if the exposed content of this string equals the exposed other
    /// string.
    pub(crate) fn exposed_eq(&self, other: &Self) -> bool {
        self.exposed_chars().eq(other.exposed_chars())
    }

    /// Test if the exposed content of this string equals the exposed other
    /// string.
    pub(crate) fn exposed_cmp(&self, other: &Self) -> Ordering {
        self.exposed_chars().cmp(other.exposed_chars())
    }

    /// Test if the exposed portion of this string equals the other string.
    pub(crate) fn str_eq(&self, other: impl AsRef<str>) -> bool {
        let other = other.as_ref();
        let mut it = self.chunks();

        let Some(first) = it.next() else {
            return other.is_empty();
        };

        let Some(rest) = other.strip_prefix(first.public()) else {
            return false;
        };

        first
            .redacted()
            .chain(it.as_rstr().exposed_chars())
            .eq(rest.chars())
    }

    /// Get the raw underlying string.
    ///
    /// This should not be used to display the string, as it will contain the
    /// redacted string even though it is encoded.
    pub(crate) fn as_raw(&self) -> &str {
        &self.0
    }

    /// Iterate over all exposed characters.
    fn exposed_chars(&self) -> impl Iterator<Item = char> + '_ {
        self.chunks()
            .flat_map(|chunk| chunk.public().chars().chain(chunk.redacted()))
    }
}

impl AsRef<RStr> for RStr {
    #[inline]
    fn as_ref(&self) -> &RStr {
        self
    }
}

impl fmt::Display for RStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for chunk in self.chunks() {
            f.write_str(chunk.public())?;

            if !chunk.redacted().is_empty() {
                f.write_str(REDACTION)?;
            }
        }

        Ok(())
    }
}

impl fmt::Debug for RStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "\"")?;

        for chunk in self.chunks() {
            f.write_str(chunk.public())?;

            if !chunk.redacted().is_empty() {
                f.write_str(REDACTION)?;
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
#[repr(transparent)]
pub(crate) struct RString(String);

impl RString {
    /// Construct a new empty redacted string.
    pub(crate) const fn new() -> Self {
        RString(String::new())
    }

    /// Construct a new empty redacted string with the given `capacity`.
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        RString(String::with_capacity(capacity))
    }

    /// Get a reference to the [`Redact`] value corresponding to this instance.
    pub(crate) fn as_rstr(&self) -> &RStr {
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

impl From<String> for RString {
    #[inline]
    fn from(value: String) -> Self {
        RString(value)
    }
}

impl From<&String> for RString {
    #[inline]
    fn from(value: &String) -> Self {
        RString(value.clone())
    }
}

impl From<&str> for RString {
    #[inline]
    fn from(value: &str) -> Self {
        RString(value.into())
    }
}

impl From<&RStr> for RString {
    #[inline]
    fn from(value: &RStr) -> Self {
        value.to_owned()
    }
}

impl Deref for RString {
    type Target = RStr;

    #[inline]
    fn deref(&self) -> &Self::Target {
        RStr::new(&self.0)
    }
}

impl ToOwned for RStr {
    type Owned = RString;

    #[inline]
    fn to_owned(&self) -> Self::Owned {
        RString(self.0.to_owned())
    }
}

impl Borrow<RStr> for RString {
    #[inline]
    fn borrow(&self) -> &RStr {
        self
    }
}

impl AsRef<RStr> for RString {
    #[inline]
    fn as_ref(&self) -> &RStr {
        self
    }
}

impl fmt::Display for RString {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl fmt::Debug for RString {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl fmt::Write for RString {
    #[inline]
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.push_str(s);
        Ok(())
    }
}

impl AsRef<RStr> for str {
    #[inline]
    fn as_ref(&self) -> &RStr {
        RStr::new(self)
    }
}

impl AsRef<RStr> for String {
    #[inline]
    fn as_ref(&self) -> &RStr {
        RStr::new(self.as_str())
    }
}

macro_rules! cmp {
    ($ty:ty) => {
        #[cfg(test)]
        impl Hash for $ty {
            fn hash<H>(&self, state: &mut H)
            where
                H: Hasher,
            {
                0xffu32.hash(state);

                for c in self.exposed_chars() {
                    c.hash(state);
                }
            }
        }

        #[cfg(test)]
        impl PartialEq for $ty {
            #[inline]
            fn eq(&self, other: &Self) -> bool {
                self.exposed_eq(&other)
            }
        }

        #[cfg(test)]
        impl Eq for $ty {}

        #[cfg(test)]
        impl PartialOrd for $ty {
            #[inline]
            fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
                Some(self.cmp(other))
            }
        }

        #[cfg(test)]
        impl Ord for $ty {
            #[inline]
            fn cmp(&self, other: &Self) -> Ordering {
                self.exposed_cmp(other)
            }
        }
    };
}

cmp!(RStr);
cmp!(RString);

impl From<Box<str>> for Box<RStr> {
    #[inline]
    fn from(value: Box<str>) -> Self {
        unsafe { Box::from_raw(Box::into_raw(value) as *mut RStr) }
    }
}

impl From<&str> for Box<RStr> {
    #[inline]
    fn from(value: &str) -> Self {
        Self::from(Box::<str>::from(value))
    }
}

impl From<&RStr> for Box<RStr> {
    #[inline]
    fn from(value: &RStr) -> Self {
        Self::from(&value.0)
    }
}

impl From<&String> for Box<RStr> {
    #[inline]
    fn from(value: &String) -> Self {
        Self::from(value.as_str())
    }
}

impl From<String> for Box<RStr> {
    #[inline]
    fn from(value: String) -> Self {
        Self::from(Box::<str>::from(value))
    }
}

impl From<RString> for Box<RStr> {
    #[inline]
    fn from(value: RString) -> Self {
        Self::from(value.0)
    }
}

impl From<&RString> for Box<RStr> {
    #[inline]
    fn from(value: &RString) -> Self {
        Self::from(value.as_rstr())
    }
}

impl Clone for Box<RStr> {
    #[inline]
    fn clone(&self) -> Self {
        Box::from(self.as_ref())
    }
}
