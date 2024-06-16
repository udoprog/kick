use std::borrow::Cow;

use crate::model::ShellFlavor;

/// Escape a string into a bash command.
pub(crate) fn escape(source: &str, flavor: ShellFlavor) -> Cow<'_, str> {
    let i = 'bail: {
        for (i, c) in source.char_indices() {
            match c {
                'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '=' | '/' | ',' | '.' | '+' => {
                    continue
                }
                _ => break 'bail i,
            }
        }

        return Cow::Borrowed(source);
    };

    let mut out = String::with_capacity(source.len() + 2);

    out.push('"');
    out.push_str(&source[..i]);

    let (nl, tab, cr) = match flavor {
        ShellFlavor::Sh => ("\\n", "\\t", "\\r"),
        ShellFlavor::Powershell => ("`n", "`t", "`r"),
    };

    for c in source[i..].chars() {
        match c {
            '$' => out.push_str("\\$"),
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '!' => out.push_str("\\!"),
            '\n' => out.push_str(nl),
            '\r' => out.push_str(cr),
            '\t' => out.push_str(tab),
            _ => out.push(c),
        }
    }

    out.push('"');
    Cow::Owned(out)
}
