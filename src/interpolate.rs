//! Interpolation of `~` and environment variables in path-valued
//! configuration.
//!
//! * A leading `~` (the whole value, or followed by `/`) expands to the home
//!   directory. `~user` is not supported and is kept as-is.
//! * `$VAR` and `${VAR}` expand to the value of the environment variable
//!   `VAR`. It is an error if it is not set, it never silently expands to an
//!   empty string.
//! * `${VAR:-default}` expands to `default` if `VAR` is unset or empty. The
//!   default may itself contain interpolations, and is only expanded if it is
//!   used.
//! * `$$` is a literal `$`. Any other use of `$` is an error.

use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

/// An error raised while interpolating a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Error {
    /// A variable referenced without a fallback is not set.
    Unset { name: String },
    /// A variable is not valid UTF-8.
    NotUnicode { name: String },
    /// A `$` which is not followed by a variable name, `{` or `$`.
    Dangling,
    /// A `${` which does not contain a valid variable name.
    InvalidName,
    /// Something other than `}` or `:-` follows the name in `${NAME`.
    InvalidBraced { name: String },
    /// A `${` without a closing `}`.
    Unterminated,
    /// A `~` could not be expanded since the home directory is not known.
    NoHome,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unset { name } => write!(
                f,
                "environment variable `{name}` is not set, use `${{{name}:-default}}` to fall back to a default"
            ),
            Error::NotUnicode { name } => {
                write!(f, "environment variable `{name}` is not valid UTF-8")
            }
            Error::Dangling => write!(
                f,
                "`$` must be followed by a variable name, `{{` or `$`, use `$$` for a literal `$`"
            ),
            Error::InvalidName => write!(f, "`${{` must be followed by a variable name"),
            Error::InvalidBraced { name } => write!(
                f,
                "expected `}}` or `:-` after `${{{name}`, use `${{{name}}}` or `${{{name}:-default}}`"
            ),
            Error::Unterminated => write!(f, "unterminated `${{`, expected a closing `}}`"),
            Error::NoHome => write!(f, "cannot expand `~` since the home directory is not known"),
        }
    }
}

impl std::error::Error for Error {}

/// Look up a variable in the environment of the process.
pub(crate) fn env(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// Interpolate `~` and environment variables in a path-valued setting.
///
/// The returned path is not resolved against anything, so it might be
/// relative.
pub(crate) fn path(
    value: &str,
    home: Option<&Path>,
    lookup: impl Fn(&str) -> Option<OsString>,
) -> Result<PathBuf, Error> {
    let rest = if value == "~" {
        Some("")
    } else {
        value.strip_prefix("~/")
    };

    match rest {
        Some(rest) => {
            let home = home.ok_or(Error::NoHome)?;
            let rest = string(rest, &lookup)?;

            if rest.is_empty() {
                Ok(home.to_owned())
            } else {
                Ok(home.join(rest))
            }
        }
        None => Ok(PathBuf::from(string(value, &lookup)?)),
    }
}

/// Interpolate environment variables in `input`.
pub(crate) fn string(
    input: &str,
    lookup: &impl Fn(&str) -> Option<OsString>,
) -> Result<String, Error> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(n) = rest.find('$') {
        out.push_str(&rest[..n]);
        rest = &rest[n + 1..];

        if let Some(tail) = rest.strip_prefix('$') {
            out.push('$');
            rest = tail;
            continue;
        }

        if let Some(tail) = rest.strip_prefix('{') {
            let (name, tail) = split_name(tail);

            if name.is_empty() {
                return Err(if tail.is_empty() {
                    Error::Unterminated
                } else {
                    Error::InvalidName
                });
            }

            if let Some(tail) = tail.strip_prefix('}') {
                out.push_str(&required(name, lookup)?);
                rest = tail;
                continue;
            }

            if let Some(tail) = tail.strip_prefix(":-") {
                let close = find_close(tail).ok_or(Error::Unterminated)?;

                match get(name, lookup)? {
                    Some(value) if !value.is_empty() => out.push_str(&value),
                    _ => out.push_str(&string(&tail[..close], lookup)?),
                }

                rest = &tail[close + 1..];
                continue;
            }

            if tail.is_empty() {
                return Err(Error::Unterminated);
            }

            return Err(Error::InvalidBraced {
                name: name.to_owned(),
            });
        }

        let (name, tail) = split_name(rest);

        if name.is_empty() {
            return Err(Error::Dangling);
        }

        out.push_str(&required(name, lookup)?);
        rest = tail;
    }

    out.push_str(rest);
    Ok(out)
}

/// Split a leading variable name off of `input`, which is
/// `[A-Za-z_][A-Za-z0-9_]*`.
fn split_name(input: &str) -> (&str, &str) {
    let mut end = 0;

    for (i, b) in input.bytes().enumerate() {
        let valid = b == b'_' || b.is_ascii_alphabetic() || (i > 0 && b.is_ascii_digit());

        if !valid {
            break;
        }

        end = i + 1;
    }

    input.split_at(end)
}

/// Find the `}` closing a default value, skipping over nested `${...}` and
/// `$$` escapes.
fn find_close(input: &str) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;

    while i < bytes.len() {
        match bytes[i] {
            b'$' if matches!(bytes.get(i + 1), Some(b'$' | b'{')) => {
                if bytes[i + 1] == b'{' {
                    depth += 1;
                }

                i += 2;
                continue;
            }
            b'}' if depth == 0 => return Some(i),
            b'}' => depth -= 1,
            _ => {}
        }

        i += 1;
    }

    None
}

