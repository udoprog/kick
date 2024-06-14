use super::Syntax;

use Syntax::{
    And, CloseParen, DoubleString, Eq, Error, Neq, OpenParen, Or, SingleString, Variable,
    Whitespace,
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

    /// Peek the next character of input.
    fn peek(&self) -> char {
        let s = self.source.get(self.cursor..).unwrap_or_default();
        s.chars().next().unwrap_or(NUL)
    }

    /// Peek the next character of input.
    fn peek2(&self) -> (char, char) {
        let s = self.source.get(self.cursor..).unwrap_or_default();
        let mut it = s.chars();
        let a = it.next().unwrap_or(NUL);
        let b = it.next().unwrap_or(NUL);
        (a, b)
    }

    /// Step over the next character.
    fn step(&mut self) {
        let c = self.peek();

        if self.peek() != NUL {
            self.cursor += c.len_utf8();
        }
    }

    /// Step over the two next characters.
    fn step2(&mut self) {
        self.step();
        self.step();
    }

    fn string(&mut self, delim: char) -> bool {
        self.step();

        loop {
            match self.peek() {
                NUL => return false,
                '\\' => {
                    self.step();
                    self.step();
                }
                c if c == delim => {
                    self.step();
                    break;
                }
                _ => self.step(),
            }
        }

        true
    }

    /// Consume input until we hit non-numerics.
    fn consume_while(&mut self, cond: fn(char) -> bool) {
        loop {
            let c = self.peek();

            if c == NUL || !cond(c) {
                break;
            }

            self.cursor += c.len_utf8();
        }
    }

    /// Get the next token.
    pub(crate) fn next(&mut self) -> Token {
        let (a, b) = self.peek2();
        let start = self.cursor;

        let syntax = match (a, b) {
            (NUL, _) => {
                return Token {
                    len: 0,
                    syntax: Syntax::Eof,
                }
            }
            (c, _) if c.is_whitespace() => {
                self.consume_while(char::is_whitespace);
                Whitespace
            }
            ('&', '&') => {
                self.step2();
                And
            }
            ('|', '|') => {
                self.step2();
                Or
            }
            ('=', '=') => {
                self.step2();
                Eq
            }
            ('!', '=') => {
                self.step2();
                Neq
            }
            ('(', _) => {
                self.step();
                OpenParen
            }
            (')', _) => {
                self.step();
                CloseParen
            }
            ('a'..='z', _) => {
                self.consume_while(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '.'));
                Variable
            }
            ('\'', _) => {
                if self.string('\'') {
                    SingleString
                } else {
                    Error
                }
            }
            ('\"', _) => {
                if self.string('"') {
                    DoubleString
                } else {
                    Error
                }
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
