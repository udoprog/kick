#[cfg(test)]
mod tests;

mod fns;
pub(crate) use self::fns::lookup_function;

mod eval;
mod grammar;
mod lexer;
mod parsing;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet, HashSet, VecDeque};
use std::fmt;
use std::fs;
use std::io;
use std::rc::Rc;
use std::str;

use anyhow::{anyhow, bail, Context, Result};
use nondestructive::yaml;
use relative_path::{RelativePath, RelativePathBuf};
use syntree::Span;

use crate::ctxt::Ctxt;
use crate::model::Repo;
use crate::rstr::{RStr, RString};
use crate::shell::Shell;

use self::eval::{EvalError, Expr};

type CustomFunction = for<'m> fn(&Span<u32>, &[Expr<'m>]) -> Result<Expr<'m>, eval::EvalError>;

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

pub struct WorkflowManifests<'a, 'cx> {
    cx: &'a Ctxt<'cx>,
    path: RelativePathBuf,
    ids: BTreeSet<String>,
}

impl<'a, 'cx> WorkflowManifests<'a, 'cx> {
    /// Open the workflows directory in the specified repo.
    pub(crate) fn new(cx: &'a Ctxt<'cx>, repo: &Repo) -> Result<Self> {
        let path = repo.path().join(".github").join("workflows");
        let ids = list_workflow_ids(cx, &path)?;
        Ok(Self { cx, path, ids })
    }

    /// Get ids of existing workflows.
    pub(crate) fn workflows(&self) -> impl Iterator<Item = Result<WorkflowManifest<'a, 'cx>>> + '_ {
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
    fn open(&self, id: &str) -> Result<Option<WorkflowManifest<'a, 'cx>>> {
        let path = self.path(id);
        let p = self.cx.to_path(&path);

        let bytes = match fs::read(&p) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e).context(p.display().to_string()),
        };

        let doc = yaml::from_slice(bytes)
            .with_context(|| anyhow!("{}: Reading YAML file", p.display()))?;

        Ok(Some(WorkflowManifest {
            cx: self.cx,
            id: id.to_owned(),
            path,
            doc,
        }))
    }
}

fn build_job(
    id: &str,
    value: yaml::Mapping<'_>,
    ignore: &HashSet<String>,
    filter: &[(String, String)],
    eval: &Eval<'_>,
) -> Result<Job> {
    let runs_on = value
        .get("runs-on")
        .and_then(|value| value.as_str())
        .context("Missing runs-on")?;

    let name = value.get("name").and_then(|v| v.as_str());

    let mut matrices = Vec::new();

    for matrix in build_matrices(&value, ignore, filter, eval)? {
        let tree = eval.tree().with_prefix(["matrix"], matrix.matrix.clone());
        let eval = eval.with_tree(&tree);

        let (steps, step_mappings, tree) = load_steps(&value, &eval)?;
        let eval = eval.with_tree(&tree);

        let steps = Steps {
            runs_on: eval.eval(runs_on)?.into_owned(),
            name: name
                .map(|name| eval.eval(name))
                .transpose()?
                .map(Cow::into_owned),
            steps,
            step_mappings,
        };

        matrices.push((matrix, steps));
    }

    Ok(Job {
        id: id.to_owned(),
        name: name.map(str::to_owned),
        matrices,
    })
}

