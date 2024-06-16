#[cfg(test)]
mod tests;

mod eval;
mod grammar;
mod lexer;
mod parsing;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub(crate) enum Syntax {
    Number,
    Bool,
    Null,
    Ident,
    Star,
    Comma,
    Dot,
    Not,
    Eq,
    Neq,
    And,
    Or,
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    SingleString,
    DoubleString,
    Whitespace,
    Operator,
    // `(`.
    OpenParen,
    // `)`.
    CloseParen,
    // `${{`.
    OpenExpr,
    // `}}`.
    CloseExpr,
    // A lookup expression `<ident> [<dot> <ident>]*`.
    Lookup,
    // A function call.
    Function,
    // A unary operation.
    Unary,
    // A binary operation.
    Binary,
    // Precedence group.
    Group,
    // Enf of file.
    Eof,
    // An error.
    Error,
}

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fmt;
use std::fs;
use std::io;
use std::str;

use anyhow::{anyhow, bail, Context, Result};
use nondestructive::yaml;
use relative_path::{RelativePath, RelativePathBuf};
use syntree::Span;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::rstr::{RStr, RString};

use self::eval::Expr;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ExprError {
    NoExpressions(Box<str>),
    MultipleExpressions(Box<str>),
    EvalError(eval::EvalError, Box<str>),
    SynTree(syntree::Error),
}

impl From<syntree::Error> for ExprError {
    fn from(e: syntree::Error) -> Self {
        Self::SynTree(e)
    }
}

impl fmt::Display for ExprError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ExprError::NoExpressions(ref source) => write!(f, "No expressions: {source}"),
            ExprError::MultipleExpressions(ref source) => {
                write!(f, "Multiple expressions: {source}")
            }
            ExprError::EvalError(ref e, ref source) => {
                write!(f, "Evaluation error: {e}: `{source}`")
            }
            ExprError::SynTree(..) => write!(f, "Syntax tree error"),
        }
    }
}

impl std::error::Error for ExprError {
    #[inline]
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            ExprError::SynTree(ref e) => Some(e),
            _ => None,
        }
    }
}

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
    pub(crate) fn workflows(&self) -> impl Iterator<Item = Result<Workflow>> + '_ {
        self.ids
            .iter()
            .flat_map(|s| self.open(s.as_str()).transpose())
    }

    /// Return the expected workflow path.
    pub(crate) fn path(&self, id: &str) -> RelativePathBuf {
        let mut path = self.path.clone();
        path.push(id);
        path.set_extension("yml");
        path
    }

    /// Open a workflow by id.
    fn open(&self, id: &str) -> Result<Option<Workflow>> {
        let path = self.path(id);
        let p = self.cx.to_path(&path);

        let bytes = match fs::read(&p) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).with_context(|| anyhow!("{}", p.display())),
        };

        let doc = yaml::from_slice(bytes)
            .with_context(|| anyhow!("{}: Reading YAML file", p.display()))?;

        Ok(Some(Workflow {
            cx: self.cx,
            id: id.to_owned(),
            path,
            doc,
        }))
    }
}

