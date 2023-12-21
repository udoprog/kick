use std::collections::HashMap;
use std::str;

use anyhow::{bail, Context, Result};

use super::{Date, Name, SemanticVersion, Version, VersionKind};

const EOF: char = '\0';

macro_rules! expect {
    ($slf:expr, $pat:pat) => {{
        let b = $slf.peek();

        if !matches!(b, $pat) {
            bail!("Expected {}, but got '{}'", stringify!($pat), b);
        }

        $slf.next();
        b
    }};
}

macro_rules! ws {
    () => {
        ' ' | '\t' | '\n' | '\r'
    };
}

pub(super) struct Vars<'a> {
    today: Date,
    values: HashMap<&'a str, &'a str>,
}

impl<'a> Vars<'a> {
    pub(super) fn new(today: Date) -> Self {
        Vars {
            today,
            values: HashMap::new(),
        }
    }

    fn get(&self, name: &str) -> Option<&'a str> {
        self.values.get(name).copied()
    }

    pub(super) fn insert(&mut self, name: &'a str, value: &'a str) {
        self.values.insert(name, value);
    }
}

pub(super) fn expr<'a>(input: &'a str, vars: &Vars<'a>) -> Result<Option<Version<'a>>> {
    let mut parser = Parser::new(input, vars);
    parser.expr()
}

#[cfg(test)]
fn parse<'a>(input: &'a str, vars: &'a Vars) -> Result<Option<Version<'a>>> {
    let mut parser = Parser::new(input, vars);
    parser.release()
}

struct Parser<'vars, 'a> {
    data: &'a str,
    vars: &'vars Vars<'a>,
    index: usize,
}

impl<'vars, 'a> Parser<'vars, 'a> {
    fn new(data: &'a str, vars: &'vars Vars<'a>) -> Self {
        Parser {
            data,
            vars,
            index: 0,
        }
    }

    fn peek(&mut self) -> char {
        let Some(s) = self.data.get(self.index..) else {
            return EOF;
        };

        let mut it = s.chars();
        it.next().unwrap_or(EOF)
    }

    fn peek2(&mut self) -> (char, char) {
        let Some(s) = self.data.get(self.index..) else {
            return (EOF, EOF);
        };

        let mut it = s.chars();

        let Some(a) = it.next() else {
            return (EOF, EOF);
        };

        let Some(b) = it.next() else {
            return (a, EOF);
        };

        (a, b)
    }

    fn next(&mut self) -> char {
        let b = self.peek();

        if b != EOF {
            self.index += b.len_utf8();
        }

        b
    }

    fn version(&mut self) -> Result<SemanticVersion<'a>> {
        let start = self.index;

        let major = self.number()?.context("Expected major version")?;
        expect!(self, '.');
        let minor = self.number()?.context("Expected minor version")?;

        let (end, patch) = if self.peek() == '.' {
            let at = self.index;
            self.next();

            match self.number()? {
                Some(patch) => (self.index, Some(patch)),
                None => (at, None),
            }
        } else {
            (self.index, None)
        };

