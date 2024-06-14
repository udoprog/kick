use std::borrow::Cow;

use super::{Eval, Syntax};

use syntree::{index, pointer, Node, Span, Tree};
use thiserror::Error;

use Syntax::{And, Binary, DoubleString, Eq, Group, Neq, Not, Or, SingleString, Unary, Variable};

type UnaryFn<I> = for<'eval> fn(&Span<I>, Expr<'eval>) -> Result<Expr<'eval>, EvalError<I>>;

type BinaryFn<I> =
    for<'eval> fn(&Span<I>, Expr<'eval>, Expr<'eval>) -> Result<Expr<'eval>, EvalError<I>>;

#[derive(Debug, Error, PartialEq, Eq)]
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

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum EvalErrorKind {
    #[error("Expected {0:?} but was {1:?}")]
    Expected(Syntax, Syntax),

    #[error("Expected {0:?}")]
    Missing(Syntax),

    #[error("Bad string literal `{0}`")]
    BadString(Box<str>),

    #[error("Token `{0:?}` is not a valid operator")]
    UnexpectedOperator(Syntax),

    #[error("Expected an operator")]
    ExpectedOperator,
}

use EvalErrorKind::{BadString, Expected, ExpectedOperator, Missing, UnexpectedOperator};

/// The outcome of evaluating an expression.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Expr<'m> {
    /// A string expression.
    String(Cow<'m, str>),
    /// A boolean expression.
    Bool(bool),
    /// Expression evaluates to nothing.
    Null,
}

impl Expr<'_> {
    /// Coerce an expression into a boolean value.
    pub(crate) fn as_bool(&self) -> bool {
        match self {
            Self::String(string) => !string.is_empty(),
            Self::Bool(b) => *b,
            Self::Null => false,
        }
    }
}

fn eval_node<'node, 'a, I, W>(
    mut node: Node<'node, Syntax, I, W>,
    source: &'a str,
    eval: &Eval<'a>,
) -> Result<Expr<'a>, EvalError<I>>
where
    I: index::Index,
    W: 'node + pointer::Width,
{
    loop {
        return match *node.value() {
            Group => {
                node = node
                    .children()
                    .skip_tokens()
                    .next()
                    .ok_or(EvalError::new(*node.span(), Missing(Variable)))?;
                continue;
            }
            Variable => {
                let variable = &source[node.range()];

                let Some(value) = eval.get(variable) else {
                    return Ok(Expr::Null);
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
            Unary => {
                let mut it = node.children().skip_tokens();

                let op = it
                    .next()
                    .ok_or(EvalError::new(*node.span(), ExpectedOperator))?;

                let calculate: UnaryFn<I> = match *op.value() {
                    Not => op_not::<I>,
                    what => return Err(EvalError::new(*node.span(), UnexpectedOperator(what))),
                };

                let rhs = it
                    .next()
                    .ok_or(EvalError::new(*node.span(), Missing(Variable)))?;

                let rhs = eval_node(rhs, source, eval)?;
                Ok(calculate(node.span(), rhs)?)
            }
            Binary => {
                let mut it = node.children().skip_tokens();

                let first = it
                    .next()
                    .ok_or(EvalError::new(*node.span(), Missing(Variable)))?;

                let mut lhs = eval_node(first, source, eval)?;

                while let Some(op) = it.next() {
                    let op = op
                        .first()
                        .ok_or(EvalError::new(*node.span(), ExpectedOperator))?;

                    let calculate: BinaryFn<I> = match *op.value() {
                        Eq => op_eq::<I>,
                        Neq => op_neq::<I>,
                        And => op_and::<I>,
                        Or => op_or::<I>,
                        what => return Err(EvalError::new(*node.span(), UnexpectedOperator(what))),
                    };

                    let rhs = it
                        .next()
                        .ok_or(EvalError::new(*node.span(), Missing(Variable)))?;

                    let rhs = eval_node(rhs, source, eval)?;
                    lhs = calculate(node.span(), lhs, rhs)?;
                }

                Ok(lhs)
            }
            what => Err(EvalError::new(*node.span(), Expected(Variable, what))),
        };
    }
}

fn op_not<'a, I>(_: &Span<I>, expr: Expr<'a>) -> Result<Expr<'a>, EvalError<I>>
where
    I: index::Index,
{
    Ok(Expr::Bool(!expr.as_bool()))
}

fn op_eq<'a, I>(_: &Span<I>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError<I>>
where
    I: index::Index,
{
    Ok(Expr::Bool(lhs == rhs))
}

fn op_neq<'a, I>(_: &Span<I>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError<I>>
where
    I: index::Index,
{
    Ok(Expr::Bool(lhs != rhs))
}

fn op_and<'a, I>(_: &Span<I>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError<I>>
where
    I: index::Index,
{
    match (lhs, rhs) {
        (Expr::Bool(a), Expr::Bool(b)) => Ok(Expr::Bool(a && b)),
        (lhs, rhs) => {
            if lhs.as_bool() && rhs.as_bool() {
                return Ok(rhs);
            }

            Ok(Expr::Null)
        }
    }
}

fn op_or<'a, I>(_: &Span<I>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError<I>>
where
    I: index::Index,
{
    match (lhs, rhs) {
        (Expr::Bool(lhs), Expr::Bool(rhs)) => Ok(Expr::Bool(lhs || rhs)),
        (lhs, rhs) => {
            if lhs.as_bool() {
                Ok(lhs)
            } else {
                Ok(rhs)
            }
        }
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
    I: index::Index + std::fmt::Display,
    W: pointer::Width,
{
    tree.children()
        .skip_tokens()
        .map(move |node| eval_node(node, source, eval))
}
