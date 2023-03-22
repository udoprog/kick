#[cfg(test)]
mod tests;

use std::collections::VecDeque;
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
        let dirs = std::fs::read_dir(current.to_path(self.root))?;

        for e in dirs {
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
}

pub(crate) struct Matcher<'a> {
    root: &'a Path,
    queue: VecDeque<(RelativePathBuf, &'a [Component<'a>])>,
}

impl<'a> Iterator for Matcher<'a> {
    type Item = io::Result<RelativePathBuf>;

    fn next(&mut self) -> Option<Self::Item> {
        'outer: loop {
            let (c, p) = self.queue.pop_front()?;
            let mut iter = p.iter();

            while let Some(component) = iter.next() {
                match component {
                    Component::CurDir => {}
                    Component::ParentDir => {}
                    Component::Normal(fragment) => {
                        if let Err(e) = self
                            .expand_filesystem(&c, iter.as_slice(), |name| fragment.is_match(name))
                        {
                            return Some(Err(e));
                        }

                        continue 'outer;
                    }
                }
            }

            return Some(Ok(c));
        }
    }
}

#[derive(Clone)]
enum Component<'a> {
    CurDir,
    ParentDir,
    Normal(Fragment<'a>),
}

fn compile_pattern<P>(pattern: &P) -> Vec<Component<'_>>
where
    P: ?Sized + AsRef<RelativePath>,
{
    let pattern = pattern.as_ref();

    let mut output = Vec::new();

    for c in pattern.components() {
        output.push(match c {
            relative_path::Component::CurDir => Component::CurDir,
            relative_path::Component::ParentDir => Component::ParentDir,
            relative_path::Component::Normal(normal) => Component::Normal(Fragment::parse(normal)),
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
struct Fragment<'a> {
    parts: Box<[Part<'a>]>,
}

impl<'a> Fragment<'a> {
    fn parse(string: &'a str) -> Fragment<'a> {
        let mut literal = true;
        let mut parts = Vec::new();
        let mut start = None;

        for (n, c) in string.char_indices() {
            match c {
                '*' => {
                    if let Some(s) = start.take() {
                        parts.push(Part::Literal(&string[s..n]));
                    }

                    if mem::take(&mut literal) {
                        parts.push(Part::Star);
                    }
                }
                _ => {
                    if start.is_none() {
                        start = Some(n);
                    }

                    literal = true;
                }
            }
        }

        if let Some(s) = start {
            parts.push(Part::Literal(&string[s..]));
        }

        Fragment {
            parts: parts.into(),
        }
    }

    /// Test if the given string matches the current fragment.
    fn is_match(&self, string: &str) -> bool {
        let mut backtrack = VecDeque::new();
        backtrack.push_back((self.parts.as_ref(), string));

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
}