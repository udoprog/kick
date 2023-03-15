use std::io;
use std::path::Path;

use anyhow::Result;

#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LineColumn {
    pub(crate) line: usize,
    pub(crate) column: usize,
}

/// A file loaded into memory.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct File {
    data: Vec<u8>,
    line_starts: Vec<usize>,
}

impl File {
    /// Construct a new empty file.
    pub(crate) fn new() -> Self {
        Self {
            data: Vec::new(),
            line_starts: vec![0],
        }
    }

    /// Load a file from the given path.
    pub(crate) fn read<P>(path: P) -> io::Result<Self>
    where
        P: AsRef<Path>,
    {
        let bytes = std::fs::read(path)?;
        Ok(Self::from_vec(bytes))
    }

    /// Construct a new rust file wrapper.
    pub(crate) fn from_vec(data: Vec<u8>) -> Self {
        Self {
            data: data.to_vec(),
            line_starts: line_starts(&data),
        }
    }

    /// Get bytes of the file.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Iterate over comments.
    pub(crate) fn lines(&self) -> Lines<'_> {
        Lines {
            iter: self.data.split(|b| *b == b'\n'),
        }
    }

    /// Push a line onto the file.
    pub(crate) fn push(&mut self, line: &[u8]) {
        if !self.data.is_empty() {
            self.data.push(b'\n');
            self.line_starts.push(self.data.len());
        }

        self.data.extend(line);
    }

    /// Ensure that file has a trailing newline.
    pub(crate) fn ensure_trailing_newline(&mut self) {
        if !self.data.is_empty() && !self.data.ends_with(b"\n") {
            self.data.push(b'\n');
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
            Some(next) => std::str::from_utf8(&self.data[line_start..*next])?.trim_end(),
            None => std::str::from_utf8(&self.data[line_start..])?.trim_end(),
        };

        let column = offset.saturating_sub(line_start);
        Ok((LineColumn { line, column }, rest))
    }
}

pub(crate) struct Lines<'a> {
    iter: core::slice::Split<'a, u8, fn(&u8) -> bool>,
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
    line: &'a [u8],
}

impl<'a> Line<'a> {
    /// Get underlying bytes for the line.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        self.line
    }

    /// Get the comment.
    pub(crate) fn as_rust_comment(&self) -> Option<&'a [u8]> {
        if self.line.get(..3) == Some(&b"//!"[..]) {
            return self.line.get(3..);
        }

        None
    }
}

fn line_starts(source: &[u8]) -> Vec<usize> {
    let mut output = vec![0];

    for (index, n) in source.iter().enumerate() {
        if *n == b'\n' {
            output.push(index + 1);
        }
    }

    output
}