/// Load steps from the given YAML value.
pub(crate) fn load_steps(
    mapping: &yaml::Mapping<'_>,
    eval: &Eval<'_>,
) -> Result<(Vec<Rc<Step>>, Vec<StepMapping>, Rc<Tree>)> {
    let mut steps = Vec::new();
    let mut step_mappings = Vec::new();

    let Some(seq) = mapping.get("steps").and_then(|steps| steps.as_sequence()) else {
        return Ok((steps, step_mappings, Rc::new(eval.tree().clone())));
    };

    let tree = eval
        .tree()
        .with_prefix(["env"], extract_env(eval, mapping)?);
    let tree = Rc::new(tree);
    let eval = eval.with_tree(tree.as_ref());

    for value in seq {
        let Some(value) = value.as_mapping() else {
            continue;
        };

        let env = extract_raw_env(&value)?;
        let working_directory = value.get("working-directory").and_then(|v| v.as_str());

        let mut condition = None;
        let mut condition_mapping = None;

        if let Some((id, expr)) = value.get("if").and_then(|v| Some((v.id(), v.as_str()?))) {
            condition = Some(expr);
            condition_mapping = Some(id);
        }

        let mut uses = None;
        let mut uses_mapping = None;

        if let Some((id, s)) = value.get("uses").and_then(|v| Some((v.id(), v.as_str()?))) {
            uses = Some(eval.eval(s)?.as_rc());
            uses_mapping = Some(id);
        }

        let mut with = BTreeMap::new();

        if let Some(mapping) = value.get("with").and_then(|v| v.as_mapping()) {
            for (key, value) in mapping {
                let (Ok(key), Some(value)) = (str::from_utf8(key), value.as_str()) else {
                    continue;
                };

                with.insert(key.to_owned(), value.to_owned());
            }
        }

        let id = value.get("id").and_then(|v| v.as_str());
        let name = value.get("name").and_then(|v| v.as_str());
        let run = value.get("run").and_then(|v| v.as_str());
        let shell = value.get("shell").and_then(|v| v.as_str());

        steps.push(Rc::new(Step {
            id: id.map(str::to_owned),
            uses,
            tree: tree.clone(),
            env,
            working_directory: working_directory.map(str::to_owned),
            condition: condition.map(str::to_owned),
            with,
            name: name.map(str::to_owned),
            run: run.map(str::to_owned),
            shell: shell.map(str::to_owned),
        }));

        step_mappings.push(StepMapping {
            id: value.id(),
            condition: condition_mapping,
            uses: uses_mapping,
        });
    }

    Ok((steps, step_mappings, tree))
}

/// Iterate over all matrices.
pub(crate) fn build_matrices(
    value: &yaml::Mapping<'_>,
    ignore: &HashSet<String>,
    filter: &[(String, String)],
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

            let filter = |value: &RStr| {
                for (k, v) in filter {
                    if k != key {
                        continue;
                    }

                    if !value.str_eq(v) {
                        return false;
                    }
                }

                true
            };

            let mut values = Vec::new();

            match value.into_any() {
                yaml::Any::Sequence(sequence) => {
                    for value in sequence {
                        let id = value.id();

                        if let Some(value) = value.as_str() {
                            let value = eval.eval(value)?.into_owned();

                            if filter(&value) {
                                values.push((value, id));
                            }
                        }
                    }
                }
                yaml::Any::Scalar(value) => {
                    let id = value.id();

                    if let Some(value) = value.as_str() {
                        let value = eval.eval(value)?.into_owned();

                        if filter(&value) {
                            values.push((value, id));
                        }
                    }
                }
                _ => {}
            }

            if !values.is_empty() {
                variables.push((key, values));
            }
        }
    };

    if !variables.is_empty() {
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

fn extract_raw_env(m: &yaml::Mapping<'_>) -> Result<BTreeMap<String, String>> {
    let mut env = BTreeMap::new();

    let Some(m) = m.get("env").and_then(|v| v.as_mapping()) else {
        return Ok(env);
    };

    for (key, value) in m {
        let key = str::from_utf8(key).context("Decoding key")?;

        let Some(value) = value.as_str() else {
            continue;
        };

        env.insert(key.to_owned(), value.to_owned());
    }

    Ok(env)
}

pub(crate) struct WorkflowManifest<'a, 'cx> {
    cx: &'a Ctxt<'cx>,
    pub(crate) id: String,
    pub(crate) path: RelativePathBuf,
    pub(crate) doc: yaml::Document,
}

impl<'a> WorkflowManifest<'a, '_> {
    /// Get the identifier of the workflow.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// Iterate over all jobs.
    pub(crate) fn jobs(
        &self,
        ignore: &HashSet<String>,
        filter: &[(String, String)],
    ) -> Result<Vec<Job>> {
        let Some(mapping) = self.doc.as_ref().as_mapping() else {
            bail!(
                "{}: Root is not a mapping",
                self.cx.to_path(&self.path).display()
            );
        };

        let mut tree = self.cx.eval.tree().clone();

        tree.insert(["runner", "os"], self.cx.os.as_tree_value());

        if let Some(auth) = self.cx.github_auth() {
            if let Some(owned) = RString::redacted(auth.as_secret()) {
                tree.insert_prefix(["secrets"], vec![("GITHUB_TOKEN".to_owned(), owned)]);
            }
        }

        tree.insert_prefix(["env"], extract_env(&self.cx.eval, &mapping)?);
        let eval = self.cx.eval.with_tree(&tree);

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

            outputs.push(
                build_job(name, job, ignore, filter, &eval).with_context(|| {
                    anyhow!(
                        "{}: Building job `{name}`",
                        self.cx.to_path(&self.path).display()
                    )
                })?,
            );
        }

        Ok(outputs)
    }
}