fn build_job(
    name: &str,
    value: yaml::Mapping<'_>,
    ignore: &HashSet<String>,
    eval: &Eval<'_>,
) -> Result<Job> {
    let runs_on = value
        .get("runs-on")
        .and_then(|value| value.as_str())
        .context("Missing runs-on")?;

    let name = value
        .get("name")
        .and_then(|value| Some(eval.eval(value.as_str()?)))
        .transpose()?
        .unwrap_or(Cow::Borrowed(RStr::new(name)));

    let mut matrices = Vec::new();

    for matrix in build_matrices(&value, ignore, eval)? {
        let mut tree = eval.tree().clone();
        tree.insert_prefix("matrix", matrix.matrix.clone());
        let eval = eval.with_tree(&tree);

        let mut steps = Vec::new();

        if let Some(s) = value.get("steps").and_then(|steps| steps.as_sequence()) {
            let mut tree = eval.tree().clone();
            tree.insert_prefix("env", extract_env(&eval, &value)?);
            let eval = eval.with_tree(&tree);

            for s in s {
                let Some(value) = s.as_mapping() else {
                    continue;
                };

                let mut tree = eval.tree().clone();
                tree.insert_prefix("env", extract_env(&eval, &value)?);
                let eval = eval.with_tree(&tree);

                let working_directory =
                    match value.get("working-directory").and_then(|v| v.as_str()) {
                        Some(dir) => Some(eval.eval(dir)?.into_owned()),
                        None => None,
                    };

                let mut skipped = None;
                let mut condition = None;

                if let Some((id, expr)) = value.get("if").and_then(|v| Some((v.id(), v.as_str()?)))
                {
                    if !eval.test(expr)? {
                        skipped = Some(expr.to_owned());
                    }

                    condition = Some((id, expr.to_owned()));
                }

                let uses = if let Some((id, uses)) =
                    value.get("uses").and_then(|v| Some((v.id(), v.as_str()?)))
                {
                    Some((id, eval.eval(uses)?.into_owned()))
                } else {
                    None
                };

                let mut with = BTreeMap::new();

                if let Some(mapping) = value.get("with").and_then(|v| v.as_mapping()) {
                    for (key, value) in mapping {
                        let (Ok(key), Some(value)) = (str::from_utf8(key), value.as_str()) else {
                            continue;
                        };

                        with.insert(key.to_owned(), eval.eval(value)?.into_owned());
                    }
                }

                let name = value
                    .get("name")
                    .and_then(|v| Some(eval.eval(v.as_str()?)))
                    .transpose()?;

                let run = value
                    .get("run")
                    .and_then(|v| Some(eval.eval(v.as_str()?)))
                    .transpose()?;

                steps.push(Step {
                    id: value.id(),
                    env: eval.tree().get_prefix("env"),
                    working_directory,
                    skipped,
                    condition,
                    uses,
                    with,
                    name: name.map(Cow::into_owned),
                    run: run.map(Cow::into_owned),
                })
            }
        };

        let steps = Steps {
            runs_on: eval.eval(runs_on)?.into_owned(),
            steps,
        };

        matrices.push((matrix, steps));
    }

    Ok(Job {
        name: name.into_owned(),
        matrices,
    })
}

/// Iterate over all matrices.
pub(crate) fn build_matrices(
    value: &yaml::Mapping<'_>,
    ignore: &HashSet<String>,
    eval: &Eval<'_>,
) -> Result<Vec<Matrix>> {
    let mut matrices = Vec::new();
    let mut included = Vec::new();
    let mut variables = Vec::new();

    'bail: {
        let Some(strategy) = value.get("strategy").and_then(|s| s.as_mapping()) else {
            break 'bail;
        };

        let Some(matrix) = strategy
            .get("matrix")
            .and_then(|matrix| matrix.as_mapping())
        else {
            break 'bail;
        };

        for (key, value) in matrix {
            let key = str::from_utf8(key).context("Bad matrix key")?;

            if key == "include" {
                for (index, mapping) in value
                    .as_sequence()
                    .into_iter()
                    .flatten()
                    .flat_map(|v| v.as_mapping())
                    .enumerate()
                {
                    let mut matrix = Matrix::new();

                    for (key, value) in mapping {
                        let id = value.id();
                        let key = str::from_utf8(key).context("Bad matrix key")?;
                        let value = value.as_str().with_context(|| {
                            anyhow!(".include[{index}][{key}]: Value must be a string")
                        })?;
                        let value = eval.eval(value)?;
                        matrix.insert_with_id(key, value, id);
                    }

                    included.push(matrix);
                }

                continue;
            }

            if ignore.contains(key) {
                continue;
            }

            let mut values = Vec::new();

            match value.into_any() {
                yaml::Any::Sequence(sequence) => {
                    for value in sequence {
                        let id = value.id();

                        if let Some(value) = value.as_str() {
                            values.push((eval.eval(value)?.into_owned(), id));
                        }
                    }
                }
                yaml::Any::Scalar(value) => {
                    let id = value.id();

                    if let Some(value) = value.as_str() {
                        values.push((eval.eval(value)?.into_owned(), id));
                    }
                }
                _ => {}
            }

            if !values.is_empty() {
                variables.push((key, values));
            }
        }
    };

    let mut positions = vec![0usize; variables.len()];

    'outer: loop {
        let mut matrix = Matrix::new();

        for (n, &p) in positions.iter().enumerate() {
            let (key, ref values) = variables[n];
            let (ref value, id) = values[p];
            matrix.insert_with_id(key, value, id);
        }

        matrices.push(matrix);

        for (p, (_, values)) in positions.iter_mut().zip(&variables) {
            *p += 1;

            if *p < values.len() {
                continue 'outer;
            }

            *p = 0;
        }

        break;
    }

    matrices.extend(included);

    if matrices.is_empty() {
        matrices.push(Matrix::new());
    }

    Ok(matrices)
}

