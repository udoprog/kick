#![allow(clippy::manual_is_ascii_check)]

use std::collections::{HashMap, HashSet};
use std::str;

use anyhow::{bail, Context, Result};

use super::{Date, Name, SemanticVersion, Tail, Version, VersionKind};

const EOF: char = '\0';

enum Outcome<T> {
    Some(T),
    None,
    MissingVar,
}

impl<T> Outcome<T> {
    /// Map an outcome's inner value to a different type.
    fn map<U, F>(self, f: F) -> Outcome<U>
    where
        F: FnOnce(T) -> U,
    {
        match self {
            Outcome::Some(value) => Outcome::Some(f(value)),
            Outcome::None => Outcome::None,
            Outcome::MissingVar => Outcome::MissingVar,
        }
    }

    /// Coerce an outcome into an option.
    #[cfg(test)]
    fn some(self) -> Option<T> {
        match self {
            Outcome::Some(value) => Some(value),
            _ => None,
        }
    }
}

macro_rules! propagate {
    ($expr:expr) => {
        match $expr {
            Outcome::MissingVar => return Ok(Outcome::MissingVar),
            Outcome::Some(value) => Some(value),
            Outcome::None => None,
        }
    };
}

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

macro_rules! ident_start {
    () => {
        'a'..='z' | 'A'..='Z'
    }
}

macro_rules! ident_cont {
    () => {
        'a'..='z' | 'A'..='Z' | '0'..='9'
    }
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

pub(super) fn expr<'a>(
    input: &'a str,
    vars: &Vars<'a>,
    prefixes: &HashSet<String>,
) -> Result<Option<Version<'a>>> {
    let mut parser = Parser::new(input, vars, prefixes);
    parser.expr()
}

#[cfg(test)]
fn parse<'a>(
    input: &'a str,
    vars: &'a Vars,
    prefixes: &HashSet<String>,
) -> Result<Option<Version<'a>>> {
    let mut parser = Parser::new(input, vars, prefixes);
    Ok(parser.release()?.some())
}

struct Parser<'vars, 'a, 'b> {
    data: &'a str,
    vars: &'vars Vars<'a>,
    index: usize,
    prefixes: &'b HashSet<String>,
    max_prefix: usize,
}

impl<'vars, 'a, 'b> Parser<'vars, 'a, 'b> {
    fn new(data: &'a str, vars: &'vars Vars<'a>, prefixes: &'b HashSet<String>) -> Self {
        let max_prefix = prefixes.iter().map(|s| s.len()).max().unwrap_or(0);

        Parser {
            data,
            vars,
            index: 0,
            prefixes,
            max_prefix,
        }
    }

    fn ws(&mut self) {
        while matches!(self.peek(), ws!()) {
            self.next();
        }
    }

