//! A small Rust source parser which finds the items declared in a module file.
//!
//! It only looks at item headers and into the containers needed to find items
//! and members: inline module bodies, impl blocks, enum variants and struct
//! fields. Everything else is skipped as a balanced token tree, and input it
//! does not understand is skipped up to the next item.

use syntree::{Builder, FlavorDefault, Node, Tree};

/// Nesting of inline modules and impl blocks beyond which bodies are skipped.
const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Syntax {
    Whitespace,
    Comment,
    Ident,
    Lifetime,
    Literal,
    Punct,
    OpenParen,
    CloseParen,
    OpenBracket,
    CloseBracket,
    OpenBrace,
    CloseBrace,
    Root,
    Attribute,
    Visibility,
    Name,
    /// A balanced token tree which is not looked into.
    Group,
    /// The items of an inline module or an impl block.
    Block,
    Variants,
    Variant,
    Fields,
    Field,
    ImplTrait,
    SelfType,
    UsePath,
    Rename,
    Fn,
    Struct,
    Enum,
    Union,
    Trait,
    TypeAlias,
    Const,
    Static,
    Mod,
    Impl,
    MacroRules,
    Use,
    ExternCrate,
    /// Anything else, such as macro invocations.
    Other,
}

use Syntax::*;

#[derive(Debug, Clone, Copy)]
struct Tok {
    syntax: Syntax,
    start: usize,
    end: usize,
}

