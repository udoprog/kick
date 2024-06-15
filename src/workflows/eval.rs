use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::str::FromStr;

use crate::redact::{OwnedRedact, Redact};

use super::{Eval, Syntax};

use syntree::{pointer, Node, Span, Tree};
use thiserror::Error;

use Syntax::*;

type UnaryFn = for<'eval> fn(&Span<u32>, Expr<'eval>) -> Result<Expr<'eval>, EvalError>;

type BinaryFn =
    for<'eval> fn(&Span<u32>, Expr<'eval>, Expr<'eval>) -> Result<Expr<'eval>, EvalError>;

#[derive(Debug, Error, PartialEq, Eq)]
#[error("{kind}")]
#[non_exhaustive]
pub(crate) struct EvalError {
    pub(crate) span: Span<u32>,
    pub(crate) kind: EvalErrorKind,
}

impl EvalError {
    pub(crate) fn new<K>(span: Span<u32>, kind: K) -> Self
    where
        EvalErrorKind: From<K>,
    {
        Self {
            span,
            kind: EvalErrorKind::from(kind),
        }
    }

    pub(crate) fn custom<C>(span: Span<u32>, custom: C) -> Self
    where
        C: fmt::Display,
    {
        let custom = custom.to_string();

        Self {
            span,
            kind: EvalErrorKind::Custom(custom.into()),
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub(crate) enum EvalErrorKind {
    #[error("Expected {0:?}")]
    Missing(Syntax),

    #[error("Bad number")]
    BadNumber,

    #[error("Bad string literal `{0}`")]
    BadString(Box<str>),

    #[error("Token `{0:?}` is not a valid operator")]
    UnexpectedOperator(Syntax),

    #[error("Expected boolean")]
    BadBoolean(Box<str>),

    #[error("Expected an operator")]
    ExpectedOperator,

    #[error("Expected an expression")]
    ExpectedExpression,

    #[error("Missing function `{0}`")]
    MissingFunction(Box<str>),

    #[error("{0}")]
    Custom(Box<str>),
}

use EvalErrorKind::*;

/// The outcome of evaluating an expression.
#[derive(Debug, PartialEq)]
pub(crate) enum Expr<'m> {
    /// An array of values.
    Array(Box<[Expr<'m>]>),
    /// A string expression.
    String(Cow<'m, Redact>),
    /// A floating-point expression.
    Float(f64),
    /// A boolean expression.
    Bool(bool),
    /// Expression evaluates to nothing.
    Null,
}

#[cfg(test)]
impl<'m> From<&'m str> for Expr<'m> {
    #[inline]
    fn from(s: &'m str) -> Self {
        Self::String(Cow::Borrowed(Redact::new(s)))
    }
}

#[cfg(test)]
impl<'m> From<f64> for Expr<'m> {
    #[inline]
    fn from(value: f64) -> Self {
        Self::Float(value)
    }
}

impl Expr<'_> {
    /// Test if the number is equivalent to `NaN`.
    pub(crate) fn as_f64(&self) -> f64 {
        match *self {
            Self::Array(..) => f64::NAN,
            // NB: non-empty strings treated as numbers are not numbers.
            Self::String(ref string) => {
                if string.is_empty() {
                    0.0
                } else {
                    f64::NAN
                }
            }
            Self::Float(float) => float,
            Self::Bool(b) => {
                if b {
                    1.0
                } else {
                    0.0
                }
            }
            Self::Null => 0.0,
        }
    }

    /// Coerce an expression into a boolean value.
    pub(crate) fn as_bool(&self) -> bool {
        match *self {
            Self::Array(..) => false,
            Self::String(ref string) => !string.is_empty(),
            Self::Float(float) => {
                if float.is_nan() {
                    false
                } else {
                    float != 0.0
                }
            }
            Self::Bool(b) => b,
            Self::Null => false,
        }
    }

    /// Get the expression as a string.
    pub(crate) fn as_str(&self) -> Option<&Redact> {
        match *self {
            Self::String(ref string) => Some(string),
            _ => None,
        }
    }
}

fn eval_node<'node, 'm, W>(
    mut node: Node<'node, Syntax, u32, W>,
    source: &'m str,
    eval: &Eval<'m>,
) -> Result<Expr<'m>, EvalError>
where
    W: 'node + pointer::Width,
{
    loop {
        return match *node.value() {
            Function => {
                let mut it = node.children().skip_tokens();

                let ident = it
                    .next()
                    .ok_or(EvalError::new(*node.span(), Missing(Ident)))?;

                let mut args = Vec::new();

                for node in it {
                    args.push(eval_node(node, source, eval)?);
                }

                let ident = &source[ident.range()];

                let Some(function) = eval.function(ident) else {
                    return Err(EvalError::new(*node.span(), MissingFunction(ident.into())));
                };

                function(node.span(), &args)
            }
            Group => {
                node = node
                    .children()
                    .skip_tokens()
                    .next()
                    .ok_or(EvalError::new(*node.span(), Missing(Ident)))?;
                continue;
            }
            Null => Ok(Expr::Null),
            Bool => match &source[node.range()] {
                "true" => Ok(Expr::Bool(true)),
                "false" => Ok(Expr::Bool(false)),
                what => Err(EvalError::new(*node.span(), BadBoolean(what.into()))),
            },
            Number => {
                let Ok(number) = f64::from_str(&source[node.range()]) else {
                    return Err(EvalError::new(*node.span(), BadNumber));
                };

                Ok(Expr::Float(number))
            }
            Lookup => {
                let keys = node
                    .children()
                    .skip_tokens()
                    .map(|n| &source[n.span().range()]);

                let value = eval.lookup(keys);

                match &value[..] {
                    [] => Ok(Expr::Null),
                    [value] => Ok(Expr::String(Cow::Borrowed(*value))),
                    values => {
                        let values = values
                            .iter()
                            .map(|v| Expr::String(Cow::Borrowed(*v)))
                            .collect::<Vec<_>>()
                            .into();
                        Ok(Expr::Array(values))
                    }
                }
            }
            SingleString | DoubleString => {
                let value = &source[node.range()];

                let Some(value) = value.get(1..value.len().saturating_sub(1)) else {
                    return Err(EvalError::new(*node.span(), BadString(value.into())));
                };

                let Some(value) = unescape(value) else {
                    return Err(EvalError::new(*node.span(), BadString(value.into())));
                };

                let value = match value {
                    Cow::Borrowed(s) => Cow::Borrowed(Redact::new(s)),
                    Cow::Owned(s) => Cow::Owned(OwnedRedact::from(s)),
                };

                Ok(Expr::String(value))
            }
            Unary => {
                let mut it = node.children().skip_tokens();

                let op = it
                    .next()
                    .ok_or(EvalError::new(*node.span(), ExpectedOperator))?;

                let calculate: UnaryFn = match *op.value() {
                    Not => op_not,
                    what => return Err(EvalError::new(*node.span(), UnexpectedOperator(what))),
                };

                let rhs = it
                    .next()
                    .ok_or(EvalError::new(*node.span(), Missing(Lookup)))?;

                let rhs = eval_node(rhs, source, eval)?;
                Ok(calculate(node.span(), rhs)?)
            }
            Binary => {
                let mut it = node.children().skip_tokens();

                let first = it
                    .next()
                    .ok_or(EvalError::new(*node.span(), Missing(Lookup)))?;

                let mut lhs = eval_node(first, source, eval)?;

                while let Some(op) = it.next() {
                    let op = op
                        .first()
                        .ok_or(EvalError::new(*node.span(), ExpectedOperator))?;

                    let calculate: BinaryFn = match *op.value() {
                        Eq => op_eq,
                        Neq => op_neq,
                        And => op_and,
                        Or => op_or,
                        Less => op_lt,
                        LessEqual => op_lte,
                        Greater => op_gt,
                        GreaterEqual => op_gte,
                        what => return Err(EvalError::new(*node.span(), UnexpectedOperator(what))),
                    };

                    let rhs = it
                        .next()
                        .ok_or(EvalError::new(*node.span(), Missing(Lookup)))?;

                    let rhs = eval_node(rhs, source, eval)?;
                    lhs = calculate(node.span(), lhs, rhs)?;
                }

                Ok(lhs)
            }
            _ => Err(EvalError::new(*node.span(), ExpectedExpression)),
        };
    }
}

fn op_not<'a>(_: &Span<u32>, expr: Expr<'a>) -> Result<Expr<'a>, EvalError> {
    Ok(Expr::Bool(!expr.as_bool()))
}

fn op_cmp<'a>(_: &Span<u32>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Option<Ordering>, EvalError> {
    match (lhs, rhs) {
        (Expr::String(lhs), Expr::String(rhs)) => Ok(lhs.partial_cmp(&rhs)),
        (Expr::Float(lhs), Expr::Float(rhs)) => Ok(lhs.partial_cmp(&rhs)),
        (Expr::Bool(lhs), Expr::Bool(rhs)) => Ok(lhs.partial_cmp(&rhs)),
        (Expr::Null, Expr::Null) => Ok(Some(Ordering::Equal)),
        (lhs, rhs) => Ok(lhs.as_f64().partial_cmp(&rhs.as_f64())),
    }
}

