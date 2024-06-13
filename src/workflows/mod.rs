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

        Ok(Some(Workflow { path, doc }))
    }
}

pub(crate) struct Workflow {
    pub(crate) path: RelativePathBuf,
    pub(crate) doc: yaml::Document,
}

impl Workflow {
    /// Iterate over all jobs.
    pub(crate) fn jobs(&self) -> impl Iterator<Item = (&BStr, Job<'_>)> {
        self.doc.as_ref().as_mapping().into_iter().flat_map(|m| {
            m.get("jobs")
                .and_then(|jobs| jobs.as_mapping())
                .into_iter()
                .flatten()
                .flat_map(|(key, job)| Some((key, Job::new(job.as_mapping()?))))
        })
    }

    /// Get the root level environment variables.
    pub(crate) fn env(&self, eval: &Eval<'_>) -> Result<BTreeMap<String, String>> {
        let mut env = BTreeMap::new();

        if let Some(root) = self.doc.as_ref().as_mapping() {
            if let Some(m) = root.get("env").and_then(|v| v.as_mapping()) {
                for (key, value) in m {
                    let Some(value) = value.as_str() else {
                        continue;
                    };

                    let value = eval.eval(value)?;
                    env.insert(key.to_string(), value.into_owned());
                }
            }
        }

        Ok(env)
    }
}

pub(crate) struct Job<'a> {
    pub(crate) value: yaml::Mapping<'a>,
}

impl<'a> Job<'a> {
    pub(crate) fn new(value: yaml::Mapping<'a>) -> Self {
        Self { value }
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

#[derive(Clone, Copy)]
pub(crate) struct Eval<'a> {
    pub(crate) matrix: Option<&'a Matrix>,
    env: Option<&'a BTreeMap<String, String>>,
}

impl<'a> Eval<'a> {
    pub(crate) const fn new() -> Eval<'a> {
        Self {
            matrix: None,
            env: None,
        }
    }

    /// Get the environment associated with the evaluation.
    pub(crate) fn env(&self) -> BTreeMap<String, String> {
        self.env.cloned().unwrap_or_default()
    }

    /// Associate a matrix with the evaluation.
    pub(crate) fn with_matrix(self, matrix: &'a Matrix) -> Self {
        Self {
            matrix: Some(matrix),
            ..self
        }
    }

    /// Associate an environment with the evaluation.
    pub(crate) fn with_env(self, env: &'a BTreeMap<String, String>) -> Self {
        Self {
            env: Some(env),
            ..self
        }
    }

    /// Evaluate a string with matrix variables.
    pub(crate) fn eval<'s>(&self, s: &'s str) -> Result<Cow<'s, str>> {
        let Some(i) = s.find("${{") else {
            return Ok(Cow::Borrowed(s));
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

            let expr = rest[..end].trim();

            match self.expr(expr)? {
                Expr::String(s) => result.push_str(s.as_ref()),
                Expr::Bool(b) => {
                    result.push_str(if b { "true" } else { "false" });
                }
                Expr::Null => {}
            }

            s = &rest[end + 2..];
        }

        Ok(Cow::Owned(result))
    }

    pub(crate) fn expr<'expr>(&'expr self, source: &'expr str) -> Result<Expr<'expr>> {
        let mut p = parsing::Parser::new(source);
        grammar::root(&mut p)?;
        let tree = p.tree.build()?;

        let mut it = self::eval::eval(&tree, source, self);

        let Some(expr) = it.next() else {
            bail!("No expressions");
        };

        if it.next().is_some() {
            bail!("Multiple expressions");
        }

        match expr {
            Ok(expr) => Ok(expr),
            Err(e) => {
                bail!("{e}: {}", &source[e.span.range()]);
            }
        }
    }

    pub(crate) fn test(&self, source: &str) -> Result<bool> {
        match self.expr(source)? {
            Expr::Bool(b) => Ok(b),
            _ => bail!("Expected boolean expression"),
        }
    }

    /// Get a variable from the matrix.
    fn get(&self, key: &str) -> Option<&'a str> {
        let (key, value) = key.split_once('.')?;

        match (key.trim(), value.trim()) {
            ("env", key) => self.env?.get(key).map(String::as_str),
            ("matrix", key) => self.matrix?.matrix.get(key).map(String::as_str),
            ("github", key) => {
                let (what, rest) = key.split_once('.')?;

                match what {
                    "event" => self.event(rest),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    fn event(&self, _: &str) -> Option<&'a str> {
        // TODO: Support setting event variables.
        None
    }
}

/// A matrix of variables.
pub(crate) struct Matrix {
    matrix: BTreeMap<String, String>,
}

impl Matrix {
    /// Create a new matrix.
    #[cfg(test)]
    pub(crate) fn new() -> Self {
        Self {
            matrix: BTreeMap::new(),
        }
    }

    /// Insert a value into the matrix.
    #[cfg(test)]
    pub(crate) fn insert<K, V>(&mut self, key: K, value: V)
    where
        K: AsRef<str>,
        V: AsRef<str>,
    {
        self.matrix
            .insert(key.as_ref().to_owned(), value.as_ref().to_owned());
    }

    /// Test if the matrix is empty.
    #[inline]
    pub(crate) fn is_empty(&self) -> bool {
        self.matrix.is_empty()
    }

    #[inline]
    pub(crate) fn display(&self) -> Display<'_> {
        Display {
            matrix: &self.matrix,
        }
    }
}

pub(crate) struct Display<'a> {
    matrix: &'a BTreeMap<String, String>,
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut it = self.matrix.iter();

        write!(f, "{{")?;

        if let Some((key, value)) = it.next() {
            write!(f, "{}={}", escape(key), escape(value))?;

            for (key, value) in it {
                write!(f, ", {}={}", escape(key), escape(value))?;
            }
        }

        write!(f, "}}")?;
        Ok(())
    }
}

fn escape(s: &str) -> Escape<'_> {
    Escape { s }
}

struct Escape<'a> {
    s: &'a str,
}

impl fmt::Display for Escape<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self
            .s
            .contains(|c: char| !matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_'))
        {
            write!(f, "\"{}\"", self.s)
        } else {
            write!(f, "{}", self.s)
        }
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
        let mut matrix = Matrix::new();
        matrix.insert("a", "1");
        matrix.insert("b", "2");

        let env = BTreeMap::new();

        let eval = Eval::new().with_env(&env).with_matrix(&matrix);

        assert!(eval.test("matrix.a == '1'").unwrap());
        assert!(eval.test("matrix.a != '2'").unwrap());
        assert!(eval.test("matrix.a != matrix.b").unwrap());
        assert!(!eval.test("matrix.a == matrix.b").unwrap());
        assert!(eval
            .test("matrix.a == matrix.b || matrix.a != matrix.b")
            .unwrap());
    }

    #[test]
    fn or_test() {
        let mut matrix = Matrix::new();
        matrix.insert("bar", "right");

        let eval = Eval::new().with_matrix(&matrix);

        assert_eq!(
            eval.expr("matrix.foo || matrix.bar").unwrap(),
            Expr::String(Cow::Borrowed("right"))
        );
    }

    #[test]
    fn and_test() {
        let mut matrix = Matrix::new();
        matrix.insert("foo", "wrong");
        matrix.insert("bar", "right");

        let eval = Eval::new().with_matrix(&matrix);

        assert_eq!(
            eval.expr("matrix.foo && matrix.bar").unwrap(),
            Expr::String(Cow::Borrowed("right"))
        );

        assert_eq!(eval.expr("matrix.baz && matrix.bar").unwrap(), Expr::Null);
    }
}