pub(crate) struct Job {
    pub(crate) id: String,
    #[allow(unused)]
    pub(crate) name: Option<String>,
    pub(crate) matrices: Vec<(Matrix, Steps)>,
}

pub(crate) struct Steps {
    pub(crate) runs_on: RString,
    pub(crate) name: Option<RString>,
    pub(crate) steps: Vec<Rc<Step>>,
    pub(crate) step_mappings: Vec<StepMapping>,
}

pub(crate) struct StepMapping {
    pub(crate) id: yaml::Id,
    pub(crate) condition: Option<yaml::Id>,
    pub(crate) uses: Option<yaml::Id>,
}

#[derive(Default, Debug, Clone)]
pub(crate) struct Step {
    pub(crate) id: Option<String>,
    pub(crate) uses: Option<Rc<RStr>>,
    pub(crate) tree: Rc<Tree>,
    pub(crate) env: BTreeMap<String, String>,
    pub(crate) working_directory: Option<String>,
    pub(crate) condition: Option<String>,
    pub(crate) with: BTreeMap<String, String>,
    pub(crate) name: Option<String>,
    pub(crate) run: Option<String>,
    pub(crate) shell: Option<String>,
}

#[derive(Default, Clone)]
struct Node {
    value: Option<RString>,
    children: BTreeMap<String, Node>,
}

impl Node {
    const fn new() -> Self {
        Self {
            value: None,
            children: BTreeMap::new(),
        }
    }
}

/// A tree used for variable evaluation.
#[derive(Default, Clone)]
pub(crate) struct Tree {
    root: Node,
}

impl Tree {
    /// Construct a new tree.
    pub(crate) const fn new() -> Self {
        Tree { root: Node::new() }
    }

    /// Return a modified clone of the current tree with the given prefix set.
    pub(crate) fn with_prefix<I, V, U>(&self, key: I, vars: V) -> Self
    where
        I: IntoIterator<Item: AsRef<str>>,
        V: IntoIterator<Item = (String, U)>,
        RString: From<U>,
    {
        let mut tree = self.clone();
        tree.insert_prefix(key, vars);
        tree
    }

    /// Extend this tree with another tree.
    pub(crate) fn extend(&mut self, other: &Self) {
        let mut queue = VecDeque::new();

        if self.root.value.is_none() {
            self.root.value.clone_from(&other.root.value);
        }

        queue.push_back((&mut self.root, &other.root));

        while let Some((this, other)) = queue.pop_front() {
            for (key, other) in other.children.iter() {
                let node = this.children.entry(key.clone()).or_default();

                if node.value.is_none() {
                    node.value.clone_from(&other.value);
                }
            }

            for (key, this) in this.children.iter_mut() {
                if let Some(other) = other.children.get(key) {
                    queue.push_back((this, other));
                }
            }
        }
    }

    /// Insert a prefix into the current tree.
    pub(crate) fn insert_prefix<I, V, U>(&mut self, key: I, vars: V)
    where
        I: IntoIterator<Item: AsRef<str>>,
        V: IntoIterator<Item = (String, U)>,
        RString: From<U>,
    {
        let mut current = &mut self.root;

        for key in key {
            let key = key.as_ref();

            current = current.children.entry(key.to_owned()).or_default();
        }

        for (key, value) in vars {
            current.children.entry(key).or_default().value = Some(RString::from(value));
        }
    }

