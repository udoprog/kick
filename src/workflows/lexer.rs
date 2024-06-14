use core::array;

use super::Syntax;

use Syntax::{
    And, CloseExpr, CloseParen, DoubleString, Eq, Error, Neq, Not, OpenExpr, OpenParen, Or,
    SingleString, Variable, Whitespace,
};

const NUL: char = '\0';

#[derive(Debug, Clone, Copy)]
pub(crate) struct Token {
    pub(crate) len: usize,
    pub(crate) syntax: Syntax,
}

pub(crate) struct Lexer<'a> {
    source: &'a str,
    cursor: usize,
}

impl<'a> Lexer<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        Self { source, cursor: 0 }
    }

    /// Peek `N` characters of input.
    fn peek<const N: usize>(&self) -> [char; N] {
        let s = self.source.get(self.cursor..).unwrap_or_default();
        let mut it = s.chars();
        array::from_fn(move |_| it.next().unwrap_or(NUL))
    }

    /// Step over the next character.
    fn step(&mut self, n: usize) {
        let Some(string) = self.source.get(self.cursor..) else {
            return;
        };

        for c in string.chars().take(n) {
            self.cursor += c.len_utf8();
        }
    }

    fn string(&mut self, delim: char) -> bool {
        self.step(1);

        loop {
            let [c] = self.peek();

            match c {
                NUL => return false,
                '\\' => {
                    self.step(2);
                }
                c if c == delim => {
                    self.step(1);
                    break;
                }
                _ => self.step(1),
            }
        }

        true
    }

    /// Consume input until we hit non-numerics.
    fn consume_while(&mut self, cond: fn(char) -> bool) {
        loop {
            let [c] = self.peek();

            if c == NUL || !cond(c) {
                break;
            }

            self.cursor += c.len_utf8();
        }
    }

    /// Get the next token.
    pub(crate) fn next(&mut self) -> Token {
        let [a, b, c] = self.peek();
        let start = self.cursor;

        let syntax = match (a, b, c) {
            (NUL, _, _) => {
                return Token {
                    len: 0,
                    syntax: Syntax::Eof,
                }
            }
            (c, _, _) if c.is_whitespace() => {
                self.consume_while(char::is_whitespace);
                Whitespace
            }
            ('&', '&', _) => {
                self.step(2);
                And
            }
            ('|', '|', _) => {
                self.step(2);
                Or
            }
            ('=', '=', _) => {
                self.step(2);
                Eq
            }
            ('!', '=', _) => {
                self.step(2);
                Neq
            }
            ('!', _, _) => {
                self.step(1);
                Not
            }
            ('(', _, _) => {
                self.step(1);
                OpenParen
            }
            (')', _, _) => {
                self.step(1);
                CloseParen
            }
            ('a'..='z', _, _) => {
                self.consume_while(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '.'));
                Variable
            }
            ('\'', _, _) => {
                if self.string('\'') {
                    SingleString
                } else {
                    Error
                }
            }
            ('\"', _, _) => {
                if self.string('"') {
                    DoubleString
                } else {
                    Error
                }
            }
            ('$', '{', '{') => {
                self.step(3);
                OpenExpr
            }
            ('}', '}', _) => {
                self.step(2);
                CloseExpr
            }
            _ => {
                self.consume_while(|c| !c.is_whitespace());
                Error
            }
        };

        let len = self.cursor.saturating_sub(start);
        Token { len, syntax }
    }
}
