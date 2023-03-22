use std::collections::VecDeque;
use std::io;
use std::path::Path;

use relative_path::{Component, RelativePath, RelativePathBuf};

/// A compiled glob expression.
pub struct Glob<'a> {
    root: &'a Path,
    queue: VecDeque<(RelativePathBuf, RelativePathBuf)>,
}

impl<'a> Glob<'a> {
    /// Construct a new glob pattern.
    pub fn new<R, P>(root: &'a R, pattern: P) -> Self
    where
        R: ?Sized + AsRef<Path>,
        P: AsRef<RelativePath>,
    {
        Self {
            root: root.as_ref(),
            queue: [(RelativePathBuf::new(), pattern.as_ref().to_owned())]
                .into_iter()
                .collect(),
        }
    }

    fn expand<M>(
        &mut self,
        current: &RelativePathBuf,
        rest: &RelativePath,
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
            self.queue.push_back((new, rest.to_owned()));
        }

        Ok(())
    }
}

impl<'a> Iterator for Glob<'a> {
    type Item = io::Result<RelativePathBuf>;

    fn next(&mut self) -> Option<Self::Item> {
        'outer: loop {
            let (mut c, p) = self.queue.pop_front()?;
            let mut iter = p.components();

            while let Some(component) = iter.next() {
                match component {
                    Component::CurDir => {}
                    Component::ParentDir => {}
                    Component::Normal("*") => {
                        if let Err(e) = self.expand(&c, iter.as_relative_path(), |_| true) {
                            return Some(Err(e));
                        }

                        continue 'outer;
                    }
                    Component::Normal(normal) => match split(normal) {
                        Some(("*", "*")) => {
                            if let Err(e) =
                                self.expand(&c, iter.as_relative_path(), |n| split(n).is_some())
                            {
                                return Some(Err(e));
                            }
                        }
                        Some(("*", ext)) => {
                            if let Err(e) = self.expand(
                                &c,
                                iter.as_relative_path(),
                                |n| matches!(split(n), Some((_, e)) if e == ext),
                            ) {
                                return Some(Err(e));
                            }
                        }
                        Some((base, "*")) => {
                            if let Err(e) = self.expand(
                                &c,
                                iter.as_relative_path(),
                                |n| matches!(split(n), Some((b, _)) if b == base),
                            ) {
                                return Some(Err(e));
                            }
                        }
                        _ => {
                            c.push(normal);
                        }
                    },
                }
            }

            return Some(Ok(c));
        }
    }
}

fn split(string: &str) -> Option<(&str, &str)> {
    let n = string.rfind('.')?;
    Some((&string[..n], &string[n + 1..]))
}