    /// Insert a value into the tree.
    pub(crate) fn insert(
        &mut self,
        keys: impl IntoIterator<Item: AsRef<str>>,
        value: impl AsRef<RStr>,
    ) {
        let mut current = &mut self.root;

        for key in keys {
            let key = key.as_ref();
            current = current.children.entry(key.to_owned()).or_default();
        }

        current.value = Some(value.as_ref().to_owned());
    }

    /// Get a value from the tree.
    pub(crate) fn get<K>(&self, key: K) -> Vec<&RStr>
    where
        K: IntoIterator<Item: AsRef<str>, IntoIter: Clone>,
    {
        let key = key.into_iter();

        let mut output = Vec::new();

        let mut queue = VecDeque::new();
        queue.push_back((&self.root, key));

        while let Some((node, mut keys)) = queue.pop_front() {
            let Some(head) = keys.next() else {
                output.extend(node.value.as_deref());
                continue;
            };

            let head = head.as_ref();

            if head == "*" {
                queue.extend(node.children.values().map(|n| (n, keys.clone())));
            } else {
                queue.extend(node.children.get(head).map(|n| (n, keys.clone())));
            }
        }

        output
    }
}

impl fmt::Debug for Tree {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Tree").finish_non_exhaustive()
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Eval<'a> {
    tree: &'a Tree,
    functions: fn(&str) -> Option<CustomFunction>,
}

impl<'a> Eval<'a> {
    pub(crate) const fn new(tree: &'a Tree) -> Self {
        Self {
            tree,
            functions: lookup_function,
        }
    }

    pub(crate) fn empty() -> Self {
        static EMPTY_TREE: Tree = Tree::new();
        Self::new(&EMPTY_TREE)
    }

    /// Get a function by name.
    pub(crate) fn function(&self, name: &str) -> Option<CustomFunction> {
        (self.functions)(name)
    }

    /// Modify the environment with a matrix.
    pub(crate) fn with_tree(self, tree: &'a Tree) -> Self {
        Self { tree, ..self }
    }

    /// Clone the current tree so that it can be modified.
    #[inline]
    pub(crate) fn tree(&self) -> &Tree {
        self.tree
    }

    /// Evaluate a string with matrix variables.
    pub(crate) fn eval<'s>(&self, s: &'s str) -> Result<Cow<'s, RStr>> {
        use std::fmt::Write;

        let Some(i) = s.find("${{") else {
            return Ok(Cow::Borrowed(RStr::new(s)));
        };

        let mut result = RString::new();
        let (head, mut s) = s.split_at(i);
        result.push_rstr(head);

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
                Expr::String(s) => result.push_rstr(s.as_raw()),
                Expr::Float(f) => {
                    write!(result, "{f}").context("Failed to format float")?;
                }
                Expr::Bool(b) => {
                    result.push_rstr(if b { "true" } else { "false" });
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
    fn lookup<I>(&self, key: I) -> Vec<&'a RStr>
    where
        I: IntoIterator<Item: AsRef<str>, IntoIter: Clone>,
    {
        self.tree.get(key)
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

impl fmt::Debug for Matrix {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.matrix.fmt(f)
    }
}

pub(crate) struct Display<'a> {
    matrix: &'a BTreeMap<String, RString>,
}

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut it = self.matrix.iter();

        let shell = Shell::Bash;

        write!(f, "{{")?;

        if let Some((key, value)) = it.next() {
            let value = value.to_string_lossy();
            write!(f, "{key}={}", shell.escape(value.as_ref()))?;

            for (key, value) in it {
                let value = value.to_string_lossy();
                write!(f, ", {key}={}", shell.escape(value.as_ref()))?;
            }
        }

        write!(f, "}}")?;
        Ok(())
    }
}

impl fmt::Debug for Display<'_> {
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