impl Tok {
    fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

struct Lexer<'a> {
    source: &'a str,
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn rest(&self) -> &'a str {
        self.source.get(self.pos..).unwrap_or_default()
    }

    fn peek(&self) -> Option<char> {
        self.rest().chars().next()
    }

    fn peek2(&self) -> Option<char> {
        self.rest().chars().nth(1)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.pos = self.pos.saturating_add(c.len_utf8());
        Some(c)
    }

    fn eat_while(&mut self, f: impl std::ops::Fn(char) -> bool) {
        while self.peek().is_some_and(&f) {
            self.bump();
        }
    }

    fn string(&mut self) {
        while let Some(c) = self.bump() {
            match c {
                '\\' => {
                    self.bump();
                }
                '"' => break,
                _ => {}
            }
        }
    }

    /// A raw string, positioned after the `r`.
    fn raw_string(&mut self) {
        let mut hashes = 0usize;

        while self.peek() == Some('#') {
            self.bump();
            hashes = hashes.saturating_add(1);
        }

        self.bump();

        while let Some(c) = self.bump() {
            if c == '"' {
                let rest = self.rest();
                let closing = rest
                    .len()
                    .saturating_sub(rest.trim_start_matches('#').len());

                if closing >= hashes {
                    self.pos = self.pos.saturating_add(hashes);
                    break;
                }
            }
        }
    }

    /// A character literal, positioned after the opening quote.
    fn char(&mut self) {
        while let Some(c) = self.bump() {
            match c {
                '\\' => {
                    self.bump();
                }
                '\'' | '\n' => break,
                _ => {}
            }
        }
    }

    fn block_comment(&mut self) {
        let mut depth = 1usize;

        while let Some(c) = self.bump() {
            match c {
                '/' if self.peek() == Some('*') => {
                    self.bump();
                    depth = depth.saturating_add(1);
                }
                '*' if self.peek() == Some('/') => {
                    self.bump();
                    depth = depth.saturating_sub(1);

                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    fn starts_raw_string(s: &str) -> bool {
        s.trim_start_matches('#').starts_with('"')
    }

    /// A literal with a prefix, positioned after the prefix character `c`.
    fn prefixed(&mut self, c: char) -> bool {
        let rest = self.rest();

        if c == 'r' {
            if Self::starts_raw_string(rest) {
                self.raw_string();
                return true;
            }

            return false;
        }

        if rest.starts_with('"') {
            self.bump();
            self.string();
            return true;
        }

        if c == 'b' && rest.starts_with('\'') {
            self.bump();
            self.char();
            return true;
        }

        if rest.strip_prefix('r').is_some_and(Self::starts_raw_string) {
            self.bump();
            self.raw_string();
            return true;
        }

        false
    }

    /// A lifetime or a character literal, positioned after the quote.
    fn quote(&mut self) -> Syntax {
        match (self.peek(), self.peek2()) {
            (Some('\\'), _) => {
                self.char();
                Literal
            }
            (Some(_), Some('\'')) => {
                self.bump();
                self.bump();
                Literal
            }
            (Some(c), _) if is_ident_start(c) => {
                self.eat_while(is_ident_continue);
                Lifetime
            }
            _ => Punct,
        }
    }

    fn next(&mut self) -> Option<Tok> {
        let start = self.pos;
        let c = self.bump()?;

        let syntax = match c {
            c if c.is_whitespace() => {
                self.eat_while(char::is_whitespace);
                Whitespace
            }
            '/' if self.peek() == Some('/') => {
                self.eat_while(|c| c != '\n');
                Comment
            }
            '/' if self.peek() == Some('*') => {
                self.bump();
                self.block_comment();
                Comment
            }
            '"' => {
                self.string();
                Literal
            }
            '\'' => self.quote(),
            'r' if self.peek() == Some('#') && self.peek2().is_some_and(is_ident_start) => {
                self.bump();
                self.eat_while(is_ident_continue);
                Ident
            }
            'b' | 'c' | 'r' if self.prefixed(c) => Literal,
            c if is_ident_start(c) => {
                self.eat_while(is_ident_continue);
                Ident
            }
            c if c.is_ascii_digit() => {
                self.eat_while(is_ident_continue);

                if self.peek() == Some('.') && self.peek2().is_some_and(|c| c.is_ascii_digit()) {
                    self.bump();
                    self.eat_while(is_ident_continue);
                }

                Literal
            }
            '(' => OpenParen,
            ')' => CloseParen,
            '[' => OpenBracket,
            ']' => CloseBracket,
            '{' => OpenBrace,
            '}' => CloseBrace,
            ':' if self.peek() == Some(':') => {
                self.bump();
                Punct
            }
            '-' | '=' if self.peek() == Some('>') => {
                self.bump();
                Punct
            }
            _ => Punct,
        };

        Some(Tok {
            syntax,
            start,
            end: self.pos,
        })
    }
}

fn is_open(syntax: Syntax) -> bool {
    matches!(syntax, OpenParen | OpenBracket | OpenBrace)
}

fn is_close(syntax: Syntax) -> bool {
    matches!(syntax, CloseParen | CloseBracket | CloseBrace)
}

type Result<T, E = syntree::Error> = std::result::Result<T, E>;

struct Parser<'a> {
    source: &'a str,
    tokens: Vec<Tok>,
    pos: usize,
    tree: Builder<Syntax>,
}

impl<'a> Parser<'a> {
    /// The `n`th significant token from the current position.
    fn nth(&self, n: usize) -> Option<Tok> {
        self.tokens
            .get(self.pos..)?
            .iter()
            .filter(|t| !matches!(t.syntax, Whitespace | Comment))
            .nth(n)
            .copied()
    }

    fn peek(&self) -> Option<Syntax> {
        Some(self.nth(0)?.syntax)
    }

    fn text(&self, tok: Tok) -> &'a str {
        self.source.get(tok.start..tok.end).unwrap_or_default()
    }

    fn nth_is(&self, n: usize, s: &str) -> bool {
        self.nth(n)
            .is_some_and(|t| matches!(t.syntax, Ident | Punct) && self.text(t) == s)
    }

    fn is(&self, s: &str) -> bool {
        self.nth_is(0, s)
    }

    fn trivia(&mut self) -> Result<()> {
        while let Some(tok) = self.tokens.get(self.pos).copied() {
            if !matches!(tok.syntax, Whitespace | Comment) {
                break;
            }

            self.tree.trivia(tok.syntax, tok.len())?;
            self.pos = self.pos.saturating_add(1);
        }

        Ok(())
    }

    fn bump(&mut self) -> Result<()> {
        self.trivia()?;

        if let Some(tok) = self.tokens.get(self.pos).copied() {
            self.tree.token(tok.syntax, tok.len())?;
            self.pos = self.pos.saturating_add(1);
        }

        Ok(())
    }

    fn open(&mut self, syntax: Syntax) -> Result<()> {
        self.trivia()?;
        self.tree.open(syntax)?;
        Ok(())
    }

    /// Skip a balanced token tree, positioned at its opening delimiter.
    fn group(&mut self) -> Result<()> {
        self.open(Group)?;
        let mut depth = 0usize;

        while let Some(syntax) = self.peek() {
            self.bump()?;

            if is_open(syntax) {
                depth = depth.saturating_add(1);
            } else if is_close(syntax) {
                depth = depth.saturating_sub(1);
            }

            if depth == 0 {
                break;
            }
        }

        self.tree.close()?;
        Ok(())
    }

    /// Skip to the end of an item, which is a `;` or a `{ .. }` block.
    fn skip_rest(&mut self) -> Result<()> {
        while let Some(syntax) = self.peek() {
            match syntax {
                OpenBrace => return self.group(),
                OpenParen | OpenBracket => self.group()?,
                syntax if is_close(syntax) => break,
                _ if self.is(";") => return self.bump(),
                _ => self.bump()?,
            }
        }

        Ok(())
    }

    /// Skip to the end of an item which ends with a `;`.
    fn skip_to_semi(&mut self) -> Result<()> {
        while let Some(syntax) = self.peek() {
            match syntax {
                syntax if is_open(syntax) => self.group()?,
                syntax if is_close(syntax) => break,
                _ if self.is(";") => return self.bump(),
                _ => self.bump()?,
            }
        }

        Ok(())
    }

    fn name(&mut self) -> Result<()> {
        if self.peek() == Some(Ident) {
            self.open(Name)?;
            self.bump()?;
            self.tree.close()?;
        }

        Ok(())
    }

    /// An outer `#[..]` or inner `#![..]` attribute.
    fn attribute(&mut self, inner: bool) -> Result<bool> {
        let bracket = if inner {
            self.nth_is(1, "!") && self.nth(2).is_some_and(|t| t.syntax == OpenBracket)
        } else {
            self.nth(1).is_some_and(|t| t.syntax == OpenBracket)
        };

        if !self.is("#") || !bracket {
            return Ok(false);
        }

        self.open(Attribute)?;
        self.bump()?;

        if inner {
            self.bump()?;
        }

        self.group()?;
        self.tree.close()?;
        Ok(true)
    }

    fn attributes(&mut self) -> Result<()> {
        while self.attribute(false)? {}
        Ok(())
    }

    fn visibility(&mut self) -> Result<()> {
        if self.is("pub") {
            self.open(Visibility)?;
            self.bump()?;

            if self.peek() == Some(OpenParen) {
                self.group()?;
            }

            self.tree.close()?;
        }

        Ok(())
    }

    fn items(&mut self, depth: usize, block: bool) -> Result<()> {
        while let Some(syntax) = self.peek() {
            match syntax {
                CloseBrace if block => break,
                syntax if is_close(syntax) => self.bump()?,
                _ => self.item(depth)?,
            }
        }

        Ok(())
    }

    /// The items of an inline module or impl block, positioned at the `{`.
    fn block(&mut self, depth: usize) -> Result<()> {
        if depth >= MAX_DEPTH {
            return self.group();
        }

        self.open(Block)?;
        self.bump()?;
        self.items(depth.saturating_add(1), true)?;

        if self.peek() == Some(CloseBrace) {
            self.bump()?;
        }

        self.tree.close()?;
        Ok(())
    }

    fn kind(&mut self) -> Result<Syntax> {
        loop {
            let Some(tok) = self.nth(0) else {
                return Ok(Other);
            };

            if tok.syntax != Ident {
                return Ok(Other);
            }

            let kind = match self.text(tok) {
                "unsafe" | "async" | "default" | "auto" | "safe" => None,
                "extern" => {
                    self.bump()?;

                    if self.peek() == Some(Literal) {
                        self.bump()?;
                    }

                    if self.is("crate") {
                        return Ok(ExternCrate);
                    }

                    continue;
                }
                "const"
                    if ["fn", "unsafe", "async", "extern"]
                        .iter()
                        .any(|s| self.nth_is(1, s)) =>
                {
                    None
                }
                "const" => Some(Const),
                "static" => Some(Static),
                "fn" => Some(Fn),
                "struct" => Some(Struct),
                "enum" => Some(Enum),
                "trait" => Some(Trait),
                "type" => Some(TypeAlias),
                "mod" => Some(Mod),
                "impl" => Some(Impl),
                "use" => Some(Use),
                "union" if self.nth(1).is_some_and(|t| t.syntax == Ident) => Some(Union),
                "macro_rules" if self.nth_is(1, "!") => Some(MacroRules),
                _ => return Ok(Other),
            };

            match kind {
                Some(kind) => return Ok(kind),
                None => self.bump()?,
            }
        }
    }

    fn item(&mut self, depth: usize) -> Result<()> {
        self.trivia()?;
        let c = self.tree.checkpoint()?;

        // Inner attributes are items of their own.
        if self.attribute(true)? {
            self.tree.close_at(c, Other)?;
            return Ok(());
        }

        self.attributes()?;
        self.visibility()?;
        let kind = self.kind()?;

        match kind {
            Fn | Trait => {
                self.bump()?;
                self.name()?;
                self.skip_rest()?;
            }
            Struct | Union => {
                self.bump()?;
                self.name()?;
                self.struct_rest()?;
            }
            Enum => {
                self.bump()?;
                self.name()?;
                self.enum_rest()?;
            }
            Const | TypeAlias => {
                self.bump()?;
                self.name()?;
                self.skip_to_semi()?;
            }
            Static => {
                self.bump()?;

                if self.is("mut") {
                    self.bump()?;
                }

                self.name()?;
                self.skip_to_semi()?;
            }
            Mod => {
                self.bump()?;
                self.name()?;

                if self.peek() == Some(OpenBrace) {
                    self.block(depth)?;
                } else {
                    self.skip_rest()?;
                }
            }
            Impl => {
                self.bump()?;
                self.impl_rest(depth)?;
            }
            MacroRules => {
                self.bump()?;
                self.bump()?;
                self.name()?;

                if self.peek().is_some_and(is_open) {
                    self.group()?;
                }

                if self.is(";") {
                    self.bump()?;
                }
            }
            Use => {
                self.bump()?;
                self.use_rest()?;
            }
            ExternCrate => self.skip_to_semi()?,
            _ => self.skip_rest()?,
        }

        self.tree.close_at(c, kind)?;
        Ok(())
    }

    fn struct_rest(&mut self) -> Result<()> {
        while let Some(syntax) = self.peek() {
            match syntax {
                OpenBrace => return self.fields(),
                OpenParen | OpenBracket => self.group()?,
                syntax if is_close(syntax) => break,
                _ if self.is(";") => return self.bump(),
                _ => self.bump()?,
            }
        }

        Ok(())
    }

    fn enum_rest(&mut self) -> Result<()> {
        while let Some(syntax) = self.peek() {
            match syntax {
                OpenBrace => return self.variants(),
                OpenParen | OpenBracket => self.group()?,
                syntax if is_close(syntax) => break,
                _ if self.is(";") => return self.bump(),
                _ => self.bump()?,
            }
        }

        Ok(())
    }

    fn fields(&mut self) -> Result<()> {
        self.open(Fields)?;
        self.bump()?;

        while let Some(syntax) = self.peek() {
            if syntax == CloseBrace {
                self.bump()?;
                break;
            }

            self.open(Field)?;
            self.attributes()?;
            self.visibility()?;
            self.name()?;

            // Commas inside generic arguments of the field type are skipped by
            // counting angle brackets.
            let mut angle = 0usize;

            while let Some(syntax) = self.peek() {
                match syntax {
                    CloseBrace => break,
                    syntax if is_open(syntax) => self.group()?,
                    _ if angle == 0 && self.is(",") => {
                        self.bump()?;
                        break;
                    }
                    _ => {
                        if self.is("<") {
                            angle = angle.saturating_add(1);
                        } else if self.is(">") {
                            angle = angle.saturating_sub(1);
                        }

                        self.bump()?;
                    }
                }
            }

            self.tree.close()?;
        }

        self.tree.close()?;
        Ok(())
    }

    fn variants(&mut self) -> Result<()> {
        self.open(Variants)?;
        self.bump()?;

        while let Some(syntax) = self.peek() {
            if syntax == CloseBrace {
                self.bump()?;
                break;
            }

            self.open(Variant)?;
            self.attributes()?;
            self.name()?;

            while let Some(syntax) = self.peek() {
                match syntax {
                    CloseBrace => break,
                    syntax if is_open(syntax) => self.group()?,
                    _ if self.is(",") => {
                        self.bump()?;
                        break;
                    }
                    _ => self.bump()?,
                }
            }

            self.tree.close()?;
        }

        self.tree.close()?;
        Ok(())
    }

    /// The header and items of an impl block, positioned after `impl`.
    fn impl_rest(&mut self, depth: usize) -> Result<()> {
        let mut angle = 0usize;

        if self.is("<") {
            while let Some(syntax) = self.peek() {
                if is_open(syntax) {
                    self.group()?;
                    continue;
                }

                if is_close(syntax) {
                    break;
                }

                if self.is("<") {
                    angle = angle.saturating_add(1);
                } else if self.is(">") {
                    angle = angle.saturating_sub(1);
                }

                self.bump()?;

                if angle == 0 {
                    break;
                }
            }
        }

        if self.is("!") {
            self.bump()?;
        }

        self.trivia()?;
        let mut c = self.tree.checkpoint()?;
        angle = 0;

        while let Some(syntax) = self.peek() {
            match syntax {
                OpenBrace => break,
                OpenParen | OpenBracket => self.group()?,
                syntax if is_close(syntax) => break,
                _ if angle == 0 && (self.is(";") || self.is("where")) => break,
                _ if angle == 0 && self.is("for") && !self.nth_is(1, "<") => {
                    self.tree.close_at(c, ImplTrait)?;
                    self.bump()?;
                    self.trivia()?;
                    c = self.tree.checkpoint()?;
                }
                _ => {
                    if self.is("<") {
                        angle = angle.saturating_add(1);
                    } else if self.is(">") {
                        angle = angle.saturating_sub(1);
                    }

                    self.bump()?;
                }
            }
        }

        self.tree.close_at(c, SelfType)?;

        while let Some(syntax) = self.peek() {
            match syntax {
                OpenBrace => return self.block(depth),
                OpenParen | OpenBracket => self.group()?,
                syntax if is_close(syntax) => break,
                _ if self.is(";") => return self.bump(),
                _ => self.bump()?,
            }
        }

        Ok(())
    }

    /// The tree of a use declaration, positioned after `use`.
    fn use_rest(&mut self) -> Result<()> {
        self.open(UsePath)?;

        while let Some(tok) = self.nth(0) {
            let path = match tok.syntax {
                Ident => self.text(tok) != "as",
                Punct => self.text(tok) == "::",
                _ => false,
            };

            if !path {
                break;
            }

            self.bump()?;
        }

        self.tree.close()?;

        if self.is("as") {
            self.bump()?;
            self.open(Rename)?;

            if self.peek() == Some(Ident) || self.is("_") {
                self.bump()?;
            }

            self.tree.close()?;
        }

        self.skip_to_semi()
    }
}

/// A parsed module file.
pub(super) struct File {
    source: String,
    tree: Tree<Syntax, FlavorDefault>,
}

type N<'a> = Node<'a, Syntax, FlavorDefault>;

/// Parse the items of a module file.
pub(super) fn parse(source: String) -> Result<File> {
    let mut lexer = Lexer {
        source: &source,
        pos: 0,
    };

    let mut tokens = Vec::new();

    while let Some(tok) = lexer.next() {
        tokens.push(tok);
    }

    let mut parser = Parser {
        source: &source,
        tokens,
        pos: 0,
        tree: Builder::new(),
    };

    parser.open(Root)?;
    parser.items(0, false)?;
    parser.tree.close()?;
    parser.trivia()?;

    let tree = parser.tree.build()?;
    Ok(File { source, tree })
}

fn strip_raw(ident: &str) -> &str {
    ident.strip_prefix("r#").unwrap_or(ident)
}

impl File {
    fn text(&self, node: N<'_>) -> &str {
        self.source.get(node.range()).unwrap_or_default()
    }

    fn root(&self) -> Option<Scope<'_>> {
        let node = self.tree.children().find_map(|e| e.into_node())?;
        Some(Scope { file: self, node })
    }

    /// The scope of the inline module at `path` in the file, the file itself
    /// when it is empty.
    pub(super) fn scope<S>(&self, path: &[S]) -> Option<Scope<'_>>
    where
        S: AsRef<str>,
    {
        let mut scope = self.root()?;

        for name in path {
            scope = scope.items().find_map(|item| {
                (item.kind() == Mod && item.name() == Some(name.as_ref()))
                    .then(|| item.block())
                    .flatten()
            })?;
        }

        Some(scope)
    }

    /// The impl blocks for the type named `ty` anywhere in the file.
    fn impls(&self, ty: &str) -> impl Iterator<Item = Item<'_>> {
        self.tree
            .walk()
            .filter_map(|e| e.into_node())
            .filter(|node| node.value() == Impl)
            .map(|node| Item { file: self, node })
            .filter(move |item| item.self_type() == Some(ty))
    }
}

