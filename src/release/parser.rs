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

pub(super) fn expr(input: &str) -> Result<Option<Release<'_>>> {
    let mut parser = Parser::new(input.as_bytes())?;
    parser.expr()
}

#[cfg(test)]
fn expr_with(input: &str, today: Date) -> Result<Option<Release<'_>>> {
    let mut parser = Parser::new_with(input.as_bytes(), today);
    parser.expr()
}

#[cfg(test)]
fn parse_with(input: &str, today: Date) -> Result<Release<'_>> {
    let mut parser = Parser::new_with(input.as_bytes(), today);
    parser.release()
}

struct Parser<'a> {
    data: &'a [u8],
    index: usize,
    today: Date,
}

impl<'a> Parser<'a> {
    fn new(data: &'a [u8]) -> Result<Self> {
        Ok(Self::new_with(data, Date::today()?))
    }

    fn new_with(data: &'a [u8], today: Date) -> Self {
        Parser {
            data,
            index: 0,
            today,
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

    fn parse_channel(&mut self, start: usize) -> Result<Channel<'a>> {
        let name = self.parse_ident(start)?;

        let pre = if self.peek().is_ascii_digit() {
            Some(self.parse_number()?)
        } else {
            None
        };

        Ok(Channel { name, pre })
    }

    fn parse_ident(&mut self, start: usize) -> Result<&'a str> {
        while self.peek().is_ascii_lowercase() {
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

            Ok(Some(self.parse_channel(self.index)?))
        } else {
            Ok(None)
        }
    }

    fn release(&mut self) -> Result<Release<'a>> {
        let start = self.index;

        while self.peek().is_ascii_lowercase() {
            self.next();
        }

        let mut prefix = if self.index != start {
            Some(str::from_utf8(&self.data[start..self.index])?)
        } else {
            None
        };

        let kind = match self.peek() {
            '%' => {
                self.next();

                match self.parse_ident(self.index)? {
                    "date" => {
                        let date = self.today;
                        let channel = self.maybe_channel()?;
                        ReleaseKind::Date { date, channel }
                    }
                    other => {
                        bail!("Unknown variable `{}`", other);
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
                        let channel = self.maybe_channel()?;
                        ReleaseKind::Version { version, channel }
                    }
                    '-' => {
                        self.next();
                        let date = self.parse_date(first)?;
                        let channel = self.maybe_channel()?;
                        ReleaseKind::Date { date, channel }
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
                let channel = self.parse_channel(self.index)?;
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

        let mut append = Vec::new();

        while self.peek() == '.' {
            self.next();
            let start = self.index;

            while matches!(self.peek(), '0'..='9' | 'a'..='z') {
                self.next();
            }

            append.push(str::from_utf8(&self.data[start..self.index])?);
        }

        Ok(Release {
            prefix,
            kind,
            append,
        })
    }

    fn expr(&mut self) -> Result<Option<Release<'a>>> {
        let mut last = None;

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
                    continue;
                }
                ('-' | '.', _) => {
                    self.next();

                    while matches!(self.peek(), '-' | '.') {
                        self.next();
                    }

                    continue;
                }
                ('0'..='9' | 'a'..='z' | '%', _) => {
                    let release = self.release()?;

                    if last.is_none() {
                        last = Some(release);
                    }
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

    let today = Date::new(2023, 1, 1).unwrap();

    assert_eq!(
        parse_with("1.2.3", today).unwrap(),
        Release {
            prefix: None,
            kind: ReleaseKind::Version {
                version: version!(1, 2, 3),
                channel: None
            },
            append: Vec::new()
        }
    );

    assert_eq!(
        parse_with("0000001.000000000.000003", today).unwrap(),
        Release {
            prefix: None,
            kind: ReleaseKind::Version {
                version: Version {
                    original: "0000001.000000000.000003",
                    ..version!(1, 0, 3)
                },
                channel: None
            },
            append: Vec::new()
        }
    );

    assert_eq!(
        parse_with("v1.2.3", today).unwrap(),
        Release {
            prefix: Some("v"),
            kind: ReleaseKind::Version {
                version: version!(1, 2, 3),
                channel: None
            },
            append: Vec::new()
        }
    );

    assert_eq!(
        parse_with("v1.2.3-pre1", today).unwrap(),
        Release {
            prefix: Some("v"),
            kind: ReleaseKind::Version {
                version: version!(1, 2, 3),
                channel: Some(channel!(pre, 1)),
            },
            append: Vec::new()
        }
    );

    assert_eq!(
        parse_with("2023-1-1", today).unwrap(),
        Release {
            prefix: None,
            kind: ReleaseKind::Date {
                date: date!(2023, 1, 1),
                channel: None,
            },
            append: Vec::new()
        }
    );

    assert_eq!(
        parse_with("2023-1-1-pre1", today).unwrap(),
        Release {
            prefix: None,
            kind: ReleaseKind::Date {
                date: date!(2023, 1, 1),
                channel: Some(channel!(pre, 1)),
            },
            append: Vec::new()
        }
    );

    assert_eq!(
        parse_with("%date-pre1", today).unwrap(),
        Release {
            prefix: None,
            kind: ReleaseKind::Date {
                date: date!(2023, 1, 1),
                channel: Some(channel!(pre, 1)),
            },
            append: Vec::new()
        }
    );

    assert_eq!(
        expr_with("|| %date-pre1\n|| ", today).unwrap(),
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
        expr_with(" ||   || 1.2.3- ||", today).unwrap(),
        Some(Release {
            prefix: None,
            kind: ReleaseKind::Version {
                version: version!(1, 2, 3),
                channel: None,
            },
            append: Vec::new()
        })
    );
}
