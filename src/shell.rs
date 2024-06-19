use std::borrow::Cow;

use crate::model::Shell;

/// Escape a string into a bash command.
pub(crate) fn escape(source: &str, flavor: Shell) -> Cow<'_, str> {
    macro_rules! base_pat {
        ($($pat:pat_param)|*) => {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '=' | '/' | ',' | '.' | '+' $(| $pat)*
        }
    }

    let i = 'bail: {
        match flavor {
            Shell::Bash => {
                for (i, c) in source.char_indices() {
                    match c {
                        base_pat!() => continue,
                        _ => break 'bail i,
                    }
                }
            }
            Shell::Powershell => {
                for (i, c) in source.char_indices() {
                    match c {
                        base_pat!('\\' | ':' | '`') => continue,
                        _ => break 'bail i,
                    }
                }
            }
        }

        return Cow::Borrowed(source);
    };

    let mut out = String::with_capacity(source.len() + 2);

    out.push('"');
    out.push_str(&source[..i]);

    let e = flavor.escapes();

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