    /// Inner parsing.
    fn parse<P, O>(&self, input: &'a str, parse: P) -> Result<Outcome<O>>
    where
        P: FnOnce(&mut Parser<'vars, 'a, 'b>) -> Result<Outcome<O>>,
    {
        let mut parser = Parser::new(input, self.vars, self.prefixes);
        parser.ws();
        let output = parse(&mut parser)?;
        parser.ws();

        if parser.peek() != EOF {
            bail!("Unexpected input '{}'", &parser.data[parser.index..]);
        }

        Ok(output)
    }

    fn expand<P, O>(&mut self, parse: P) -> Result<Outcome<O>>
    where
        P: FnOnce(&mut Parser<'vars, 'a, 'b>) -> Result<Outcome<O>>,
    {
        let name = self.variable()?;

        let Some(value) = self.vars.get(name) else {
            return Ok(Outcome::MissingVar);
        };

        self.parse(value, parse)
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

    fn version(&mut self) -> Result<Outcome<SemanticVersion<'a>>> {
        let start = self.index;

        let major = self.number()?;
        expect!(self, '.');
        let minor = self.number()?;

        let patch = if self.peek() == '.' {
            let at = self.index;
            self.next();

            match self.number()? {
                Outcome::Some(patch) => Outcome::Some((self.index, Some(patch))),
                Outcome::None => Outcome::Some((at, None)),
                Outcome::MissingVar => Outcome::MissingVar,
            }
        } else {
            Outcome::None
        };

        let (end, patch) = match propagate!(patch) {
            Some((at, patch)) => (at, patch),
            None => (self.index, None),
        };

        Ok(Outcome::Some(SemanticVersion {
            original: &self.data[start..end],
            major: propagate!(major).context("Expected major")?,
            minor: propagate!(minor).context("Expected minor")?,
            patch,
        }))
    }

    fn date(&mut self) -> Result<Outcome<Date>> {
        let year = self.number()?;
        expect!(self, '-');
        let month = self.number()?;
        expect!(self, '-');
        let day = self.number()?;

        let year = propagate!(year).context("Expected year")?;
        let month = propagate!(month).context("Expected month")?;
        let day = propagate!(day).context("Expected day")?;

        let Ok(year) = i32::try_from(year) else {
            bail!("Year is out of range");
        };

        Ok(Outcome::Some(Date::new(year, month, day)?))
    }

    fn variable(&mut self) -> Result<&'a str> {
        debug_assert_eq!(self.next(), '%');

        if self.peek() != '{' {
            return self.ident();
        }

        self.next();
        let start = self.index;

        while matches!(self.peek(), ident_cont!() | '-' | '_' | '.') {
            self.next();
        }

        let end = self.index;
        expect!(self, '}');
        Ok(&self.data[start..end])
    }

    fn prefix(&mut self) -> Option<&'a str> {
        if self.prefixes.is_empty() {
            return None;
        }

        let start = self.index;

        while matches!(self.peek(), ident_start!()) {
            self.next();

            let value = &self.data[start..self.index];

            if self.prefixes.contains(value) {
                return Some(value);
            }

            if self.index - start >= self.max_prefix {
                self.index = start;
                return None;
            }
        }

        self.index = start;
        None
    }

    fn channel_ident(&mut self) -> Option<&'a str> {
        let start = self.index;

        while matches!(self.peek(), ident_start!()) {
            if matches!(&self.data[start..self.index], "git") {
                break;
            }

            self.next();
        }

        if start == self.index {
            return None;
        }

        Some(&self.data[start..self.index])
    }

    fn ident(&mut self) -> Result<&'a str> {
        let start = self.index;

        expect!(self, ident_start!());

        while let ident_cont!() = self.peek() {
            self.next();
        }

        if start == self.index {
            bail!("Identifier cannot be empty at '{}'", &self.data[start..]);
        }