fn op_eq<'a>(span: &Span<u32>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError> {
    match op_cmp(span, lhs, rhs)? {
        Some(Ordering::Equal) => Ok(Expr::Bool(true)),
        _ => Ok(Expr::Bool(false)),
    }
}

fn op_neq<'a>(span: &Span<u32>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError> {
    match op_cmp(span, lhs, rhs)? {
        Some(Ordering::Less | Ordering::Greater) => Ok(Expr::Bool(true)),
        _ => Ok(Expr::Bool(false)),
    }
}

fn op_lt<'a>(span: &Span<u32>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError> {
    match op_cmp(span, lhs, rhs)? {
        Some(Ordering::Less) => Ok(Expr::Bool(true)),
        _ => Ok(Expr::Bool(false)),
    }
}

fn op_lte<'a>(span: &Span<u32>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError> {
    match op_cmp(span, lhs, rhs)? {
        Some(Ordering::Less | Ordering::Equal) => Ok(Expr::Bool(true)),
        _ => Ok(Expr::Bool(false)),
    }
}

fn op_gt<'a>(span: &Span<u32>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError> {
    match op_cmp(span, lhs, rhs)? {
        Some(Ordering::Greater) => Ok(Expr::Bool(true)),
        _ => Ok(Expr::Bool(false)),
    }
}

fn op_gte<'a>(span: &Span<u32>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError> {
    match op_cmp(span, lhs, rhs)? {
        Some(Ordering::Greater | Ordering::Equal) => Ok(Expr::Bool(true)),
        _ => Ok(Expr::Bool(false)),
    }
}

fn op_and<'a>(_: &Span<u32>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError> {
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

fn op_or<'a>(_: &Span<u32>, lhs: Expr<'a>, rhs: Expr<'a>) -> Result<Expr<'a>, EvalError> {
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
pub(crate) fn eval<'b, 'a, W>(
    tree: &'b Tree<Syntax, u32, W>,
    source: &'a str,
    eval: &'b Eval<'a>,
) -> impl Iterator<Item = Result<Expr<'a>, EvalError>> + 'b
where
    W: pointer::Width,
{
    tree.children()
        .skip_tokens()
        .map(move |node| eval_node(node, source, eval))
}
