//! Parsing for .gitmodules files.

use thiserror::Error;

pub(crate) struct SectionParser<'a, 'p> {
    name: &'p str,
    parser: &'a mut Parser<'p>,
}

impl<'a, 'p> SectionParser<'a, 'p> {
    /// Get the name of the section.
    pub(crate) fn name(&self) -> &'p str {
        self.name
    }

    /// Parse the next section.
    pub(crate) fn next_section(&mut self) -> Result<Option<(&'p str, &'p [u8])>, ParseError> {
        // Another section coming.
        if matches!(self.parser.get(0), Some(b'[') | None) {
            return Ok(None);
        }

        let key = self.parser.ident()?;
        self.parser.expect(b'=')?;
        self.parser.skip_whitespace();
        let value = self.parser.until_eol()?;
        Ok(Some((key, value)))
    }
}

pub(crate) struct Parser<'a> {
    input: &'a [u8],
    cursor: usize,
}

impl<'a> Parser<'a> {
    /// Construct a new parser.
    pub(crate) fn new(input: &'a [u8]) -> Self {
        Self { input, cursor: 0 }
    }

    fn get(&mut self, n: usize) -> Option<u8> {
        self.skip_whitespace();
        Some(*self.input.get(self.cursor.checked_add(n)?)?)
    }

    #[inline]
    fn skip(&mut self) {
        self.cursor += 1;
    }

    fn skip_whitespace(&mut self) {
        while matches!(
            self.input.get(self.cursor),
            Some(b' ' | b'\t' | b'\n' | b'\r')
        ) {
            self.skip();
        }
    }

    /// Process value until end-of-line.
    fn until_eol(&mut self) -> Result<&'a [u8], ParseError> {
        let start = self.cursor;

        while !matches!(self.input.get(self.cursor), Some(b'\n') | None) {
            self.skip();
        }

        self.slice(start, self.cursor)
    }

    /// Test expectation.
    fn expect(&mut self, expected: u8) -> Result<(), ParseError> {
        if self.get(0) != Some(expected) {
            return Err(ParseError::Expected {
                expected,
                actual: self.get(0),
            });
        }

        self.skip();
        Ok(())
    }

    /// Get a slice.
    fn slice(&self, from: usize, to: usize) -> Result<&'a [u8], ParseError> {
        self.input.get(from..to).ok_or(ParseError::SliceError)
    }

    /// Get a slice as a string.
    fn slice_str(&self, from: usize, to: usize) -> Result<&'a str, ParseError> {
        std::str::from_utf8(self.slice(from, to)?).map_err(|_| ParseError::Utf8Error)
    }

    /// Parse an identifier.
    fn ident(&mut self) -> Result<&'a str, ParseError> {
        let start = self.cursor;

        while matches!(self.input.get(self.cursor), Some(b'a'..=b'z' | b'_' | b'-')) {
            self.cursor += 1;
        }

        self.slice_str(start, self.cursor)
    }

    /// Parse an identifier.
    fn quoted_string(&mut self) -> Result<&'a str, ParseError> {
        match self.get(0) {
            Some(b'"') => {
                self.skip();
            }
            _ => return Err(ParseError::ExpectedString),
        }

        let start = self.cursor;

        let end = loop {
            match self.input.get(self.cursor) {
                Some(b'"') => {
                    let end = self.cursor;
                    self.skip();
                    break end;
                }
                Some(..) => {
                    self.skip();
                }
                None => {
                    return Err(ParseError::UnclosedString);
                }
            }
        };

        self.slice_str(start, end)
    }

    /// Parse a section.
    pub(crate) fn parse_section(&mut self) -> Result<Option<SectionParser<'_, 'a>>, ParseError> {
        if self.get(0).is_none() {
            return Ok(None);
        }

        self.expect(b'[')?;

        match self.ident()? {
            "submodule" => {}
            actual => {
                return Err(ParseError::ExpectedSlice {
                    expected: Box::from("submodule"),
                    actual: Box::from(actual),
                })
            }
        }

        let name = self.quoted_string()?;
        self.expect(b']')?;
        Ok(Some(SectionParser { name, parser: self }))
    }
}

#[derive(Debug, Error)]
pub(crate) enum ParseError {
    #[error("slice error")]
    SliceError,
    #[error("utf-8 error")]
    Utf8Error,
    #[error("expected byte {expected}, but got {actual:?}")]
    Expected { expected: u8, actual: Option<u8> },
    #[error("expected {expected:?}, but got {actual:?}")]
    ExpectedSlice {
        expected: Box<str>,
        actual: Box<str>,
    },
    #[error("expected string")]
    ExpectedString,
    #[error("encountered an unclosed string")]
    UnclosedString,
}
