use std::fmt;
use std::fs;
use std::io;
use std::path::Path;

use anyhow::Result;
use musli::{Decode, Encode};

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineColumn {
    pub(crate) line: usize,
    pub(crate) column: usize,
}

/// A file loaded into memory.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
pub(crate) struct File {
    data: String,
    line_starts: Vec<usize>,
}

impl File {
    /// Construct a new empty file.
    pub(crate) fn new() -> Self {
        Self {
            data: String::new(),
            line_starts: vec![0],
        }
    }

    /// Load a file from the given path.
    pub(crate) fn read<P>(path: P) -> io::Result<Self>
    where
        P: AsRef<Path>,
    {
        let string = fs::read_to_string(path)?;
        Ok(Self::from_string(string))
    }

    /// Construct a new rust file wrapper.
    pub(crate) fn from_string(data: String) -> Self {
        let line_starts = line_starts(&data);

        Self { data, line_starts }
    }

    /// Get string of the file.
    pub(crate) fn as_str(&self) -> &str {
        &self.data
    }

    /// Try to coerce file into a non-empty string.
    pub(crate) fn as_non_empty_str(&self) -> Option<&str> {
        if self.data.is_empty() {
            return None;
        }

        Some(&self.data)
    }

    /// Iterate over comments.
    pub(crate) fn lines(&self) -> Lines<'_> {
        Lines {
            iter: self.data.lines(),
        }
    }

    /// Push a line onto the file.
    pub(crate) fn line(&mut self, line: impl fmt::Display) {
        use std::fmt::Write;

        if !self.data.is_empty() {
            self.data.push('\n');
            self.line_starts.push(self.data.len());
        }

        write!(self.data, "{line}").unwrap();
    }

    /// Ensure that file has a trailing newline.
    pub(crate) fn ensure_trailing_newline(&mut self) {
        if !self.data.is_empty() && !self.data.ends_with('\n') {
            self.data.push('\n');
            self.line_starts.push(self.data.len());
        }
    }

    /// Get the line and column for the given byte range.
    pub(crate) fn line_column(&self, offset: usize) -> Result<(LineColumn, &str)> {
        if offset == 0 {
            return Ok(Default::default());
        }

        let line = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(0) => return Ok(Default::default()),
            Err(n) => n - 1,
        };

        let line_start = self.line_starts[line];

        let rest = match self.line_starts.get(line + 1) {
            Some(next) => self.data[line_start..*next].trim_end(),
            None => self.data[line_start..].trim_end(),
        };

        let column = offset.saturating_sub(line_start);
        Ok((LineColumn { line, column }, rest))
    }
}

impl<S> FromIterator<S> for File
where
    S: AsRef<str>,
{
    #[inline]
    fn from_iter<T: IntoIterator<Item = S>>(iter: T) -> Self {
        let mut file = File::new();

        for line in iter {
            file.line(line.as_ref());
        }

        file
    }
}

pub(crate) struct Lines<'a> {
    iter: core::str::Lines<'a>,
}

impl<'a> Iterator for Lines<'a> {
    type Item = Line<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        Some(Line {
            line: self.iter.next()?,
        })
    }
}

pub(crate) struct Line<'a> {
    line: &'a str,
}

impl<'a> Line<'a> {
    /// Get the comment.
    pub(crate) fn as_rust_comment(&self) -> Option<&'a str> {
        if self.line.get(..3) == Some("//!") {
            let line = self.line.get(3..)?;

            return if let Some(" ") = line.get(..1) {
                line.get(1..)
            } else {
                Some(line)
            };
        }

        None
    }
}

impl AsRef<str> for Line<'_> {
    #[inline]
    fn as_ref(&self) -> &str {
        self.line
    }
}

impl fmt::Display for Line<'_> {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.line.fmt(f)
    }
}

fn line_starts(source: &str) -> Vec<usize> {
    let mut output = vec![0];

    for (index, n) in source.bytes().enumerate() {
        if n == b'\n' {
            output.push(index + 1);
        }
    }

    output
}
