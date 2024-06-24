use std::borrow::Cow;
use std::fmt;

use clap::ValueEnum;

macro_rules! base {
    ($($pat:pat_param)|*) => {
        'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '=' | '/' | ',' | '.' | '+' $(| $pat)*
    }
}

#[derive(Default, Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Shell {
    #[default]
    Bash,
    Powershell,
}

impl Shell {
    /// Perform a command escape.
    pub(crate) fn escape<'a>(&self, source: &'a str) -> Cow<'a, str> {
        let i = 'escape: {
            match *self {
                Shell::Bash => {
                    for (i, c) in source.char_indices() {
                        match c {
                            base!() => continue,
                            _ => break 'escape i,
                        }
                    }
                }
                Shell::Powershell => {
                    for (i, c) in source.char_indices() {
                        match c {
                            base!('\\' | ':' | '`') => continue,
                            _ => break 'escape i,
                        }
                    }
                }
            }

            return Cow::Borrowed(source);
        };

        Cow::Owned(self.inner_escape_string(source, i))
    }

    /// Explicitly perform a string escape.
    pub(crate) fn escape_string(&self, source: &str) -> String {
        self.inner_escape_string(source, 0)
    }

    /// Test if the environment literal needs to be escaped.
    pub(crate) fn is_env_literal(&self, s: &str) -> bool {
        match *self {
            Shell::Bash => s
                .chars()
                .all(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-')),
            Shell::Powershell => s
                .chars()
                .all(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_')),
        }
    }

    fn inner_escape_string(&self, source: &str, i: usize) -> String {
        let e = self.escapes();

        let mut out = String::with_capacity(source.len() + 2);

        out.push('"');
        out.push_str(&source[..i]);

        for c in source[i..].chars() {
            if let Some(ext) = e.escape(c) {
                out.push_str(ext);
                continue;
            }

            out.push(c)
        }

        out.push('"');
        out
    }

    fn escapes(&self) -> &'static Escapes {
        match *self {
            Shell::Bash => &Escapes {
                dollar: "\\$",
                backslash: Some("\\\\"),
                backtick: "\\`",
                double: "\\\"",
                single: "\\'",
                esclamation: "\\!",
                n: "\\n",
                r: "\\r",
                t: "\\t",
            },
            Shell::Powershell => &Escapes {
                dollar: "`$",
                backslash: None,
                backtick: "``",
                double: "`\"",
                single: "`'",
                esclamation: "`!",
                n: "`n",
                r: "`r",
                t: "`t",
            },
        }
    }
}

impl fmt::Display for Shell {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Shell::Bash => write!(f, "bash"),
            Shell::Powershell => write!(f, "powershell"),
        }
    }
}

pub(crate) struct Escapes {
    dollar: &'static str,
    backslash: Option<&'static str>,
    backtick: &'static str,
    double: &'static str,
    single: &'static str,
    esclamation: &'static str,
    n: &'static str,
    r: &'static str,
    t: &'static str,
}

impl Escapes {
    pub(crate) fn escape(&self, c: char) -> Option<&str> {
        match c {
            '$' => Some(self.dollar),
            '\\' => self.backslash,
            '`' => Some(self.backtick),
            '"' => Some(self.double),
            '\'' => Some(self.single),
            '!' => Some(self.esclamation),
            '\n' => Some(self.n),
            '\r' => Some(self.r),
            '\t' => Some(self.t),
            _ => None,
        }
    }
}