        Ok(SemanticVersion {
            original: &self.data[start..end],
            major,
            minor,
            patch,
        })
    }

    fn date(&mut self) -> Result<Date> {
        let year = self.number()?.context("Expected year")?;

        let Ok(year) = i32::try_from(year) else {
            bail!("Year is out of range");
        };

        expect!(self, '-');
        let month = self.number()?.context("Expected month")?;
        expect!(self, '-');
        let day = self.number()?.context("Expected day")?;
        Date::new(year, month, day)
    }

    fn channel(&mut self, start: usize) -> Result<Name<'a>> {
        let name = self.channel_ident(start)?;
        let number = self.number()?;
        Ok(Name { name, number })
    }

    fn variable(&mut self) -> Result<&'a str> {
        if self.peek() != '{' {
            return self.ident(self.index);
        }

        self.next();
        let start = self.index;

        while matches!(self.peek(), 'a'..='z' | '0'..='9' | '-' | '_' | '.') {
            self.next();
        }

        let end = self.index;
        expect!(self, '}');
        Ok(&self.data[start..end])
    }

    fn channel_ident(&mut self, start: usize) -> Result<&'a str> {
        while self.peek().is_ascii_lowercase() {
            self.next();
        }

        if start == self.index {
            bail!("Identifier cannot be empty at {}", start);
        }

        let end = self.index;
        Ok(&self.data[start..end])
    }

    fn ident(&mut self, start: usize) -> Result<&'a str> {
        expect!(self, 'a'..='z');

        while let 'a'..='z' | '0'..='9' = self.peek() {
            self.next();
        }

        if start == self.index {
            bail!("Identifier cannot be empty at {}", start);
        }

        let end = self.index;
        Ok(&self.data[start..end])
    }

    fn number(&mut self) -> Result<Option<u32>> {
        let start = self.index;

        // NB: Ignore zero-prefixing.
        if self.peek() == '0' {
            while self.peek() == '0' {
                self.next();
            }
        } else if !matches!(self.peek(), '1'..='9') {
            return Ok(None);
        };

        Ok(Some(self.number_rem(start)?))
    }

    fn number_rem(&mut self, start: usize) -> Result<u32> {
        let mut number = 0u32;

        while let digit @ '0'..='9' = self.peek() {
            self.next();

            let Some(update) = number.checked_mul(10) else {
                bail!("Numerical overflow at '{}'", &self.data[start..self.index]);
            };

            number = update + (digit as u8 - b'0') as u32;
        }

        Ok(number)
    }

    fn maybe_channel(&mut self) -> Result<Option<Name<'a>>> {
        if self.peek() == '-' {
            self.next();

            if matches!(self.peek(), EOF | ws!()) {
                return Ok(None);
            }

            Ok(Some(self.channel(self.index)?))
        } else {
            Ok(None)
        }
    }

    fn release(&mut self) -> Result<Option<Version<'a>>> {
        let start = self.index;

        while self.peek().is_ascii_lowercase() {
            self.next();
        }

        let mut prefix = if self.index != start {
            Some((start, &self.data[start..self.index]))
        } else {
            None
        };

        let mut release = 'release: {
            let kind = 'kind: {
                match self.peek() {
                    '%' => {
                        self.next();

                        match self.variable()? {
                            "date" => break 'kind VersionKind::Date(self.vars.today),
                            other => {
                                let Some(value) = self.vars.get(other) else {
                                    break 'release None;
                                };

                                let mut parser = Parser::new(value, self.vars);
                                break 'release parser.release()?;
                            }
                        }
                    }
                    '0'..='9' => {
                        let stored = self.index;

                        if let Ok(version) = self.version() {
                            break 'kind VersionKind::SemanticVersion(version);
                        }

                        self.index = stored;

                        if let Ok(date) = self.date() {
                            break 'kind VersionKind::Date(date);
                        }

                        self.index = stored;
                    }
                    _ => {}
                }

                let Some((start, ..)) = prefix.take() else {
                    bail!(
                        "Expected valid version or date at '{}'",
                        &self.data[self.index..],
                    );
                };

                let name = &self.data[start..self.index];
                let number = self.number()?;

                VersionKind::Name(Name { name, number })
            };

            Some(Version {
                prefix: prefix.take().map(|(_, prefix)| prefix),
                kind,
                pre: None,
                append: Vec::new(),
            })
        };

        if let Some((_, prefix)) = prefix.take() {
            if let Some(release) = &mut release {
                release.prefix = Some(prefix);
            }
        }

        if let Some(c) = self.maybe_channel()? {
            if let Some(release) = &mut release {
                release.pre = Some(c);
            }
        }

        while self.peek() == '.' {
            self.next();
            let start = self.index;

            while matches!(self.peek(), '0'..='9' | 'a'..='z') {
                self.next();
            }

            if let Some(release) = &mut release {
                release.append.push(&self.data[start..self.index]);
            }
        }

        Ok(release)
    }

    fn expr(&mut self) -> Result<Option<Version<'a>>> {
        let mut last = None;
        let mut needs_or = false;

        while self.peek() != EOF {
            match self.peek2() {
                (ws!(), _) => {
                    self.next();

                    while matches!(self.peek(), ws!()) {
                        self.next();
                    }
                }
                ('|', '|') => {
                    self.next();
                    self.next();
                    needs_or = false;
                    continue;
                }
                ('-' | '.', _) if !needs_or => {
                    self.next();

                    while matches!(self.peek(), '-' | '.') {
                        self.next();
                    }

                    continue;
                }
                ('0'..='9' | 'a'..='z' | '%', _) if !needs_or => {
                    let Some(release) = self.release()? else {
                        continue;
                    };

                    if last.is_none() {
                        last = Some(release);
                    }

                    needs_or = true;
                }
                _ => {
                    let b = self.peek();
                    bail!("Unexpected input '{}' at {}", self.index, b);
                }
            }
        }

        Ok(last)
    }
}

