use std::borrow::Cow;

use anyhow::Result;
use syntree::Span;

use crate::rstr::RString;

use super::{CustomFunction, EvalError, Expr};

/// Default lookup function.
pub(crate) fn lookup_function(name: &str) -> Option<CustomFunction> {
    match name {
        "fromJSON" => Some(from_json),
        "startsWith" => Some(starts_with),
        "contains" => Some(contains),
        "cancelled" => Some(cancelled),
        "failure" => Some(failure),
        "success" => Some(success),
        "hashFiles" => Some(not_implemented),
        _ => None,
    }
}

fn from_json<'m>(span: &Span<u32>, args: &[Expr<'m>]) -> Result<Expr<'m>, EvalError> {
    let [Expr::String(string)] = args else {
        return Err(EvalError::custom(*span, "Expected one string argument"));
    };

    // NB: Figure out if we want to carry redaction into the decoded expression.
    let string = string.to_exposed();

    let is_secret = matches!(string, Cow::Owned(..));

    match serde_json::from_str(string.as_ref()) {
        Ok(value) => value_to_expr(span, value, is_secret),
        Err(error) => Err(EvalError::custom(*span, format_args!("{error}"))),
    }
}

fn starts_with<'m>(span: &Span<u32>, args: &[Expr<'m>]) -> Result<Expr<'m>, EvalError> {
    let [Expr::String(what), Expr::String(expect)] = args else {
        return Err(EvalError::custom(*span, "Expected two arguments"));
    };

    let what = what.to_exposed();
    let expect = expect.to_exposed();
    Ok(Expr::Bool(what.starts_with(expect.as_ref())))
}

fn contains<'m>(span: &Span<u32>, args: &[Expr<'m>]) -> Result<Expr<'m>, EvalError> {
    let [lhs, Expr::String(needle)] = args else {
        return Err(EvalError::custom(
            *span,
            "Expected two strings as arguments",
        ));
    };

    let needle = needle.to_exposed();

    match lhs {
        Expr::String(string) => {
            let string = string.to_exposed();
            Ok(Expr::Bool(string.contains(needle.as_ref())))
        }
        Expr::Array(array) => {
            let found = array
                .iter()
                .flat_map(|v| v.as_str())
                .any(|s| s.to_exposed().as_ref() == needle.as_ref());

            Ok(Expr::Bool(found))
        }
        lhs => {
            return Err(EvalError::custom(
                *span,
                format_args!("Expected string or array, got {lhs:?}"),
            ));
        }
    }
}

fn cancelled<'m>(span: &Span<u32>, args: &[Expr<'m>]) -> Result<Expr<'m>, EvalError> {
    let [] = args else {
        return Err(EvalError::custom(*span, "Expected no arguments"));
    };

    Ok(Expr::Bool(false))
}

fn failure<'m>(span: &Span<u32>, args: &[Expr<'m>]) -> Result<Expr<'m>, EvalError> {
    let [] = args else {
        return Err(EvalError::custom(*span, "Expected no arguments"));
    };

    Ok(Expr::Bool(false))
}

fn success<'m>(span: &Span<u32>, args: &[Expr<'m>]) -> Result<Expr<'m>, EvalError> {
    let [] = args else {
        return Err(EvalError::custom(*span, "Expected no arguments"));
    };

    Ok(Expr::Bool(true))
}

fn not_implemented<'m>(span: &Span<u32>, _: &[Expr<'m>]) -> Result<Expr<'m>, EvalError> {
    Err(EvalError::custom(
        *span,
        "Function has not been implemented yet",
    ))
}

fn value_to_expr(
    span: &Span<u32>,
    value: serde_json::Value,
    is_secret: bool,
) -> Result<Expr<'static>, EvalError> {
    let expr = match value {
        serde_json::Value::Null => Expr::Null,
        serde_json::Value::Bool(b) => Expr::Bool(b),
        serde_json::Value::Number(n) => {
            let Some(v) = n.as_f64() else {
                return Err(EvalError::custom(*span, "Failed to convert JSON number"));
            };

            Expr::Float(v)
        }
        serde_json::Value::String(string) => {
            let string = if is_secret {
                let Some(string) = RString::redacted(string) else {
                    return Err(EvalError::custom(
                        *span,
                        "Cannot make the given string a secret",
                    ));
                };

                string
            } else {
                RString::from(string)
            };

            Expr::String(Cow::Owned(string))
        }
        serde_json::Value::Array(array) => {
            let mut values = Vec::with_capacity(array.len());

            for value in array {
                values.push(value_to_expr(span, value, is_secret)?);
            }

            Expr::Array(values.into())
        }
        serde_json::Value::Object(_) => {
            return Err(EvalError::custom(*span, "Objects are not supported"));
        }
    };

    Ok(expr)
}