fn get(name: &str, lookup: &impl Fn(&str) -> Option<OsString>) -> Result<Option<String>, Error> {
    match lookup(name) {
        Some(value) => match value.into_string() {
            Ok(value) => Ok(Some(value)),
            Err(..) => Err(Error::NotUnicode {
                name: name.to_owned(),
            }),
        },
        None => Ok(None),
    }
}

fn required(name: &str, lookup: &impl Fn(&str) -> Option<OsString>) -> Result<String, Error> {
    get(name, lookup)?.ok_or_else(|| Error::Unset {
        name: name.to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::Path;

    use super::{Error, path, string};

    fn lookup(name: &str) -> Option<OsString> {
        match name {
            "SRC" => Some(OsString::from("/opt/src")),
            "NAME" => Some(OsString::from("kick")),
            "EMPTY" => Some(OsString::new()),
            "TILDE" => Some(OsString::from("~/x")),
            _ => None,
        }
    }

    fn s(input: &str) -> Result<String, Error> {
        string(input, &lookup)
    }

    #[test]
    fn plain() {
        assert_eq!(s("a/b/c").unwrap(), "a/b/c");
        assert_eq!(s("").unwrap(), "");
    }

    #[test]
    fn set_variables() {
        assert_eq!(s("$SRC/a").unwrap(), "/opt/src/a");
        assert_eq!(s("${SRC}/a").unwrap(), "/opt/src/a");
        assert_eq!(s("x-${NAME}-y").unwrap(), "x-kick-y");
        assert_eq!(s("$NAME.toml").unwrap(), "kick.toml");
        assert_eq!(s("$NAME$NAME").unwrap(), "kickkick");
        // A set but empty variable is not an error.
        assert_eq!(s("a${EMPTY}b").unwrap(), "ab");
    }

    #[test]
    fn unset_without_fallback_is_an_error() {
        let unset = Error::Unset {
            name: String::from("MISSING"),
        };

        assert_eq!(s("$MISSING/a"), Err(unset.clone()));
        assert_eq!(s("a/${MISSING}"), Err(unset.clone()));
        assert!(unset.to_string().contains("`MISSING`"), "{unset}");
    }

    #[test]
    fn fallbacks() {
        assert_eq!(s("${MISSING:-/default}/a").unwrap(), "/default/a");
        assert_eq!(s("${EMPTY:-def}").unwrap(), "def");
        assert_eq!(s("${SRC:-def}").unwrap(), "/opt/src");
        assert_eq!(s("${MISSING:-}x").unwrap(), "x");
        // Defaults are interpolated, but only if used.
        assert_eq!(s("${MISSING:-$SRC/x}").unwrap(), "/opt/src/x");
        assert_eq!(s("${MISSING:-${NAME}}").unwrap(), "kick");
        assert_eq!(s("${MISSING:-${OTHER:-z}}").unwrap(), "z");
        assert_eq!(s("${SRC:-$MISSING}").unwrap(), "/opt/src");
        assert_eq!(
            s("${MISSING:-$OTHER}"),
            Err(Error::Unset {
                name: String::from("OTHER")
            })
        );
        assert_eq!(s("${MISSING:-a$$}b").unwrap(), "a$b");
    }

    #[test]
    fn escapes() {
        assert_eq!(s("$$").unwrap(), "$");
        assert_eq!(s("a$$b").unwrap(), "a$b");
        assert_eq!(s("$$NAME").unwrap(), "$NAME");
        assert_eq!(s("$$$NAME").unwrap(), "$kick");
        assert_eq!(s("$${NAME}").unwrap(), "${NAME}");
    }

    #[test]
    fn malformed() {
        assert_eq!(s("a$"), Err(Error::Dangling));
        assert_eq!(s("a$/b"), Err(Error::Dangling));
        assert_eq!(s("$1"), Err(Error::Dangling));
        assert_eq!(s("${"), Err(Error::Unterminated));
        assert_eq!(s("${NAME"), Err(Error::Unterminated));
        assert_eq!(s("${MISSING:-abc"), Err(Error::Unterminated));
        assert_eq!(s("${}"), Err(Error::InvalidName));
        assert_eq!(
            s("${NAME-x}"),
            Err(Error::InvalidBraced {
                name: String::from("NAME")
            })
        );
    }

    #[test]
    fn tilde() {
        let home = Some(Path::new("/home/user"));

        assert_eq!(path("~", home, lookup).unwrap(), Path::new("/home/user"));
        assert_eq!(
            path("~/a/b", home, lookup).unwrap(),
            Path::new("/home/user/a/b")
        );
        assert_eq!(
            path("~/$NAME", home, lookup).unwrap(),
            Path::new("/home/user/kick")
        );
        // Only a leading `~` is expanded.
        assert_eq!(path("a/~/b", home, lookup).unwrap(), Path::new("a/~/b"));
        assert_eq!(path("~user/a", home, lookup).unwrap(), Path::new("~user/a"));
        // `~` is not expanded in the value of a variable.
        assert_eq!(path("$TILDE", home, lookup).unwrap(), Path::new("~/x"));
        assert_eq!(path("~/a", None, lookup), Err(Error::NoHome));
        assert_eq!(path("rel/a", None, lookup).unwrap(), Path::new("rel/a"));
    }
}
