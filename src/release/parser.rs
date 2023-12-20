use std::collections::HashMap;
use std::str;

use anyhow::{bail, Context, Result};

use super::{Channel, Date, Release, ReleaseKind, Version};

const EOF: char = '\0';

macro_rules! fail {
    ($slf:expr, $pat:pat) => {{
        let b = $slf.peek();
        bail!(
            "Expected {} at {}, but got '{}'",
            stringify!($pat),
            $slf.index,
            b
        );
    }};
}

macro_rules! expect {
    ($slf:expr, $pat:pat) => {{
        let b = $slf.peek();

        if !matches!(b, $pat) {
            bail!(
                "Expected {} at {}, but got '{}'",
                stringify!($pat),
                $slf.index,
                b
            );
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

pub(super) fn expr<'a>(input: &'a str, vars: &Vars<'a>) -> Result<Option<Release<'a>>> {
    let mut parser = Parser::new(input.as_bytes(), vars);
    parser.expr()
}

#[cfg(test)]
fn parse<'a>(input: &'a str, vars: &'a Vars) -> Result<Option<Release<'a>>> {
    let mut parser = Parser::new(input.as_bytes(), vars);
    parser.release()
}

struct Parser<'vars, 'a> {
    data: &'a [u8],
    vars: &'vars Vars<'a>,
    index: usize,
}

impl<'vars, 'a> Parser<'vars, 'a> {
    fn new(data: &'a [u8], vars: &'vars Vars<'a>) -> Self {
        Parser {
            data,
            vars,
            index: 0,
        }
    }

    fn peek(&mut self) -> char {
        let Some(&b) = self.data.get(self.index) else {
            return EOF;
        };

        b as char
    }

    fn peek2(&mut self) -> char {
        let Some(index) = self.index.checked_add(1) else {
            return EOF;
        };

        let Some(&b) = self.data.get(index) else {
            return EOF;
        };

        b as char
    }

    fn next(&mut self) -> char {
        let b = self.peek();

        if b != EOF {
            self.index += 1;
        }

        b
    }

    fn parse_version(&mut self, start: usize, major: u32) -> Result<Version<'a>> {
        let minor = self.parse_number()?;
        expect!(self, '.');
        let patch = self.parse_number()?;

        Ok(Version {
            original: str::from_utf8(&self.data[start..self.index])?,
            major,
            minor,
            patch,
        })
    }

    fn parse_date(&mut self, year: u32) -> Result<Date> {
        let year = i32::try_from(year).context("Year out of range")?;
        let month = self.parse_number()?;
        expect!(self, '-');
        let day = self.parse_number()?;
        Date::new(year, month, day)
    }

    fn channel(&mut self, start: usize) -> Result<Channel<'a>> {
        let name = self.channel_ident(start)?;

        let pre = if self.peek().is_ascii_digit() {
            Some(self.parse_number()?)
        } else {
            None
        };

        Ok(Channel { name, pre })
    }

    fn variable(&mut self) -> Result<&'a str> {
        if self.peek() != '{' {
            return self.parse_ident(self.index);
        }

        self.next();
        let start = self.index;

        while matches!(self.peek(), 'a'..='z' | '0'..='9' | '-' | '_' | '.') {
            self.next();
        }

        let end = self.index;
        expect!(self, '}');
        Ok(str::from_utf8(&self.data[start..end])?)
    }

    fn channel_ident(&mut self, start: usize) -> Result<&'a str> {
        while self.peek().is_ascii_lowercase() {
            self.next();
        }

        if start == self.index {
            bail!("Identifier cannot be empty at {}", start);
        }

        let end = self.index;
        Ok(str::from_utf8(&self.data[start..end])?)
    }

    fn parse_ident(&mut self, start: usize) -> Result<&'a str> {
        expect!(self, 'a'..='z');

        while let 'a'..='z' | '0'..='9' = self.peek() {
            self.next();
        }

        if start == self.index {
            bail!("Identifier cannot be empty at {}", start);
        }

        let end = self.index;
        Ok(str::from_utf8(&self.data[start..end])?)
    }

    fn parse_number(&mut self) -> Result<u32> {
        // NB: Ignore zero-prefixing.
        let mut number = if self.peek() == '0' {
            while self.peek() == '0' {
                self.next();
            }

            0
        } else {
            expect!(self, '1'..='9') as u32 - b'0' as u32
        };

        while let digit @ '0'..='9' = self.peek() {
            self.next();

            let Some(update) = number.checked_mul(10) else {
                bail!("Numerical overflow at {}", self.index);
            };

            number = update + (digit as u8 - b'0') as u32;
        }

        Ok(number)
    }

    fn maybe_channel(&mut self) -> Result<Option<Channel<'a>>> {
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

    fn release(&mut self) -> Result<Option<Release<'a>>> {
        let start = self.index;

        while self.peek().is_ascii_lowercase() {
            self.next();
        }

        let mut prefix = if self.index != start {
            Some(str::from_utf8(&self.data[start..self.index])?)
        } else {
            None
        };

        let mut release = 'release: {
            let kind = match self.peek() {
                '%' => {
                    self.next();

                    match self.variable()? {
                        "date" => {
                            let date = self.vars.today;
                            let channel = self.maybe_channel()?;
                            ReleaseKind::Date { date, channel }
                        }
                        other => {
                            let Some(value) = self.vars.get(other) else {
                                break 'release None;
                            };

                            let mut parser = Parser::new(value.as_bytes(), self.vars);
                            break 'release parser.release()?;
                        }
                    }
                }
                '0'..='9' => {
                    let start = self.index;
                    let first = self.parse_number()?;

                    match self.peek() {
                        '.' => {
                            self.next();
                            let version = self.parse_version(start, first)?;
                            ReleaseKind::Version {
                                version,
                                channel: None,
                            }
                        }
                        '-' => {
                            self.next();
                            let date = self.parse_date(first)?;
                            ReleaseKind::Date {
                                date,
                                channel: None,
                            }
                        }
                        _ => {
                            let Some(name) = prefix.take() else {
                                fail!(self, b'.' | b'-');
                            };

                            ReleaseKind::Name {
                                channel: Channel {
                                    name,
                                    pre: Some(first),
                                },
                            }
                        }
                    }
                }
                'a'..='z' => {
                    let channel = self.channel(self.index)?;
                    ReleaseKind::Name { channel }
                }
                _ => {
                    let Some(name) = prefix.take() else {
                        fail!(self, b'0'..=b'9' | b'a'..=b'z');
                    };

                    ReleaseKind::Name {
                        channel: Channel { name, pre: None },
                    }
                }
            };

            Some(Release {
                prefix: prefix.take(),
                kind,
                append: Vec::new(),
            })
        };

        if let Some(prefix) = prefix.take() {
            if let Some(release) = &mut release {
                release.prefix = Some(prefix);
            }
        }

        if let Some(c) = self.maybe_channel()? {
            if let Some(release) = &mut release {
                if let Some(channel) = release.channel_mut() {
                    *channel = Some(c);
                }
            }
        }

        while self.peek() == '.' {
            self.next();
            let start = self.index;

            while matches!(self.peek(), '0'..='9' | 'a'..='z') {
                self.next();
            }

            if let Some(release) = &mut release {
                release
                    .append
                    .push(str::from_utf8(&self.data[start..self.index])?);
            }
        }

        Ok(release)
    }

    fn expr(&mut self) -> Result<Option<Release<'a>>> {
        let mut last = None;
        let mut needs_or = false;

        while self.peek() != EOF {
            match (self.peek(), self.peek2()) {
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
    macro_rules! version {
        ($major:expr, $minor:expr, $patch:expr) => {
            Version {
                original: concat!($major, ".", $minor, ".", $patch),
                major: $major,
                minor: $minor,
                patch: $patch,
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

    macro_rules! channel {
        ($name:ident, $pre:expr) => {
            Channel {
                name: stringify!($name),
                pre: Some($pre),
            }
        };

        ($name:ident) => {
            Channel {
                name: stringify!($name),
                pre: None,
            }
        };
    }

    let mut vars = Vars {
        today: Date::new(2023, 1, 1).unwrap(),
        values: HashMap::new(),
    };

    vars.insert("fc39", "1.2.3-patch2.fc39");

    assert_eq!(
        parse("1.2.3", &vars).unwrap(),
        Some(Release {
            prefix: None,
            kind: ReleaseKind::Version {
                version: version!(1, 2, 3),
                channel: None
            },
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("0000001.000000000.000003", &vars).unwrap(),
        Some(Release {
            prefix: None,
            kind: ReleaseKind::Version {
                version: Version {
                    original: "0000001.000000000.000003",
                    ..version!(1, 0, 3)
                },
                channel: None
            },
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("v1.2.3", &vars).unwrap(),
        Some(Release {
            prefix: Some("v"),
            kind: ReleaseKind::Version {
                version: version!(1, 2, 3),
                channel: None
            },
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("v1.2.3-pre1", &vars).unwrap(),
        Some(Release {
            prefix: Some("v"),
            kind: ReleaseKind::Version {
                version: version!(1, 2, 3),
                channel: Some(channel!(pre, 1)),
            },
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("2023-1-1", &vars).unwrap(),
        Some(Release {
            prefix: None,
            kind: ReleaseKind::Date {
                date: date!(2023, 1, 1),
                channel: None,
            },
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("2023-1-1-pre1", &vars).unwrap(),
        Some(Release {
            prefix: None,
            kind: ReleaseKind::Date {
                date: date!(2023, 1, 1),
                channel: Some(channel!(pre, 1)),
            },
            append: Vec::new()
        })
    );

    assert_eq!(
        parse("%date-pre1", &vars).unwrap(),
        Some(Release {
            prefix: None,
            kind: ReleaseKind::Date {
                date: date!(2023, 1, 1),
                channel: Some(channel!(pre, 1)),
            },
            append: Vec::new()
        })
    );

    assert_eq!(
        expr("|| %date-pre1\n|| ", &vars).unwrap(),
        Some(Release {
            prefix: None,
            kind: ReleaseKind::Date {
                date: date!(2023, 1, 1),
                channel: Some(channel!(pre, 1)),
            },
            append: Vec::new()
        })
    );

    assert_eq!(
        expr(" ||   || 1.2.3- ||", &vars).unwrap(),
        Some(Release {
            prefix: None,
            kind: ReleaseKind::Version {
                version: version!(1, 2, 3),
                channel: None,
            },
            append: Vec::new()
        })
    );

    assert_eq!(
        expr("%fc39-patch1", &vars).unwrap(),
        Some(Release {
            prefix: None,
            kind: ReleaseKind::Version {
                version: version!(1, 2, 3),
                channel: Some(channel!(patch, 1)),
            },
            append: vec!["fc39"]
        })
    );
}