/// The items of a module.
#[derive(Clone, Copy)]
pub(super) struct Scope<'a> {
    file: &'a File,
    node: N<'a>,
}

impl<'a> Scope<'a> {
    pub(super) fn items(self) -> impl Iterator<Item = Item<'a>> {
        self.node.children().filter_map(move |e| {
            let node = e.into_node()?;
            (!matches!(node.value(), Group)).then_some(Item {
                file: self.file,
                node,
            })
        })
    }

    /// The members named `name` of the type `ty` declared in this scope:
    /// its variants or fields, and the items of its impl blocks in the file.
    pub(super) fn members(self, ty: &str, name: &str) -> Vec<Member> {
        let mut members = Vec::new();

        for item in self.items().filter(|item| item.name() == Some(ty)) {
            match item.kind() {
                Enum if item.variants().any(|v| v == name) => members.push(Member::Variant),
                Struct | Union if item.fields().any(|(field, public)| public && field == name) => {
                    members.push(Member::Field)
                }
                _ => {}
            }
        }

        for imp in self.file.impls(ty) {
            let is_trait = imp.child(ImplTrait).is_some();

            let Some(block) = imp.block() else {
                continue;
            };

            for item in block.items().filter(|item| item.name() == Some(name)) {
                let member = match item.kind() {
                    Fn if is_trait || item.is_public() => Member::Method,
                    Const if is_trait || item.is_public() => Member::Const,
                    TypeAlias => Member::Type,
                    _ => continue,
                };

                if !members.contains(&member) {
                    members.push(member);
                }
            }
        }

        members
    }
}

