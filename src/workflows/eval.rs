use std::borrow::Cow;
use std::fmt;

use super::{Eval, Syntax};

use syntree::{index, pointer, Node, Span, Tree};
use thiserror::Error;

use Syntax::{And, DoubleString, Eq, Group, Neq, Operation, Or, SingleString, Variable};

#[derive(Debug, Error)]
#[error("{kind}")]
#[non_exhaustive]
pub(crate) struct EvalError<I> {
    pub(crate) span: Span<I>,
    pub(crate) kind: EvalErrorKind,
}

impl<I> EvalError<I> {
    const fn new(span: Span<I>, kind: EvalErrorKind) -> Self {
        Self { span, kind }
    }
}

#[derive(Debug)]
pub(crate) enum Operator {
    And,
    Or,
}

impl fmt::Display for Operator {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::And => write!(f, "&&"),
            Self::Or => write!(f, "||"),
        }
    }
}

#[derive(Debug, Error)]
pub(crate) enum EvalErrorKind {
    #[error("Expected {0:?} but was {1:?}")]
    Expected(Syntax, Syntax),

    #[error("Expected {0:?}")]
    Missing(Syntax),

    #[error("Bad variable `{0}`")]
    BadVariable(Box<str>),

    #[error("Bad string literal `{0}`")]
    BadString(Box<str>),

    #[error("Token `{0:?}` is not a valid operator")]
    UnexpectedOperator(Syntax),

    #[error("Expected an operator")]
    ExpectedOperator,

    #[error("Expected <bool> {op} <bool> but got {lhs} {op} {rhs}")]
    ExpectedBoolean {
        lhs: ExprKind,
        rhs: ExprKind,
        op: Operator,
    },
}

use EvalErrorKind::{
    BadString, BadVariable, Expected, ExpectedBoolean, ExpectedOperator, Missing,
    UnexpectedOperator,
};

/// The outcome of evaluating an expression.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Expr<'m> {
    String(Cow<'m, str>),
    Bool(bool),
}

impl Expr<'_> {
    fn kind(&self) -> ExprKind {
        match self {
            Self::String(_) => ExprKind::String,
            Self::Bool(_) => ExprKind::Bool,
        }
    }
}

#[derive(Debug)]
pub(crate) enum ExprKind {
    String,
    Bool,
}

impl fmt::Display for ExprKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::String => write!(f, "<string>"),
            Self::Bool => write!(f, "<bool>"),
        }
    }
}

fn eval_node<'a, I, W>(
    mut node: Node<'_, Syntax, I, W>,
    source: &'a str,
    eval: &Eval<'a>,
) -> Result<Expr<'a>, EvalError<I>>
where
    I: index::Index,
    W: pointer::Width,
{
    loop {
        return match *node.value() {
            Group => {
                node = node
                    .first()
                    .ok_or(EvalError::new(*node.span(), Missing(Variable)))?;
                continue;
            }
            Variable => {
                let variable = &source[node.range()];

                let Some(value) = eval.get(variable) else {
                    return Err(EvalError::new(*node.span(), BadVariable(variable.into())));
                };

                Ok(Expr::String(Cow::Borrowed(value)))
            }
            SingleString | DoubleString => {
                let value = &source[node.range()];

                let Some(value) = value.get(1..value.len().saturating_sub(1)) else {
                    return Err(EvalError::new(*node.span(), BadString(value.into())));
                };

                let Some(value) = unescape(value) else {
                    return Err(EvalError::new(*node.span(), BadString(value.into())));
                };

                Ok(Expr::String(value))
            }
            Operation => {
                let mut it = node.children().skip_tokens();

                let first = it
                    .next()
                    .ok_or(EvalError::new(*node.span(), Missing(Variable)))?;

                let mut base = eval_node(first, source, eval)?;

                while let Some(op) = it.next() {
                    let op = op
                        .first()
                        .ok_or(EvalError::new(*node.span(), ExpectedOperator))?;

                    let calculate: fn(
                        &Span<I>,
                        Expr<'_>,
                        Expr<'_>,
                    ) -> Result<Expr<'static>, EvalError<I>> = match *op.value() {
                        Eq => op_eq::<I>,
                        Neq => op_neq::<I>,
                        And => op_and::<I>,
                        Or => op_or::<I>,
                        what => return Err(EvalError::new(*node.span(), UnexpectedOperator(what))),
                    };

                    let first = it
                        .next()
                        .ok_or(EvalError::new(*node.span(), Missing(Variable)))?;

                    let b = eval_node(first, source, eval)?;

                    base = calculate(node.span(), base, b)?;
                }

                Ok(base)
            }
            what => Err(EvalError::new(*node.span(), Expected(Variable, what))),
        };
    }
}

