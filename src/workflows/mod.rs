mod eval;
mod grammar;
mod lexer;
mod parsing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
enum Syntax {
    Variable,
    Eq,
    Neq,
    And,
    Or,
    SingleString,
    DoubleString,
    Whitespace,
    Operator,
    OpenParen,
    CloseParen,

    // An operation.
    Operation,
    // Precedence group.
    Group,
    // Enf of file.
    Eof,
    Error,
}

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fmt;
use std::fs;
use std::io;
use std::str;

use anyhow::{anyhow, bail, Context, Result};
use bstr::BStr;
use nondestructive::yaml;
use relative_path::{RelativePath, RelativePathBuf};

use crate::ctxt::Ctxt;
use crate::model::Repo;

use self::eval::Expr;

pub struct Workflows<'cx> {
    cx: &'cx Ctxt<'cx>,
    path: RelativePathBuf,
    ids: BTreeSet<String>,
}

impl<'cx> Workflows<'cx> {
    /// Open the workflows directory in the specified repo.
    pub(crate) fn new(cx: &'cx Ctxt<'cx>, repo: &Repo) -> Result<Self> {
        let path = repo.path().join(".github").join("workflows");
        let ids = list_workflow_ids(cx, &path)?;
        Ok(Self { cx, path, ids })
    }

    /// Get ids of existing workflows.
    pub(crate) fn ids(&self) -> impl Iterator<Item = &str> {
        self.ids.iter().map(|s| s.as_str())
    }

    /// Return the expected workflow path.
    pub(crate) fn path(&self, id: &str) -> RelativePathBuf {
        let mut path = self.path.clone();
        path.push(id);
        path.set_extension("yml");
        path
    }

    /// Open a workflow by id.
    pub(crate) fn open(&self, id: &str) -> Result<Option<Workflow>> {
        let path = self.path(id);
        let p = self.cx.to_path(&path);

        let bytes = match fs::read(&p) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| anyhow!("{}", p.display())),
        };

        let doc = yaml::from_slice(bytes).with_context(|| anyhow!("{}", p.display()))?;
        Ok(Some(Workflow {
            cx: self.cx,
            path,
            doc,
        }))
    }
}

pub(crate) struct Workflow<'cx> {
    cx: &'cx Ctxt<'cx>,
    pub(crate) path: RelativePathBuf,
    pub(crate) doc: yaml::Document,
}

impl<'cx> Workflow<'cx> {
    /// Iterate over all jobs.
    pub(crate) fn jobs(&self) -> impl Iterator<Item = (&BStr, Job<'cx, '_>)> {
        self.doc.as_ref().as_mapping().into_iter().flat_map(|m| {
            m.get("jobs")
                .and_then(|jobs| jobs.as_mapping())
                .into_iter()
                .flatten()
                .flat_map(|(key, job)| Some((key, Job::new(self.cx, job.as_mapping()?))))
        })
    }
}

pub(crate) struct Job<'cx, 'a> {
    #[allow(unused)]
    cx: &'cx Ctxt<'cx>,
    pub(crate) value: yaml::Mapping<'a>,
}

impl<'cx, 'a> Job<'cx, 'a> {
    pub(crate) fn new(cx: &'cx Ctxt<'cx>, value: yaml::Mapping<'a>) -> Self {
        Self { cx, value }
    }

    /// Get the runs-on value.
    pub(crate) fn runs_on(&self) -> Result<&str> {
        self.value
            .get("runs-on")
            .and_then(|runs_on| runs_on.as_str())
            .ok_or_else(|| anyhow!("Missing runs-on"))
    }

    /// Iterate over all matrices.
    pub(crate) fn matrices(&self, ignore: &HashSet<String>) -> Result<Vec<Matrix>> {
        let mut matrices = Vec::new();
        let mut variables = Vec::new();

        'bail: {
            let Some(strategy) = self.value.get("strategy").and_then(|s| s.as_mapping()) else {
                break 'bail;
            };

            let Some(matrix) = strategy
                .get("matrix")
                .and_then(|matrix| matrix.as_mapping())
            else {
                break 'bail;
            };

            for (key, value) in matrix {
                let key = str::from_utf8(key).context("matrix key")?;

                if ignore.contains(key) {
                    tracing::trace!("Ignoring matrix variable `{key}`");
                    continue;
                }

                let mut values = Vec::new();

                match value.into_any() {
                    yaml::Any::Sequence(sequence) => {
                        for value in sequence {
                            values.extend(value.as_str());
                        }
                    }
                    yaml::Any::Scalar(value) => {
                        values.extend(value.as_str());
                    }
                    _ => {}
                }

                variables.push((key, values));
            }
        };

