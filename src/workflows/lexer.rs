use core::array;

use super::Syntax;

use Syntax::*;

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

    /// Peek a single character.
    fn peek1(&self) -> char {
        let [c] = self.peek::<1>();
        c
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

    fn number(&mut self) {
        if self.peek1() == '-' {
            self.step(1);
        }

        let mut has_dot = false;
        let mut has_e = false;

        loop {
            match self.peek1() {
                '0'..='9' => self.step(1),
                '.' if !has_dot => {
                    self.step(1);
                    has_dot = true;
                }
                'e' | 'E' if !has_e => {
                    self.step(1);
                    has_dot = true;
                    has_e = true;

                    if self.peek1() == '-' {
                        self.step(1);
                    }
                }
                _ => break,
            }
        }
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
            ('*', _, _) => {
                self.step(1);
                Star
            }
            (',', _, _) => {
                self.step(1);
                Comma
            }
            ('.', _, _) => {
                self.step(1);
                Dot
            }
            ('<', '=', _) => {
                self.step(2);
                LessEqual
            }
            ('<', _, _) => {
                self.step(1);
                Less
            }
            ('>', '=', _) => {
                self.step(2);
                GreaterEqual
            }
            ('>', _, _) => {
                self.step(1);
                Greater
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
            ('-' | '0'..='9', _, _) => {
                self.number();
                Number
            }
            ('a'..='z' | 'A'..='Z', _, _) => {
                self.consume_while(|c| matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-'));

                match &self.source[start..self.cursor] {
                    "true" | "false" => Bool,
                    "nan" | "NaN" => Number,
                    "null" => Null,
                    _ => Ident,
                }
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