fn op_eq<I>(_: &Span<I>, a: Expr<'_>, b: Expr<'_>) -> Result<Expr<'static>, EvalError<I>>
where
    I: index::Index,
{
    Ok(Expr::Bool(a == b))
}

fn op_neq<I>(_: &Span<I>, a: Expr<'_>, b: Expr<'_>) -> Result<Expr<'static>, EvalError<I>>
where
    I: index::Index,
{
    Ok(Expr::Bool(a != b))
}

fn op_and<I>(span: &Span<I>, a: Expr<'_>, b: Expr<'_>) -> Result<Expr<'static>, EvalError<I>>
where
    I: index::Index,
{
    match (a, b) {
        (Expr::Bool(a), Expr::Bool(b)) => Ok(Expr::Bool(a & b)),
        (lhs, rhs) => Err(EvalError::new(
            *span,
            ExpectedBoolean {
                lhs: lhs.kind(),
                rhs: rhs.kind(),
                op: Operator::And,
            },
        )),
    }
}

fn op_or<I>(span: &Span<I>, a: Expr<'_>, b: Expr<'_>) -> Result<Expr<'static>, EvalError<I>>
where
    I: index::Index,
{
    match (a, b) {
        (Expr::Bool(a), Expr::Bool(b)) => Ok(Expr::Bool(a | b)),
        (lhs, rhs) => Err(EvalError::new(
            *span,
            ExpectedBoolean {
                lhs: lhs.kind(),
                rhs: rhs.kind(),
                op: Operator::Or,
            },
        )),
    }
}

fn unescape(string: &str) -> Option<Cow<'_, str>> {
    let escaped = 'escaped: {
        for (i, c) in string.char_indices() {
            if c == '\\' {
                break 'escaped i;
            }
        }

        return Some(Cow::Borrowed(string));
    };

    let mut unescaped = String::with_capacity(string.len());
    unescaped.push_str(&string[..escaped]);

    let mut it = string[escaped..].chars();

    while let Some(c) = it.next() {
        if c == '\\' {
            let b = it.next()?;

            match b {
                'n' => unescaped.push('\n'),
                'r' => unescaped.push('\r'),
                't' => unescaped.push('\t'),
                '\\' => unescaped.push('\\'),
                '"' => unescaped.push('"'),
                '\'' => unescaped.push('\''),
                _ => unescaped.push(b),
            }
        } else {
            unescaped.push(c);
        }
    }

    Some(Cow::Owned(unescaped))
}

/// Eval a tree emitting all available expressions parsed from it.
pub(crate) fn eval<'b, 'a, I, W>(
    tree: &'b Tree<Syntax, I, W>,
    source: &'a str,
    eval: &'b Eval<'a>,
) -> impl Iterator<Item = Result<Expr<'a>, EvalError<I>>> + 'b
where
    I: index::Index,
    W: pointer::Width,
{
    let mut it = tree.children().skip_tokens();

    std::iter::from_fn(move || {
        let node = it.next()?;
        Some(eval_node(node, source, eval))
    })
}