        let end = self.index;
        Ok(&self.data[start..end])
    }

    fn number(&mut self) -> Result<Outcome<u32>> {
        let start = self.index;

        if self.peek() == '%' {
            return self.expand(Parser::number);
        }

        // NB: Ignore zero-prefixing.
        if self.peek() == '0' {
            while self.peek() == '0' {
                self.next();
            }
        } else if !matches!(self.peek(), '1'..='9') {
            return Ok(Outcome::None);
        };

        Ok(Outcome::Some(self.number_rem(start)?))
    }

    fn hex(&mut self) -> Result<Outcome<&'a str>> {
        let start = self.index;

        if self.peek() == '%' {
            return self.expand(Parser::hex);
        }

        while self.peek().is_ascii_hexdigit() {
            self.next();
        }

        if start == self.index {
            return Ok(Outcome::None);
        }

        Ok(Outcome::Some(&self.data[start..self.index]))
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

    fn maybe_channel(&mut self) -> Result<Outcome<Name<'a>>> {
        if self.peek() == '-' {
            self.next();

            if matches!(self.peek(), EOF | ws!()) {
                return Ok(Outcome::None);
            }

            Ok(self.channel()?)
        } else {
            Ok(Outcome::None)
        }
    }

    fn channel(&mut self) -> Result<Outcome<Name<'a>>> {
        if self.peek() == '%' {
            return self.expand(Parser::channel);
        }

        let start = self.index;

        let Some(name) = self.channel_ident() else {
            bail!("Identifier cannot be empty at `{}`", &self.data[start..]);
        };

        self.make_name(name)
    }

    fn make_name(&mut self, name: &'a str) -> Result<Outcome<Name<'a>>> {
        let tail = match name {
            "git" => propagate!(self.hex()?).map(Tail::Hash),
            _ => propagate!(self.number()?).map(Tail::Number),
        };

        Ok(Outcome::Some(Name { name, tail }))
    }

    fn release(&mut self) -> Result<Outcome<Version<'a>>> {
        let start = self.index;
        let mut prefix = self.prefix().map(move |prefix| (start, prefix));

        let mut release = 'release: {
            let kind = 'kind: {
                match self.peek() {
                    '%' => match self.variable()? {
                        "date" => break 'kind Outcome::Some(VersionKind::Date(self.vars.today)),
                        other => {
                            let Some(value) = self.vars.get(other) else {
                                break 'release Outcome::MissingVar;
                            };

                            break 'release self.parse(value, Parser::release)?;
                        }
                    },
                    '0'..='9' => {
                        let stored = self.index;

                        if let Ok(Outcome::Some(version)) = self.version() {
                            break 'kind Outcome::Some(VersionKind::SemanticVersion(version));
                        }

                        self.index = stored;

                        if let Ok(Outcome::Some(date)) = self.date() {
                            break 'kind Outcome::Some(VersionKind::Date(date));
                        }

                        self.index = stored;
                    }
                    _ => {}
                }

                let start = prefix.take().map(|(index, _)| index).unwrap_or(self.index);
                self.index = start;
                self.channel()?.map(VersionKind::Name)
            };

            let prefix = prefix.take().map(|(_, prefix)| prefix);

            kind.map(|kind| Version {
                prefix,
                kind,
                names: Vec::new(),
                append: Vec::new(),
            })
        };

        if let Some((_, prefix)) = prefix.take() {
            if let Outcome::Some(release) = &mut release {
                release.prefix = Some(prefix);
            }
        }

        loop {
            let c = match self.maybe_channel()? {
                Outcome::Some(c) => c,
                Outcome::None => break,
                Outcome::MissingVar => continue,
            };

            if let Outcome::Some(release) = &mut release {
                release.names.push(c);
            }
        }

        while self.peek() == '.' {
            self.next();
            let start = self.index;

            while matches!(self.peek(), ident_cont!()) {
                self.next();
            }

            if let Outcome::Some(release) = &mut release {
                release.append.push(&self.data[start..self.index]);
            }
        }

        Ok(release)
    }

    fn expr(&mut self) -> Result<Option<Version<'a>>> {
        let mut first = None;
        let mut needs_or = false;

        while self.peek() != EOF {
            match self.peek2() {
                (ws!(), _) => {
                    self.ws();
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
                (ident_cont!() | '%', _) if !needs_or => {
                    let release = match self.release()? {
                        Outcome::Some(release) => release,
                        _ => continue,
                    };

                    if first.is_none() {
                        first = Some(release);
                    }

                    needs_or = true;
                }
                _ => {
                    bail!("Unexpected input '{}'", &self.data[self.index..]);
                }
            }
        }

        Ok(first)
    }
}

