use super::{Matrix, Syntax};

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

#[derive(Debug, Error)]
pub(crate) enum EvalErrorKind {
    #[error("expected {0:?} but was {1:?}")]
    Expected(Syntax, Syntax),

    #[error("expected {0:?}")]
    Missing(Syntax),

    #[error("bad variable")]
    BadVariable,

    #[error("bad string")]
    BadString,

    #[error("{0:?} is not a valid operator")]
    UnexpectedOperator(Syntax),

    #[error("expected an operator")]
    ExpectedOperator,

    #[error("numerical overflow")]
    Overflow,

    #[error("numerical underflow")]
    Underflow,

    #[error("divide by zero")]
    DivideByZero,
}

use EvalErrorKind::{
    BadString, BadVariable, DivideByZero, Expected, ExpectedOperator, Missing, Overflow, Underflow,
    UnexpectedOperator,
};

/// The outcome of evaluating an expression.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Expr<'m> {
    Value(&'m str),
    Bool(bool),
}

fn eval_node<'a, I, W>(
    mut node: Node<'_, Syntax, I, W>,
    source: &'a str,
    matrix: &'a Matrix,
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

                let Some(value) = matrix.get(variable) else {
                    return Err(EvalError::new(*node.span(), BadVariable));
                };

                Ok(Expr::Value(value))
            }
            SingleString | DoubleString => {
                let value = &source[node.range()];

                let Some(value) = value.get(1..value.len().saturating_sub(1)) else {
                    return Err(EvalError::new(*node.span(), BadString));
                };

                Ok(Expr::Value(value))
            }
            Operation => {
                let mut it = node.children().skip_tokens();

                let first = it
                    .next()
                    .ok_or(EvalError::new(*node.span(), Missing(Variable)))?;

                let mut base = eval_node(first, source, matrix)?;

                while let Some(op) = it.next() {
                    let op = op
                        .first()
                        .ok_or(EvalError::new(*node.span(), ExpectedOperator))?;

                    let (calculate, error): (fn(Expr<'_>, Expr<'_>) -> Option<Expr<'static>>, _) =
                        match *op.value() {
                            Eq => (op_eq, Overflow),
                            Neq => (op_neq, Underflow),
                            And => (op_and, Overflow),
                            Or => (op_or, DivideByZero),
                            what => {
                                return Err(EvalError::new(*node.span(), UnexpectedOperator(what)))
                            }
                        };

                    let first = it
                        .next()
                        .ok_or(EvalError::new(*node.span(), Missing(Variable)))?;

                    let b = eval_node(first, source, matrix)?;

                    base = match calculate(base, b) {
                        Some(n) => n,
                        None => return Err(EvalError::new(op.span().join(node.span()), error)),
                    }
                }

                Ok(base)
            }
            what => Err(EvalError::new(*node.span(), Expected(Variable, what))),
        };
    }
}

fn op_eq(a: Expr<'_>, b: Expr<'_>) -> Option<Expr<'static>> {
    Some(Expr::Bool(a == b))
}

fn op_neq(a: Expr<'_>, b: Expr<'_>) -> Option<Expr<'static>> {
    Some(Expr::Bool(a != b))
}

fn op_and(a: Expr<'_>, b: Expr<'_>) -> Option<Expr<'static>> {
    match (a, b) {
        (Expr::Bool(a), Expr::Bool(b)) => Some(Expr::Bool(a & b)),
        _ => None,
    }
}

fn op_or(a: Expr<'_>, b: Expr<'_>) -> Option<Expr<'static>> {
    match (a, b) {
        (Expr::Bool(a), Expr::Bool(b)) => Some(Expr::Bool(a | b)),
        _ => None,
    }
}

/// Eval a tree emitting all available expressions parsed from it.
pub(crate) fn eval<'a, I, W>(
    tree: &'a Tree<Syntax, I, W>,
    source: &'a str,
    matrix: &'a Matrix,
) -> impl Iterator<Item = Result<Expr<'a>, EvalError<I>>> + 'a
where
    I: index::Index,
    W: pointer::Width,
{
    let mut it = tree.children().skip_tokens();

    std::iter::from_fn(move || {
        let node = it.next()?;
        Some(eval_node(node, source, matrix))
    })
}
