use anyhow::Result;

use super::parsing::Parser;
use super::Syntax;

use self::Syntax::*;

fn op(syntax: Syntax) -> Option<(u8, u8)> {
    let prio = match syntax {
        And | Or => (1, 2),
        Eq | Neq => (3, 4),
        _ => return None,
    };

    Some(prio)
}

fn expr(p: &mut Parser<'_>, min: u8) -> Result<(), syntree::Error> {
    // Eat all available whitespace before getting a checkpoint.
    let tok = p.peek()?;

    let c = p.tree.checkpoint()?;

    match tok.syntax {
        OpenParen => {
            p.token()?;

            let c = p.tree.checkpoint()?;
            expr(p, 0)?;
            p.tree.close_at(&c, Group)?;

            if !p.eat(CloseParen)? {
                p.bump(Error)?;
                return Ok(());
            }
        }
        Variable => {
            p.bump(Variable)?;
        }
        SingleString => {
            p.bump(SingleString)?;
        }
        DoubleString => {
            p.bump(DoubleString)?;
        }
        _ => {
            p.bump(Error)?;
            return Ok(());
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
        expr(p, min)?;
        operation = true;
    }

    if operation {
        p.tree.close_at(&c, Operation)?;
    }

    Ok(())
}

/// Parse the root.
pub(crate) fn root(p: &mut Parser<'_>) -> Result<()> {
    loop {
        expr(p, 0)?;

        if p.is_eof()? {
            break;
        }

        // Simple error recovery where we consume until we find an operator
        // which will be consumed as an expression next.
        let c = p.tree.checkpoint()?;
        p.advance_until(&[Variable])?;
        p.tree.close_at(&c, Error)?;
    }

    Ok(())
}
