#[cfg(test)]
mod tests;

use std::collections::VecDeque;
use std::fs;
use std::io;
use std::mem;
use std::path::Path;

use relative_path::{RelativePath, RelativePathBuf};

/// A compiled glob expression.
pub struct Glob<'a> {
    root: &'a Path,
    components: Vec<Component<'a>>,
}

impl<'a> Glob<'a> {
    /// Construct a new glob pattern.
    pub fn new<R, P>(root: &'a R, pattern: &'a P) -> Self
    where
        R: ?Sized + AsRef<Path>,
        P: ?Sized + AsRef<RelativePath>,
    {
        let components = compile_pattern(pattern);

        Self {
            root: root.as_ref(),
            components,
        }
    }

    /// Construct a new matcher.
    pub(crate) fn matcher(&self) -> Matcher<'_> {
        Matcher {
            root: self.root,
            queue: [(RelativePathBuf::new(), self.components.as_ref())]
                .into_iter()
                .collect(),
        }
    }
}

impl<'a> Matcher<'a> {
    /// Perform an expansion in the filesystem.
    fn expand_filesystem<M>(
        &mut self,
        current: &RelativePathBuf,
        rest: &'a [Component<'a>],
        mut m: M,
    ) -> io::Result<()>
    where
        M: FnMut(&str) -> bool,
    {
        let path = current.to_path(self.root);

        match fs::metadata(&path) {
            Ok(m) => {
                if !m.is_dir() {
                    return Ok(());
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return Ok(());
            }
            Err(e) => return Err(e),
        }

        for e in fs::read_dir(path)? {
            let e = e?;
            let c = e.file_name();
            let c = c.to_string_lossy();

            if !m(c.as_ref()) {
                continue;
            }

            let mut new = current.clone();
            new.push(c.as_ref());
            self.queue.push_back((new, rest));
        }

        Ok(())
    }

    /// Perform star star expansion.
    fn walk(&mut self, current: &RelativePathBuf, rest: &'a [Component<'a>]) -> io::Result<()> {
        let path = current.to_path(self.root);

        self.queue.push_back((current.clone(), rest));

        let mut queue = VecDeque::new();
        queue.push_back((current.to_owned(), path));

        while let Some((current, path)) = queue.pop_front() {
            match fs::metadata(&path) {
                Ok(m) => {
                    if !m.is_dir() {
                        return Ok(());
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    continue;
                }
                Err(e) => return Err(e),
            }

            for e in fs::read_dir(path)? {
                let e = e?;
                let c = e.file_name();
                let c = c.to_string_lossy();
                let next = current.join(c.as_ref());
                self.queue.push_back((next.clone(), rest));
                queue.push_back((next, e.path()));
            }
        }

        Ok(())
    }
}

pub(crate) struct Matcher<'a> {
    root: &'a Path,
    queue: VecDeque<(RelativePathBuf, &'a [Component<'a>])>,
}

impl Iterator for Matcher<'_> {
    type Item = io::Result<RelativePathBuf>;

    fn next(&mut self) -> Option<Self::Item> {
        'outer: loop {
            let (mut path, mut components) = self.queue.pop_front()?;

            while let [first, rest @ ..] = components {
                match first {
                    Component::ParentDir => {
                        path = path.join(relative_path::Component::ParentDir);
                    }
                    Component::Normal(normal) => {
                        path = path.join(normal);
                    }
                    Component::Fragment(fragment) => {
                        if let Err(e) =
                            self.expand_filesystem(&path, rest, |name| fragment.is_match(name))
                        {
                            return Some(Err(e));
                        }

                        continue 'outer;
                    }
                    Component::StarStar => {
                        if let Err(e) = self.walk(&path, rest) {
                            return Some(Err(e));
                        }

                        continue 'outer;
                    }
                }

                components = rest;
            }

            return Some(Ok(path));
        }
    }
}

#[derive(Clone)]
enum Component<'a> {
    /// Parent directory.
    ParentDir,
    /// A normal component.
    Normal(&'a str),
    /// Normal component, compiled into a fragment.
    Fragment(Fragment<'a>),
    /// `**` component, which keeps expanding.
    StarStar,
}

fn compile_pattern<P>(pattern: &P) -> Vec<Component<'_>>
where
    P: ?Sized + AsRef<RelativePath>,
{
    let pattern = pattern.as_ref();

    let mut output = Vec::new();

    for c in pattern.components() {
        output.push(match c {
            relative_path::Component::CurDir => continue,
            relative_path::Component::ParentDir => Component::ParentDir,
            relative_path::Component::Normal("**") => Component::StarStar,
            relative_path::Component::Normal(normal) => {
                let fragment = Fragment::parse(normal);

                if let Some(normal) = fragment.as_literal() {
                    Component::Normal(normal)
                } else {
                    Component::Fragment(fragment)
                }
            }
        });
    }

    output
}

#[derive(Debug, Clone, Copy)]
enum Part<'a> {
    Star,
    Literal(&'a str),
}

/// A match fragment.
#[derive(Debug, Clone)]
pub(crate) struct Fragment<'a> {
    parts: Box<[Part<'a>]>,
}

impl<'a> Fragment<'a> {
    pub(crate) fn parse(mut string: &'a str) -> Fragment<'a> {
        let mut parts = Vec::new();
        // Prevent to wildcards in a row.
        let mut star = true;

        while let Some(n) = string.find('*') {
            if n > 0 {
                parts.push(Part::Literal(&string[..n]));
                star = true;
            }

            if mem::take(&mut star) {
                parts.push(Part::Star);
            }

            string = &string[n + '*'.len_utf8()..];
        }

        if !string.is_empty() {
            parts.push(Part::Literal(string));
        }

        Fragment {
            parts: parts.into_boxed_slice(),
        }
    }

    /// Test if the given string matches the current fragment.
    pub(crate) fn is_match(&self, string: &str) -> bool {
        let mut backtrack = VecDeque::new();
        backtrack.push_back((self.as_parts(), string));

        while let Some((mut parts, mut string)) = backtrack.pop_front() {
            while let Some(part) = parts.first() {
                match part {
                    Part::Star => {
                        // Peek the next literal component. If we have a
                        // trailing wildcard (which this constitutes) then it
                        // is by definition a match.
                        let Some(Part::Literal(peek)) = parts.get(1) else {
                            return true;
                        };

                        let Some(peek) = peek.chars().next() else {
                            return true;
                        };

                        while let Some(c) = string.chars().next() {
                            if c == peek {
                                backtrack.push_front((
                                    parts,
                                    string.get(c.len_utf8()..).unwrap_or_default(),
                                ));
                                break;
                            }

                            string = string.get(c.len_utf8()..).unwrap_or_default();
                        }
                    }
                    Part::Literal(literal) => {
                        // The literal component must be an exact prefix of the
                        // current string.
                        let Some(remainder) = string.strip_prefix(literal) else {
                            return false;
                        };

                        string = remainder;
                    }
                }

                parts = parts.get(1..).unwrap_or_default();
            }

            if string.is_empty() {
                return true;
            }
        }

        false
    }

    /// Get parts of the fragment.
    fn as_parts(&self) -> &[Part<'a>] {
        &self.parts
    }

    /// Treat the fragment as a single normal component.
    fn as_literal(&self) -> Option<&'a str> {
        if let [Part::Literal(one)] = self.as_parts() {
            Some(one)
        } else {
            None
        }
    }
}