#[test]
fn parsing() {
    macro_rules! semver {
        ($major:expr, $minor:expr) => {
            SemanticVersion {
                original: concat!($major, ".", $minor),
                major: $major,
                minor: $minor,
                patch: None,
            }
        };

        ($major:expr, $minor:expr, $patch:expr) => {
            SemanticVersion {
                original: concat!($major, ".", $minor, ".", $patch),
                major: $major,
                minor: $minor,
                patch: Some($patch),
            }
        };
    }

    macro_rules! date {
        ($year:expr, $month:expr, $day:expr) => {
            Date {
                year: $year,
                month: $month,
                day: $day,
            }
        };
    }

    macro_rules! name {
        ($name:expr, $number:expr) => {
            Name {
                name: $name,
                number: Some($number),
            }
        };

        ($name:expr) => {
            Name {
                name: $name,
                number: None,
            }
        };
    }

    let mut vars = Vars {
        today: Date::new(2023, 1, 1).unwrap(),
        values: HashMap::new(),
    };

    vars.insert("fc39", "1.2.3-patch2.fc39");

    assert_eq!(
        parse("1.2", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2)),
            pre: None,
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("1.2.", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2)),
            pre: None,
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("1.2.3", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            pre: None,
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("0000001.000000000.000003", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(SemanticVersion {
                original: "0000001.000000000.000003",
                ..semver!(1, 0, 3)
            }),
            pre: None,
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("v1.2.3", &vars).unwrap(),
        Some(Version {
            prefix: Some("v"),
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            pre: None,
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("v1.2.3-pre1", &vars).unwrap(),
        Some(Version {
            prefix: Some("v"),
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            pre: Some(name!("pre", 1)),
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("2023-1-1", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::Date(date!(2023, 1, 1)),
            pre: None,
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("2023-1-1-pre1", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::Date(date!(2023, 1, 1)),
            pre: Some(name!("pre", 1)),
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("%date-pre1", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::Date(date!(2023, 1, 1)),
            pre: Some(name!("pre", 1)),
            append: Vec::new()
        })
    );

    assert_eq!(
        expr("|| %date-pre1\n|| ", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::Date(date!(2023, 1, 1)),
            pre: Some(name!("pre", 1)),
            append: Vec::new()
        })
    );

    assert_eq!(
        expr(" ||   || 1.2.3- ||", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            pre: None,
            append: Vec::new()
        })
    );

    assert_eq!(
        expr("%fc39-patch1", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            pre: Some(name!("patch", 1)),
            append: vec!["fc39"]
        })
    );

    assert_eq!(
        expr("name-patch1", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::Name(name!("name")),
            pre: Some(name!("patch", 1)),
            append: Vec::new(),
        })
    );

    assert_eq!(
        expr("name-patch1", &vars).unwrap(),
        Some(Version {
            prefix: None,
            kind: VersionKind::Name(name!("name")),
            pre: Some(name!("patch", 1)),
            append: Vec::new(),
        })
    );
}