/// A member of a type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Member {
    Method,
    Const,
    Type,
    Variant,
    Field,
}

/// A simple `use` path.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct UseTree<'a> {
    pub(super) path: Vec<&'a str>,
    pub(super) rename: Option<&'a str>,
    /// Whether the path ends in `*` or a `{ .. }` group.
    pub(super) nested: bool,
}

impl<'a> UseTree<'a> {
    /// The name the use declaration binds, if it is a simple one.
    pub(super) fn name(&self) -> Option<&'a str> {
        if self.nested {
            return None;
        }

        match self.rename {
            Some("_") => None,
            Some(rename) => Some(rename),
            None => self.path.last().copied(),
        }
    }
}

/// An item declaration.
#[derive(Clone, Copy)]
pub(super) struct Item<'a> {
    file: &'a File,
    node: N<'a>,
}

impl<'a> Item<'a> {
    fn child(self, syntax: Syntax) -> Option<N<'a>> {
        self.node
            .children()
            .filter_map(|e| e.into_node())
            .find(|n| n.value() == syntax)
    }

    fn idents(self, node: N<'a>) -> impl Iterator<Item = &'a str> {
        node.children()
            .filter_map(|e| e.into_token())
            .filter(|t| t.value() == Ident)
            .filter_map(|t| self.file.source.get(t.range()))
            .map(strip_raw)
    }

    pub(super) fn kind(self) -> Syntax {
        self.node.value()
    }

    pub(super) fn name(self) -> Option<&'a str> {
        Some(strip_raw(self.file.text(self.child(Name)?)))
    }

    /// Whether the item is visible as `pub`, without restrictions.
    pub(super) fn is_public(self) -> bool {
        self.child(Visibility)
            .is_some_and(|v| self.file.text(v) == "pub")
    }

    /// The contents of the attributes of the item.
    fn attributes(self) -> impl Iterator<Item = &'a str> {
        self.node
            .children()
            .filter_map(|e| e.into_node())
            .filter(|n| n.value() == Attribute)
            .filter_map(|n| {
                let text = self.file.text(n).strip_prefix('#')?.trim_start();
                let text = text.strip_prefix('!').unwrap_or(text).trim_start();
                Some(text.strip_prefix('[')?.strip_suffix(']')?.trim())
            })
    }

    fn attribute(self, name: &str) -> Option<&'a str> {
        self.attributes().find_map(|a| {
            let rest = a.strip_prefix(name)?;
            (!rest.starts_with(is_ident_continue)).then_some(rest.trim_start())
        })
    }

    pub(super) fn is_macro_export(self) -> bool {
        self.attribute("macro_export").is_some()
    }

    /// The value of a `#[path = ".."]` attribute.
    pub(super) fn path_attribute(self) -> Option<&'a str> {
        let value = self.attribute("path")?.strip_prefix('=')?.trim();
        value.strip_prefix('"')?.strip_suffix('"')
    }

    /// The items of an inline module or impl block.
    pub(super) fn block(self) -> Option<Scope<'a>> {
        Some(Scope {
            file: self.file,
            node: self.child(Block)?,
        })
    }

    /// The name of the type of an impl block.
    pub(super) fn self_type(self) -> Option<&'a str> {
        let node = self.child(SelfType)?;
        let mut name = None;

        for e in node.children() {
            let Some(token) = e.into_token() else {
                break;
            };

            let text = self.file.source.get(token.range()).unwrap_or_default();

            match token.value() {
                Ident if !matches!(text, "mut" | "dyn" | "const") => name = Some(strip_raw(text)),
                Punct if text == "<" => break,
                _ => {}
            }
        }

        name
    }

    pub(super) fn variants(self) -> impl Iterator<Item = &'a str> {
        self.members(Variants, Variant).map(|(name, _)| name)
    }

    /// The named fields of a struct or union, and whether they are public.
    pub(super) fn fields(self) -> impl Iterator<Item = (&'a str, bool)> {
        self.members(Fields, Field)
    }

    fn members(self, outer: Syntax, inner: Syntax) -> impl Iterator<Item = (&'a str, bool)> {
        self.child(outer)
            .into_iter()
            .flat_map(|n| n.children())
            .filter_map(|e| e.into_node())
            .filter(move |n| n.value() == inner)
            .filter_map(move |node| {
                let item = Item {
                    file: self.file,
                    node,
                };
                Some((item.name()?, item.is_public()))
            })
    }

    pub(super) fn use_tree(self) -> Option<UseTree<'a>> {
        let path = self.idents(self.child(UsePath)?).collect();

        let rename = self.child(Rename).map(|n| strip_raw(self.file.text(n)));

        let nested = self.node.children().any(|e| {
            e.value() == Group
                || (e.value() == Punct && self.file.source.get(e.range()) == Some("*"))
        });

        Some(UseTree {
            path,
            rename,
            nested,
        })
    }
}

#[cfg(test)]
mod tests;
