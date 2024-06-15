use std::ops::BitAndAssign;

use anyhow::Result;

use super::parsing::Parser;
use super::{ExprError, Syntax};

use self::Syntax::*;

#[must_use = "the outcome of parsing an expression must be handled"]
enum Outcome {
    Ok,
    Error,
}

impl BitAndAssign<Outcome> for Outcome {
    #[inline]
    fn bitand_assign(&mut self, rhs: Outcome) {
        if let Outcome::Error = *self {
            return;
        }
        *self = rhs;
    }
}

fn op(syntax: Syntax) -> Option<(u8, u8)> {
    let prio = match syntax {
        And | Or => (1, 2),
        Eq | Neq => (3, 4),
        Less | LessEqual | Greater | GreaterEqual => (5, 6),
        _ => return None,
    };

    Some(prio)
}

fn is_expr_start(syntax: Syntax) -> bool {
    matches!(
        syntax,
        Ident | Number | Null | Bool | SingleString | DoubleString | OpenParen | OpenExpr
    )
}

fn expr(p: &mut Parser<'_>, min: u8) -> Result<Outcome, ExprError> {
    let mut ok = Outcome::Ok;

    // Eat all available whitespace before getting a checkpoint.
    let tok = p.peek()?;

    let c = p.tree.checkpoint()?;

    match tok.syntax {
        Not => {
            p.bump(Not)?;
            ok &= expr(p, 0)?;
            p.tree.close_at(&c, Unary)?;
        }
        OpenParen => {
            ok &= group(p, CloseParen)?;
        }
        OpenExpr => {
            ok &= group(p, CloseExpr)?;
        }
        Ident => {
            p.bump(Ident)?;

            if p.eat(OpenParen)? {
                ok &= function(p)?;
                p.tree.close_at(&c, Function)?;
            } else if lookup(p)? {
                p.tree.close_at(&c, Lookup)?;
            } else {
                p.tree.close_at(&c, Error)?;
            }
        }
        tok @ (Number | Null | Bool | SingleString | DoubleString) => {
            p.bump(tok)?;
        }
        _ => {
            p.token()?;
            return Ok(Outcome::Error);
        }
    }

    let mut operation = false;

    loop {
        let tok = p.peek()?;

        let min = match op(tok.syntax) {
            Some((left, right)) if left >= min => right,
            _ => break,
        };

        p.bump(Operator)?;
        ok &= expr(p, min)?;
        operation = true;
    }

    if operation {
        p.tree.close_at(&c, Binary)?;
    }

    Ok(ok)
}

fn lookup(p: &mut Parser) -> Result<bool, ExprError> {
    let mut ok = true;

    while p.eat(Dot)? {
        let what = p.peek()?.syntax;
        ok &= matches!(what, Ident | Star);
        // Bump whatever is there anyway in the hope that we can "keep going",
        // but treat the expression as an error.
        p.bump(what)?;
    }

    Ok(ok)
}

fn function(p: &mut Parser) -> Result<Outcome, ExprError> {
    let mut ok = Outcome::Ok;
    let mut end = false;

    loop {
        match p.peek()?.syntax {
            CloseParen => {
                p.token()?;
                break;
            }
            _ => {
                ok &= expr(p, 0)?;
            }
        }

        if end {
            if !p.eat(CloseParen)? {
                ok = Outcome::Error;
            }

            break;
        }

        if !p.eat(Comma)? {
            end = true;
        }
    }

    Ok(ok)
}

fn group(p: &mut Parser, until: Syntax) -> Result<Outcome, ExprError> {
    p.token()?;
    let c = p.tree.checkpoint()?;
    let mut ok = expr(p, 0)?;
    p.tree.close_at(&c, Group)?;

    if !p.eat(until)? {
        p.bump(Error)?;
        ok = Outcome::Error;
    }

    Ok(ok)
}

/// Parse the root.
pub(crate) fn root(p: &mut Parser<'_>) -> Result<(), ExprError> {
    while !p.is_eof()? {
        let c = p.tree.checkpoint()?;

        if matches!(expr(p, 0)?, Outcome::Ok) {
            continue;
        }

        // Simple error recovery where we consume until we find an operator
        // which will be consumed as an expression next.
        p.advance_until(is_expr_start)?;
        p.tree.close_at(&c, Error)?;
    }

    Ok(())
}