fn extract_env(eval: &Eval<'_>, m: &yaml::Mapping<'_>) -> Result<BTreeMap<String, RString>> {
    let mut env = BTreeMap::new();

    let Some(m) = m.get("env").and_then(|v| v.as_mapping()) else {
        return Ok(env);
    };

    for (key, value) in m {
        let key = str::from_utf8(key).context("Decoding key")?;

        let Some(value) = value.as_str() else {
            continue;
        };

        let value = eval.eval(value)?;
        env.insert(key.to_owned(), value.into_owned());
    }

    Ok(env)
}

pub(crate) struct Workflow<'cx> {
    cx: &'cx Ctxt<'cx>,
    pub(crate) id: String,
    pub(crate) path: RelativePathBuf,
    pub(crate) doc: yaml::Document,
}

impl Workflow<'_> {
    /// Get the identifier of the workflow.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// Iterate over all jobs.
    pub(crate) fn jobs(&self, ignore: &HashSet<String>) -> Result<Vec<Job>> {
        let Some(mapping) = self.doc.as_ref().as_mapping() else {
            bail!(
                "{}: Root is not a mapping",
                self.cx.to_path(&self.path).display()
            );
        };

        let functions = default_functions();
        let mut tree = Tree::new();

        if let Some(auth) = self.cx.github_auth() {
            if let Some(owned) = RString::redacted(auth.as_secret()) {
                tree.insert_prefix("secrets", vec![("GITHUB_TOKEN".to_owned(), owned)]);
            }
        }

        let eval = Eval::new().with_functions(&functions);
        tree.insert_prefix("env", extract_env(&eval, &mapping)?);
        let eval = eval.with_tree(&tree);

        let jobs = mapping
            .get("jobs")
            .and_then(|jobs| jobs.as_mapping())
            .into_iter()
            .flatten()
            .flat_map(|(key, job)| Some((key, job.as_mapping()?)));

        let mut outputs = Vec::new();

        for (name, job) in jobs {
            let name = str::from_utf8(name).with_context(|| {
                anyhow!(
                    "{}: Decoding job name",
                    self.cx.to_path(&self.path).display()
                )
            })?;

            outputs.push(build_job(name, job, ignore, &eval).with_context(|| {
                anyhow!(
                    "{}: Building job `{name}`",
                    self.cx.to_path(&self.path).display()
                )
            })?);
        }

        Ok(outputs)
    }
}

pub(crate) struct Job {
    pub(crate) name: RString,
    pub(crate) matrices: Vec<(Matrix, Steps)>,
}

pub(crate) struct Steps {
    pub(crate) runs_on: RString,
    pub(crate) steps: Vec<Step>,
}

pub(crate) struct Step {
    pub(crate) id: yaml::Id,
    pub(crate) env: BTreeMap<String, RString>,
    pub(crate) working_directory: Option<RString>,
    pub(crate) skipped: Option<String>,
    pub(crate) condition: Option<(yaml::Id, String)>,
    pub(crate) uses: Option<(yaml::Id, RString)>,
    pub(crate) with: BTreeMap<String, RString>,
    pub(crate) name: Option<RString>,
    pub(crate) run: Option<RString>,
}

impl Step {
    /// Construct an environment from a step.
    pub(crate) fn env(&self) -> &BTreeMap<String, RString> {
        &self.env
    }
}

pub fn default_functions() -> BTreeMap<&'static str, CustomFunction> {
    let mut functions = BTreeMap::new();
    functions.insert("fromJSON", from_json as CustomFunction);
    functions
}

fn value_to_expr(
    span: &Span<u32>,
    value: serde_json::Value,
) -> Result<Expr<'static>, eval::EvalError> {
    let expr = match value {
        serde_json::Value::Null => Expr::Null,
        serde_json::Value::Bool(b) => Expr::Bool(b),
        serde_json::Value::Number(n) => {
            let Some(v) = n.as_f64() else {
                return Err(eval::EvalError::custom(
                    *span,
                    "Failed to convert JSON number",
                ));
            };

            Expr::Float(v)
        }
        serde_json::Value::String(string) => Expr::String(Cow::Owned(RString::from(string))),
        serde_json::Value::Array(array) => {
            let mut values = Vec::with_capacity(array.len());

            for value in array {
                values.push(value_to_expr(span, value)?);
            }

            Expr::Array(values.into())
        }
        serde_json::Value::Object(_) => {
            return Err(eval::EvalError::custom(*span, "Objects are not supported"));
        }
    };

    Ok(expr)
}

