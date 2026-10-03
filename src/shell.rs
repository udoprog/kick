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
    /// Escape a value so that the shell reads it back as a single, literal
    /// word, leaving values which need no quoting as they are.
    pub(crate) fn escape<'a>(&self, source: &'a str) -> Cow<'a, str> {
        let plain = match *self {
            Shell::Bash => source.chars().all(|c| matches!(c, base!())),
            Shell::Powershell => source.chars().all(|c| matches!(c, base!('\\' | ':'))),
        };

        if plain && !source.is_empty() {
            return Cow::Borrowed(source);
        }

        Cow::Owned(self.escape_string(source))
    }

    /// Escape a value so that the shell reads it back as a single, literal
    /// word, always quoting it.
    pub(crate) fn escape_string(&self, source: &str) -> String {
        let mut out = String::with_capacity(source.len() + 2);

        match *self {
            Shell::Bash => {
                // NB: Nothing is special inside single quotes, so the only
                // thing which needs care is the single quote itself, which
                // closes the quotes, adds an escaped quote and reopens them.
                out.push('\'');

                for c in source.chars() {
                    match c {
                        '\'' => out.push_str("'\\''"),
                        c => out.push(c),
                    }
                }

                out.push('\'');
            }
            Shell::Powershell => {
                out.push('"');

                for c in source.chars() {
                    match c {
                        '$' => out.push_str("`$"),
                        '`' => out.push_str("``"),
                        '"' => out.push_str("`\""),
                        '\'' => out.push_str("`'"),
                        '!' => out.push_str("`!"),
                        '\n' => out.push_str("`n"),
                        '\r' => out.push_str("`r"),
                        '\t' => out.push_str("`t"),
                        c => out.push(c),
                    }
                }

                out.push('"');
            }
        }

        out
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

#[cfg(test)]
mod tests {
    use std::process::Command;

    use super::Shell;

    const AWKWARD: &[&str] = &[
        "",
        "plain",
        "a b",
        "it's",
        "'",
        "''",
        "'quoted'",
        "\"double\"",
        "hi!",
        "!!",
        "$HOME",
        "${HOME}",
        "$(echo no)",
        "`echo no`",
        "back\\slash",
        "\\",
        "trailing\\",
        "new\nline",
        "\n",
        "tab\there",
        "carriage\rreturn",
        "glob * ? [a]",
        "~",
        "a;b|c&d<e>f",
        "# comment",
        "unicode: åäö ✓ 日本",
        "mixed 'single' \"double\" $var `cmd` \\ ! \n\t end",
    ];

    /// Run the escaped values through the shell and read back what it passed
    /// to `printf`.
    fn round_trip(program: &str, values: &[String]) -> Vec<String> {
        let mut script = String::from("printf '%s\\0'");

        for value in values {
            script.push(' ');
            script.push_str(value);
        }

        let output = Command::new(program)
            .arg("-c")
            .arg(&script)
            .output()
            .expect("shell to run");

        assert!(
            output.status.success(),
            "{program} failed on {script:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );

        let stdout = String::from_utf8(output.stdout).expect("utf-8 output");
        let mut values = stdout.split('\0').map(str::to_owned).collect::<Vec<_>>();
        assert_eq!(values.pop().as_deref(), Some(""));
        values
    }

    fn check(program: &str) {
        let expected = AWKWARD.iter().map(|s| s.to_string()).collect::<Vec<_>>();

        let escaped = AWKWARD
            .iter()
            .map(|s| Shell::Bash.escape(s).into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            round_trip(program, &escaped),
            expected,
            "escape via {program}"
        );

        let escaped = AWKWARD
            .iter()
            .map(|s| Shell::Bash.escape_string(s))
            .collect::<Vec<_>>();
        assert_eq!(
            round_trip(program, &escaped),
            expected,
            "escape_string via {program}"
        );
    }

    #[test]
    fn bash_round_trip_sh() {
        check("sh");
    }

    #[test]
    fn bash_round_trip_bash() {
        check("bash");
    }

    #[test]
    fn bash_escape() {
        let shell = Shell::Bash;
        assert_eq!(shell.escape("plain/path-1.0"), "plain/path-1.0");
        assert_eq!(shell.escape(""), "''");
        assert_eq!(shell.escape("a b"), "'a b'");
        assert_eq!(shell.escape("it's"), r"'it'\''s'");
        assert_eq!(shell.escape("hi!"), "'hi!'");
        assert_eq!(shell.escape("$HOME"), "'$HOME'");
        assert_eq!(shell.escape("a\tb"), "'a\tb'");
        assert_eq!(shell.escape_string("plain"), "'plain'");
    }

    #[test]
    fn powershell_escape() {
        let shell = Shell::Powershell;
        assert_eq!(shell.escape(r"C:\dir\file-1.0"), r"C:\dir\file-1.0");
        assert_eq!(shell.escape(""), "\"\"");
        assert_eq!(shell.escape("a b"), "\"a b\"");
        assert_eq!(shell.escape("a`b"), "\"a``b\"");
        assert_eq!(shell.escape("$HOME"), "\"`$HOME\"");
        assert_eq!(shell.escape("say \"hi\""), "\"say `\"hi`\"\"");
        assert_eq!(shell.escape("it's"), "\"it`'s\"");
        assert_eq!(shell.escape_string("plain"), "\"plain\"");
        assert_eq!(shell.escape_string(""), "\"\"");
        assert_eq!(shell.escape_string("`"), "\"``\"");
        assert_eq!(shell.escape_string(r"C:\x"), "\"C:\\x\"");
    }

    /// Round-trip through PowerShell, skipping when `pwsh` is not installed.
    #[test]
    fn powershell_round_trip() {
        let script_head = "[Console]::OutputEncoding = [Text.Encoding]::UTF8; \
            function p { foreach ($a in $args) { [Console]::Out.Write($a + [char]0) } }; p";

        for escape_all in [false, true] {
            let mut script = String::from(script_head);

            for value in AWKWARD {
                script.push(' ');

                if escape_all {
                    script.push_str(&Shell::Powershell.escape_string(value));
                } else {
                    script.push_str(&Shell::Powershell.escape(value));
                }
            }

            let output = match Command::new("pwsh")
                .args(["-NoProfile", "-NonInteractive", "-Command"])
                .arg(&script)
                .output()
            {
                Ok(output) => output,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    eprintln!("skipping: pwsh is not installed");
                    return;
                }
                Err(e) => panic!("pwsh failed to run: {e}"),
            };

            assert!(
                output.status.success(),
                "pwsh failed on {script:?}: {}",
                String::from_utf8_lossy(&output.stderr)
            );

            let stdout = String::from_utf8(output.stdout).expect("utf-8 output");
            let mut got = stdout.split('\0').map(str::to_owned).collect::<Vec<_>>();
            assert_eq!(got.pop().as_deref(), Some(""));
            assert_eq!(got, AWKWARD, "escape_all = {escape_all}");
        }
    }
}