        let mut positions = vec![0usize; variables.len()];

        'outer: loop {
            let mut matrix = BTreeMap::new();

            for (n, &p) in positions.iter().enumerate() {
                let (key, values) = &variables[n];
                matrix.insert(key.to_string(), values[p].to_owned());
            }

            matrices.push(Matrix { matrix });

            for (p, (_, values)) in positions.iter_mut().zip(&variables) {
                *p += 1;

                if *p < values.len() {
                    continue 'outer;
                }

                *p = 0;
            }

            break;
        }

        if matrices.is_empty() {
            matrices.push(Matrix {
                matrix: BTreeMap::new(),
            });
        }

        Ok(matrices)
    }
}

/// A matrix of variables.
pub(crate) struct Matrix {
    matrix: BTreeMap<String, String>,
}

impl Matrix {
    /// Test if the matrix is empty.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.matrix.is_empty()
    }

    /// Get a variable from the matrix.
    fn get(&self, key: &str) -> Option<&str> {
        let Some(("matrix", key)) = key.split_once('.') else {
            return None;
        };

        self.matrix.get(key).map(String::as_str)
    }

    /// Evaluate a string with matrix variables.
    pub(crate) fn eval<'a>(&self, s: &'a str) -> Cow<'a, str> {
        let Some(i) = s.find("${{") else {
            return Cow::Borrowed(s);
        };

        let mut result = String::new();
        let (head, mut s) = s.split_at(i);
        result.push_str(head);

        loop {
            let Some(rest) = s.strip_prefix("${{") else {
                let mut it = s.chars();

                let Some(c) = it.next() else {
                    break;
                };

                result.push(c);
                s = it.as_str();
                continue;
            };

            let Some(end) = rest.find("}}") else {
                break;
            };

            let variable = rest[..end].trim();
            let variable = variable
                .split_once('.')
                .map(|(key, value)| (key.trim(), value.trim()));

            if let Some(("matrix", variable)) = variable {
                if let Some(value) = self.matrix.get(variable) {
                    result.push_str(value);
                }
            }

            s = &rest[end + 2..];
        }

        Cow::Owned(result)
    }

    pub(crate) fn test(&self, source: &str) -> Result<bool> {
        let mut p = parsing::Parser::new(source);
        grammar::root(&mut p)?;
        let tree = p.tree.build()?;

        let mut it = self::eval::eval(&tree, source, self);

        let Some(result) = it.next() else {
            bail!("No expressions");
        };

        match result? {
            Expr::Bool(b) => Ok(b),
            _ => bail!("Expected boolean result"),
        }
    }
}

impl fmt::Debug for Matrix {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.matrix.fmt(f)
    }
}

/// List existing workflow identifiers.
fn list_workflow_ids(cx: &Ctxt<'_>, path: &RelativePath) -> Result<BTreeSet<String>> {
    let mut ids = BTreeSet::new();

    let path = cx.to_path(path);

    for e in fs::read_dir(&path).with_context(|| path.display().to_string())? {
        let entry = e.with_context(|| path.display().to_string())?;
        let path = entry.path();

        if let Some(id) = path.file_stem().and_then(|s| s.to_str()) {
            ids.insert(id.to_owned());
        }
    }

    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_test() {
        let matrix = Matrix {
            matrix: vec![
                ("a".to_owned(), "1".to_owned()),
                ("b".to_owned(), "2".to_owned()),
            ]
            .into_iter()
            .collect(),
        };

        assert!(matrix.test("matrix.a == '1'").unwrap());
        assert!(matrix.test("matrix.a != '2'").unwrap());
        assert!(matrix.test("matrix.a != matrix.b").unwrap());
        assert!(!matrix.test("matrix.a == matrix.b").unwrap());
        assert!(matrix
            .test("matrix.a == matrix.b || matrix.a != matrix.b")
            .unwrap());
    }
}