fn from_json<'m>(span: &Span<u32>, args: &[Expr<'m>]) -> Result<Expr<'m>, eval::EvalError> {
    let Some(s) = args.first().and_then(|s| s.as_str()) else {
        return Ok(Expr::Null);
    };

    // NB: Figure out if we want to carry redaction into the decoded expression.
    let string = s.to_redacted();

    match serde_json::from_str(string.as_ref()) {
        Ok(value) => value_to_expr(span, value),
        Err(error) => Err(eval::EvalError::custom(*span, format_args!("{error}"))),
    }
}

type CustomFunction = for<'m> fn(&Span<u32>, &[Expr<'m>]) -> Result<Expr<'m>, eval::EvalError>;

#[derive(Clone, Default)]
struct Node {
    value: Option<RString>,
    children: Option<BTreeMap<String, Node>>,
}

impl Node {
    const EMPTY: Node = Node {
        value: None,
        children: None,
    };
}

#[derive(Clone, Default)]
pub(crate) struct Tree {
    root: Node,
}

impl Tree {
    /// Create a new tree.
    pub(crate) const fn new() -> Self {
        Tree { root: Node::EMPTY }
    }

    /// Insert a prefix into the current tree.
    fn insert_prefix<V>(&mut self, prefix: &str, vars: V)
    where
        V: IntoIterator<Item = (String, RString)>,
    {
        let child = self
            .root
            .children
            .get_or_insert_with(BTreeMap::new)
            .entry(prefix.to_owned())
            .or_default();

        for (key, value) in vars {
            child
                .children
                .get_or_insert_with(BTreeMap::new)
                .entry(key)
                .or_default()
                .value = Some(value);
        }
    }

    /// Get a prefix tree.
    fn get_prefix(&self, prefix: &str) -> BTreeMap<String, RString> {
        let mut output = BTreeMap::new();

        let Some(children) = &self.root.children else {
            return output;
        };

        let Some(children) = children.get(prefix).and_then(|n| n.children.as_ref()) else {
            return output;
        };

        for (key, node) in children {
            if let Some(value) = &node.value {
                output.insert(key.clone(), value.clone());
            }
        }

        output
    }

    /// Insert a value into the tree.
    #[cfg(test)]
    pub(crate) fn insert<I, V>(&mut self, keys: I, value: V)
    where
        I: IntoIterator<Item: AsRef<str>>,
        V: AsRef<str>,
    {
        let mut current = &mut self.root;

        for key in keys {
            let key = key.as_ref();
            current = current
                .children
                .get_or_insert_with(BTreeMap::new)
                .entry(key.to_owned())
                .or_default();
        }

        current.value = Some(value.as_ref().into());
    }

    /// Get a value from the tree.
    pub(crate) fn get(&self, key: &[&str]) -> Vec<&RStr> {
        let mut output = Vec::new();

        let mut queue = VecDeque::new();
        queue.push_back((&self.root, key));

        while let Some((current, keys)) = queue.pop_front() {
            let Some((head, tail)) = keys.split_first() else {
                output.extend(current.value.as_deref());
                continue;
            };

            let Some(children) = &current.children else {
                continue;
            };

            if *head == "*" {
                queue.extend(children.values().map(|n| (n, tail)));
            } else {
                queue.extend(children.get(*head).map(|n| (n, tail)));
            }
        }

        output
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Eval<'a> {
    tree: Option<&'a Tree>,
    functions: Option<&'a BTreeMap<&'static str, CustomFunction>>,
}

impl<'a> Eval<'a> {
    pub(crate) const fn new() -> Eval<'a> {
        Self {
            tree: None,
            functions: None,
        }
    }

    /// Associate function with the current environment.
    pub(crate) fn with_functions(
        self,
        functions: &'a BTreeMap<&'static str, CustomFunction>,
    ) -> Self {
        Self {
            functions: Some(functions),
            ..self
        }
    }

    /// Get a function by name.
    pub(crate) fn function(&self, name: &str) -> Option<CustomFunction> {
        self.functions?.get(name).copied()
    }