#[test]
fn parsing() {
    use crate::release::Tail;

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
        ($name:expr, {$hash:expr}) => {
            Name {
                name: $name,
                tail: Some(Tail::Hash($hash)),
            }
        };

        ($name:expr, $number:expr) => {
            Name {
                name: $name,
                tail: Some(Tail::Number($number)),
            }
        };

        ($name:expr) => {
            Name {
                name: $name,
                tail: None,
            }
        };
    }

    let mut vars = Vars {
        today: Date::new(2023, 1, 1).unwrap(),
        values: HashMap::new(),
    };

    vars.insert("fc39", "1.2.3-patch2.fc39");
    vars.insert("sha", "99aabbcceeff");
    vars.insert("channel", "patch");
    vars.insert("channel.2", "patch1");

    let mut prefixes = HashSet::new();
    prefixes.insert(String::from("v"));

    macro_rules! parse {
        ($input:expr) => {
            parse($input, &vars, &prefixes).unwrap()
        };
    }

    macro_rules! expr {
        ($input:expr) => {
            expr($input, &vars, &prefixes).unwrap()
        };
    }

    assert_eq!(
        parse!("1.2"),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2)),
            names: Vec::new(),
            append: Vec::new()
        })
    );

    assert_eq!(
        parse!("1.2."),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2)),
            names: Vec::new(),
            append: Vec::new()
        })
    );

    assert_eq!(
        parse!("1.2.3"),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            names: Vec::new(),
            append: Vec::new()
        })
    );

    assert_eq!(
        parse!("0000001.000000000.000003"),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(SemanticVersion {
                original: "0000001.000000000.000003",
                ..semver!(1, 0, 3)
            }),
            names: Vec::new(),
            append: Vec::new()
        })
    );

    assert_eq!(
        parse!("v1.2.3"),
        Some(Version {
            prefix: Some("v"),
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            names: Vec::new(),
            append: Vec::new()
        })
    );

    assert_eq!(
        parse!("v1.2.3-pre1"),
        Some(Version {
            prefix: Some("v"),
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            names: vec![name!("pre", 1)],
            append: Vec::new()
        })
    );

    assert_eq!(
        parse!("2023-1-1"),
        Some(Version {
            prefix: None,
            kind: VersionKind::Date(date!(2023, 1, 1)),
            names: Vec::new(),
            append: Vec::new()
        })
    );

    assert_eq!(
        parse!("2023-1-1-pre1"),
        Some(Version {
            prefix: None,
            kind: VersionKind::Date(date!(2023, 1, 1)),
            names: vec![name!("pre", 1)],
            append: Vec::new()
        })
    );

    assert_eq!(
        parse!("%date-pre1"),
        Some(Version {
            prefix: None,
            kind: VersionKind::Date(date!(2023, 1, 1)),
            names: vec![name!("pre", 1)],
            append: Vec::new()
        })
    );

    assert_eq!(
        expr!("|| %date-pre1\n|| "),
        Some(Version {
            prefix: None,
            kind: VersionKind::Date(date!(2023, 1, 1)),
            names: vec![name!("pre", 1)],
            append: Vec::new()
        })
    );

    assert_eq!(
        expr!(" ||   || 1.2.3- ||"),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            names: Vec::new(),
            append: Vec::new()
        })
    );

    assert_eq!(
        expr!("%fc39-patch1"),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            names: vec![name!("patch", 2), name!("patch", 1)],
            append: vec!["fc39"]
        })
    );

    assert_eq!(
        expr!("name-patch1"),
        Some(Version {
            prefix: None,
            kind: VersionKind::Name(name!("name")),
            names: vec![name!("patch", 1)],
            append: Vec::new(),
        })
    );

    assert_eq!(
        expr!("name-patch1"),
        Some(Version {
            prefix: None,
            kind: VersionKind::Name(name!("name")),
            names: vec![name!("patch", 1)],
            append: Vec::new(),
        })
    );

    assert_eq!(
        expr!("name-gitffcc11"),
        Some(Version {
            prefix: None,
            kind: VersionKind::Name(name!("name")),
            names: vec![name!("git", { "ffcc11" })],
            append: Vec::new(),
        })
    );

    assert_eq!(
        expr!("name-git%sha"),
        Some(Version {
            prefix: None,
            kind: VersionKind::Name(name!("name")),
            names: vec![name!("git", { "99aabbcceeff" })],
            append: Vec::new(),
        })
    );

    assert_eq!(
        expr!("1.2.3-%channel"),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            names: vec![name!("patch")],
            append: Vec::new(),
        })
    );

    assert_eq!(
        expr!("1.2.3-%{channel.2}"),
        Some(Version {
            prefix: None,
            kind: VersionKind::SemanticVersion(semver!(1, 2, 3)),
            names: vec![name!("patch", 1)],
            append: Vec::new(),
        })
    );
}
