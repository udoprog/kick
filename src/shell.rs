use std::borrow::Cow;

use clap::ValueEnum;

macro_rules! base_pat {
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
    /// Escape a string into a bash command.
    pub(crate) fn escape<'a>(&self, source: &'a str) -> Cow<'a, str> {
        let i = 'escape: {
            match *self {
                Shell::Bash => {
                    for (i, c) in source.char_indices() {
                        match c {
                            base_pat!() => continue,
                            _ => break 'escape i,
                        }
                    }
                }
                Shell::Powershell => {
                    for (i, c) in source.char_indices() {
                        match c {
                            base_pat!('\\' | ':' | '`') => continue,
                            _ => break 'escape i,
                        }
                    }
                }
            }

            return Cow::Borrowed(source);
        };

        let mut out = String::with_capacity(source.len() + 2);

        out.push('"');
        out.push_str(&source[..i]);

        let e = self.escapes();

        for c in source[i..].chars() {
            if let Some(ext) = e.escape(c) {
                out.push_str(ext);
                continue;
            }

            out.push(c)
        }

        out.push('"');
        Cow::Owned(out)
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