    /// Modify the environment with a matrix.
    pub(crate) fn with_tree(self, tree: &'a Tree) -> Self {
        Self {
            tree: Some(tree),
            ..self
        }
    }

    /// Clone the current tree so that it can be modified.
    #[inline]
    pub(crate) fn tree(&self) -> &Tree {
        static EMPTY: Tree = Tree::new();

        let Some(tree) = self.tree else {
            return &EMPTY;
        };

        tree
    }

    /// Evaluate a string with matrix variables.
    pub(crate) fn eval<'s>(&self, s: &'s str) -> Result<Cow<'s, RStr>> {
        use std::fmt::Write;

        let Some(i) = s.find("${{") else {
            return Ok(Cow::Borrowed(RStr::new(s)));
        };

        let mut result = RString::new();
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
                Expr::Array(..) => {}
                Expr::String(s) => result.push_str(s.as_raw()),
                Expr::Float(f) => {
                    write!(result, "{f}").context("Failed to format float")?;
                }
                Expr::Bool(b) => {
                    result.push_str(if b { "true" } else { "false" });
                }
                Expr::Null => {}
            }

            s = &rest[end + 2..];
        }

        Ok(Cow::Owned(result))
    }

    pub(crate) fn expr<'expr>(&'expr self, source: &'expr str) -> Result<Expr<'expr>, ExprError> {
        let mut p = parsing::Parser::new(source);
        grammar::root(&mut p)?;
        let tree = p.tree.build()?;

        let mut it = self::eval::eval(&tree, source, self);

        let Some(expr) = it.next() else {
            return Err(ExprError::NoExpressions(source.into()));
        };

        if it.next().is_some() {
            return Err(ExprError::MultipleExpressions(source.into()));
        }

        match expr {
            Ok(expr) => Ok(expr),
            Err(e) => {
                let source = source[e.span.range()].into();
                Err(ExprError::EvalError(e, source))
            }
        }
    }

    pub(crate) fn test(&self, source: &str) -> Result<bool, ExprError> {
        Ok(self.expr(source)?.as_bool())
    }

    /// Get a variable from the matrix.
    fn lookup<I>(&self, keys: I) -> Vec<&'a RStr>
    where
        I: IntoIterator<Item: AsRef<str>>,
    {
        let Some(tree) = self.tree else {
            return Vec::new();
        };

        let key = keys.into_iter().collect::<Vec<_>>();
        let key = key.iter().map(AsRef::as_ref).collect::<Vec<_>>();
        tree.get(&key)
    }
}

/// A matrix of variables.
#[derive(Clone)]
pub(crate) struct Matrix {
    matrix: BTreeMap<String, RString>,
    ids: BTreeMap<String, yaml::Id>,
}

impl Matrix {
    /// Create a new matrix.
    pub(crate) fn new() -> Self {
        Self {
            matrix: BTreeMap::new(),
            ids: BTreeMap::new(),
        }
    }

    /// Get a value from the matrix.
    pub(crate) fn get_with_id(&self, key: &str) -> Option<(&RStr, yaml::Id)> {
        let value = self.matrix.get(key)?;
        let id = self.ids.get(key)?;
        Some((value, *id))
    }

    /// Insert a value into the matrix.
    pub(crate) fn insert_with_id<K, V>(&mut self, key: K, value: V, id: yaml::Id)
    where
        K: AsRef<str>,
        V: AsRef<RStr>,
    {
        self.matrix
            .insert(key.as_ref().to_owned(), value.as_ref().to_owned());
        self.ids.insert(key.as_ref().to_owned(), id);
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
    matrix: &'a BTreeMap<String, RString>,
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut it = self.matrix.iter();

        write!(f, "{{")?;

        if let Some((key, value)) = it.next() {
            write!(f, "{}={}", Escape::new(key), Escape::new(value))?;

            for (key, value) in it {
                write!(f, ", {}={}", Escape::new(key), Escape::new(value))?;
            }
        }

        write!(f, "}}")?;
        Ok(())
    }
}

struct Escape<'a> {
    s: &'a RStr,
}

impl<'a> Escape<'a> {
    fn new<R>(s: &'a R) -> Self
    where
        R: ?Sized + AsRef<RStr>,
    {
        Escape { s: s.as_ref() }
    }
}

impl fmt::Display for Escape<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self
            .s
            .as_raw()
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
