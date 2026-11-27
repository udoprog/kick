//! Rewriting of rustdoc intra-doc links into absolute docs.rs links for the
//! generated README.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use anyhow::Result;
use pulldown_cmark::{BrokenLink, CowStr, Event, LinkType, Options, Parser, Tag};
use relative_path::{RelativePath, RelativePathBuf};
use reqwest::Url;

/// Upper bound on the number of module files visited when looking for
/// exported macros.
const MAX_MODULES: usize = 1024;

/// Access to the crate sources.
pub(super) trait Sources {
    fn read(&self, path: &RelativePath) -> Result<Option<String>>;
}

/// Where the crate documentation lives on docs.rs.
pub(super) struct DocsRs<'a> {
    pub(super) package: Option<&'a str>,
    pub(super) version: Option<&'a str>,
    /// The identifier of the library crate, `None` if the package has no
    /// library target.
    pub(super) crate_ident: Option<String>,
    pub(super) entry: &'a RelativePath,
}

/// A link which could not be rewritten.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct LinkError {
    pub(super) offset: usize,
    pub(super) link: String,
    pub(super) reason: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Struct,
    Enum,
    Trait,
    Union,
    Mod,
    Const,
    Fn,
    Type,
    Static,
    Macro,
    Field,
    Variant,
    Method,
}

impl Kind {
    fn page(self, name: &str) -> Option<String> {
        let prefix = match self {
            Kind::Struct => "struct",
            Kind::Enum => "enum",
            Kind::Trait => "trait",
            Kind::Union => "union",
            Kind::Const => "constant",
            Kind::Fn => "fn",
            Kind::Type => "type",
            Kind::Static => "static",
            Kind::Macro => "macro",
            Kind::Mod => return Some(format!("{name}/index.html")),
            Kind::Field | Kind::Variant | Kind::Method => return None,
        };

        Some(format!("{prefix}.{name}.html"))
    }

    fn is_type(self) -> bool {
        matches!(self, Kind::Struct | Kind::Enum | Kind::Union | Kind::Trait)
    }
}

const TYPE_NS: &[Kind] = &[
    Kind::Struct,
    Kind::Enum,
    Kind::Trait,
    Kind::Union,
    Kind::Type,
    Kind::Mod,
];
const VALUE_NS: &[Kind] = &[Kind::Fn, Kind::Const, Kind::Static];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Filter {
    Any,
    Kinds(&'static [Kind]),
    Unsupported,
}

impl Filter {
    fn from_disambiguator(d: &str) -> Option<Self> {
        let kinds: &'static [Kind] = match d {
            "struct" => &[Kind::Struct],
            "enum" => &[Kind::Enum],
            "trait" => &[Kind::Trait],
            "union" => &[Kind::Union],
            "mod" | "module" => &[Kind::Mod],
            "const" | "constant" => &[Kind::Const],
            "fn" | "function" | "method" => &[Kind::Fn, Kind::Method],
            "tyalias" => &[Kind::Type],
            "static" => &[Kind::Static],
            "macro" => &[Kind::Macro],
            "field" => &[Kind::Field],
            "variant" => &[Kind::Variant],
            "type" => TYPE_NS,
            "value" => VALUE_NS,
            "derive" | "attr" | "prim" | "primitive" => return Some(Filter::Unsupported),
            _ => return None,
        };

        Some(Filter::Kinds(kinds))
    }

    fn allows(self, kind: Kind) -> bool {
        match self {
            Filter::Any => true,
            Filter::Kinds(kinds) => kinds.contains(&kind),
            Filter::Unsupported => false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Suffix {
    None,
    Bang,
    Parens,
}

/// A link destination which looks like a rustdoc item path.
#[derive(Debug)]
struct ItemLink<'a> {
    filter: Filter,
    path: Vec<&'a str>,
    suffix: Suffix,
    generics: bool,
    /// Whether the link unmistakably refers to an item, as opposed to a bare
    /// word in brackets.
    explicit: bool,
}

fn is_ident(s: &str) -> bool {
    let mut chars = s.chars();

    let Some(first) = chars.next() else {
        return false;
    };

    (first.is_alphabetic() || first == '_')
        && chars.all(|c| c.is_alphanumeric() || c == '_')
        && s != "_"
}

impl<'a> ItemLink<'a> {
    fn parse(s: &'a str) -> Option<Self> {
        let s = s.trim();

        let (s, ticks) = match s.strip_prefix('`').and_then(|s| s.strip_suffix('`')) {
            Some(s) => (s, true),
            None => (s, false),
        };

        let (filter, s) = match s.split_once('@') {
            Some((d, rest)) => (Filter::from_disambiguator(d)?, rest),
            None => (Filter::Any, s),
        };

        let (s, suffix) = if let Some(s) = s.strip_suffix('!') {
            (s, Suffix::Bang)
        } else if let Some(s) = s.strip_suffix("()") {
            (s, Suffix::Parens)
        } else {
            (s, Suffix::None)
        };

        let (s, generics) = match s.find('<') {
            Some(index) if s.ends_with('>') => (s.get(..index)?, true),
            Some(..) => return None,
            None => (s, false),
        };

        let path = s.split("::").collect::<Vec<_>>();

        if !path.iter().all(|segment| is_ident(segment)) {
            return None;
        }

        let explicit =
            ticks || filter != Filter::Any || suffix != Suffix::None || generics || path.len() > 1;

        Some(Self {
            filter,
            path,
            suffix,
            generics,
            explicit,
        })
    }
}

/// Test if the given link destination is an intra-doc link.
pub(super) fn is_item_link(dest: &str) -> bool {
    ItemLink::parse(dest).is_some()
}

enum Unresolved {
    NotFound(String),
    Other(String),
}

impl Unresolved {
    fn into_reason(self) -> String {
        match self {
            Unresolved::NotFound(reason) | Unresolved::Other(reason) => reason,
        }
    }
}

fn other(reason: impl Into<String>) -> Unresolved {
    Unresolved::Other(reason.into())
}

/// A public item declared at the top level of a module file.
struct Item<'a> {
    kind: Kind,
    name: &'a str,
    inline: bool,
}

fn split_word(s: &str) -> (&str, &str) {
    let index = s
        .find(|c: char| !(c.is_alphanumeric() || c == '_'))
        .unwrap_or(s.len());
    (
        s.get(..index).unwrap_or_default(),
        s.get(index..).unwrap_or_default(),
    )
}

/// Parse a public item declaration starting at column zero, which is where
/// rustfmt puts module-level items.
fn parse_item(line: &str) -> Option<Item<'_>> {
    let mut rest = line.strip_prefix("pub ")?.trim_start();

    loop {
        let (word, tail) = split_word(rest);
        let tail = tail.trim_start();

        let kind = match word {
            "unsafe" | "async" => {
                rest = tail;
                continue;
            }
            "extern" => {
                rest = match tail.strip_prefix('"') {
                    Some(abi) => abi.split_once('"')?.1.trim_start(),
                    None => tail,
                };
                continue;
            }
            "const" => {
                if matches!(split_word(tail).0, "fn" | "unsafe" | "async" | "extern") {
                    rest = tail;
                    continue;
                }

                Kind::Const
            }
            "static" => {
                let (next, after) = split_word(tail);

                if next == "mut" {
                    rest = after.trim_start();
                    let (name, _) = split_word(rest);
                    return is_ident(name).then_some(Item {
                        kind: Kind::Static,
                        name,
                        inline: false,
                    });
                }

                Kind::Static
            }
            "struct" => Kind::Struct,
            "enum" => Kind::Enum,
            "trait" => Kind::Trait,
            "union" => Kind::Union,
            "mod" => Kind::Mod,
            "fn" => Kind::Fn,
            "type" => Kind::Type,
            _ => return None,
        };

        let (name, after) = split_word(tail);

        if !is_ident(name) {
            return None;
        }

        let inline = kind == Kind::Mod && !after.trim_start().starts_with(';');
        return Some(Item { kind, name, inline });
    }
}

/// Parse a `mod name;` declaration of any visibility.
fn parse_mod_decl(line: &str) -> Option<&str> {
    let mut rest = line.trim_start();

    if let Some(after) = rest.strip_prefix("pub") {
        rest = match after.trim_start().strip_prefix('(') {
            Some(vis) => vis.split_once(')')?.1,
            None => after,
        }
        .trim_start();
    }

    let (name, after) = split_word(rest.strip_prefix("mod ")?.trim_start());
    (is_ident(name) && after.trim_start().starts_with(';')).then_some(name)
}

/// Find a `#[macro_export]`ed `macro_rules!` definition of `name`.
fn defines_exported_macro(source: &str, name: &str) -> bool {
    let lines = source.lines().collect::<Vec<_>>();

    for (index, line) in lines.iter().enumerate() {
        let Some(rest) = line.trim_start().strip_prefix("macro_rules!") else {
            continue;
        };

        if split_word(rest.trim_start()).0 != name {
            continue;
        }

        let attributes = lines
            .get(..index)
            .unwrap_or_default()
            .iter()
            .rev()
            .map(|line| line.trim())
            .take_while(|line| line.starts_with("#[") || line.starts_with("//"));

        if attributes
            .into_iter()
            .any(|line| line.starts_with("#[macro_export"))
        {
            return true;
        }
    }

    false
}

/// Test if a method named `name` is declared in an impl block of the source.
fn defines_method(source: &str, name: &str) -> bool {
    source.lines().any(|line| {
        if !line.starts_with(char::is_whitespace) {
            return false;
        }

        parse_item(line.trim_start()).is_some_and(|item| item.kind == Kind::Fn && item.name == name)
    })
}

struct Module {
    path: RelativePathBuf,
    /// Whether child modules are found next to this file, as for `lib.rs`
    /// and `mod.rs`.
    is_root: bool,
}

impl Module {
    fn child_dir(&self) -> Option<RelativePathBuf> {
        let parent = self.path.parent()?;

        if self.is_root {
            Some(parent.to_owned())
        } else {
            Some(parent.join(self.path.file_stem()?))
        }
    }
}

struct Resolver<'a> {
    docs: &'a DocsRs<'a>,
    sources: &'a dyn Sources,
    cache: HashMap<RelativePathBuf, Option<String>>,
}

impl Resolver<'_> {
    fn read(&mut self, path: &RelativePath) -> Result<Option<&str>, Unresolved> {
        if !self.cache.contains_key(path) {
            let source = self
                .sources
                .read(path)
                .map_err(|e| other(format!("{path}: {e}")))?;
            self.cache.insert(path.to_owned(), source);
        }

        Ok(self.cache.get(path).and_then(|s| s.as_deref()))
    }

    fn root(&self) -> Module {
        Module {
            path: self.docs.entry.to_owned(),
            is_root: true,
        }
    }

    fn child(&mut self, module: &Module, name: &str) -> Result<Module, Unresolved> {
        let dir = module
            .child_dir()
            .ok_or_else(|| other(format!("cannot find the directory of `{}`", module.path)))?;

        let file = dir.join(format!("{name}.rs"));

        if self.read(&file)?.is_some() {
            return Ok(Module {
                path: file,
                is_root: false,
            });
        }

        let file = dir.join(name).join("mod.rs");

        if self.read(&file)?.is_some() {
            return Ok(Module {
                path: file,
                is_root: true,
            });
        }

        Err(other(format!(
            "cannot find the source of module `{name}` (`#[path]` attributes are not supported)"
        )))
    }

    fn items(&mut self, module: &Module, name: &str) -> Result<Vec<(Kind, bool)>, Unresolved> {
        let source = self
            .read(&module.path)?
            .ok_or_else(|| other(format!("cannot read the module source `{}`", module.path)))?;

        let mut items = Vec::new();

        for line in source.lines() {
            if let Some(item) = parse_item(line)
                && item.name == name
                && !items.contains(&(item.kind, item.inline))
            {
                items.push((item.kind, item.inline));
            }
        }

        Ok(items)
    }

    fn find_macro(&mut self, name: &str) -> Result<bool, Unresolved> {
        let mut queue = vec![self.root()];
        let mut seen = HashSet::new();

        while let Some(module) = queue.pop() {
            if seen.len() >= MAX_MODULES || !seen.insert(module.path.clone()) {
                continue;
            }

            let Some(source) = self.read(&module.path)? else {
                continue;
            };

            if defines_exported_macro(source, name) {
                return Ok(true);
            }

            let children = source
                .lines()
                .filter_map(parse_mod_decl)
                .map(str::to_owned)
                .collect::<Vec<_>>();

            for child in children {
                if let Ok(child) = self.child(&module, &child) {
                    queue.push(child);
                }
            }
        }

        Ok(false)
    }

    fn base(&self) -> Result<String, Unresolved> {
        let crate_ident = self.docs.crate_ident.as_deref().ok_or_else(|| {
            other("the package has no library target, so docs.rs has no item documentation to link to")
        })?;

        let package = self
            .docs
            .package
            .ok_or_else(|| other("the README does not belong to a package"))?;

        let version = self
            .docs
            .version
            .ok_or_else(|| other("the package version could not be determined"))?;

        Ok(format!(
            "https://docs.rs/{package}/{version}/{crate_ident}/"
        ))
    }

    fn resolve(&mut self, link: &ItemLink<'_>) -> Result<String, Unresolved> {
        let path = self.resolve_path(link)?;
        Ok(format!("{}{path}", self.base()?))
    }

    /// Resolve the path of the item's page relative to the crate root.
    fn resolve_path(&mut self, link: &ItemLink<'_>) -> Result<String, Unresolved> {
        if link.filter == Filter::Unsupported {
            return Err(other("this kind of item is not supported"));
        }

        if link.generics {
            return Err(other("links with generic parameters are not supported"));
        }

        let mut path = link.path.as_slice();

        if let Some((first, rest)) = path.split_first()
            && matches!(*first, "crate" | "self")
        {
            path = rest;
        }

        if path
            .iter()
            .any(|segment| matches!(*segment, "crate" | "self" | "super" | "Self"))
        {
            return Err(other("relative paths are not supported"));
        }

        let Some((name, modules)) = path.split_last() else {
            return Ok("index.html".to_owned());
        };

        if link.suffix == Suffix::Bang || link.filter == Filter::Kinds(&[Kind::Macro]) {
            if !modules.is_empty() {
                return Err(other("exported macros live at the crate root"));
            }

            if !self.find_macro(name)? {
                return Err(Unresolved::NotFound(format!(
                    "no `#[macro_export]` macro named `{name}` found"
                )));
            }

            return Ok(format!("macro.{name}.html"));
        }

        let mut module = self.root();
        let mut url = String::new();

        for (index, segment) in modules.iter().enumerate() {
            let items = self.items(&module, segment)?;

            if items.contains(&(Kind::Mod, false)) {
                module = self.child(&module, segment)?;
                url.push_str(segment);
                url.push('/');
                continue;
            }

            if items.contains(&(Kind::Mod, true)) {
                return Err(other(format!("inline module `{segment}` is not supported")));
            }

            if index.checked_add(1) == Some(modules.len())
                && let Some((kind, _)) = items.iter().find(|(kind, _)| kind.is_type())
            {
                let page = kind
                    .page(segment)
                    .ok_or_else(|| other("unsupported parent item"))?;
                let anchor = self.member(*kind, &module, name, link)?;
                return Ok(format!("{url}{page}#{anchor}"));
            }

            return Err(Unresolved::NotFound(format!(
                "no public module or type `{segment}` declared in `{}` (re-exports, external crates and items defined by macros are not supported)",
                module.path
            )));
        }

        let candidates = self
            .items(&module, name)?
            .into_iter()
            .filter(|(kind, _)| link.filter.allows(*kind))
            .filter(|(kind, _)| link.suffix != Suffix::Parens || *kind == Kind::Fn)
            .map(|(kind, _)| kind)
            .collect::<Vec<_>>();

        let kind = match candidates.as_slice() {
            [kind] => *kind,
            [] => {
                return Err(Unresolved::NotFound(format!(
                    "no matching public item `{name}` declared in `{}` (re-exports, external crates and items defined by macros are not supported)",
                    module.path
                )));
            }
            _ => {
                return Err(other(format!(
                    "`{name}` is ambiguous, add a disambiguator such as `struct@` or `fn@`"
                )));
            }
        };

        let page = kind
            .page(name)
            .ok_or_else(|| other("unsupported item kind"))?;
        Ok(format!("{url}{page}"))
    }

    fn member(
        &mut self,
        parent: Kind,
        module: &Module,
        name: &str,
        link: &ItemLink<'_>,
    ) -> Result<String, Unresolved> {
        if parent == Kind::Trait {
            return Err(other("links to trait members are not supported"));
        }

        let kind = match link.filter {
            Filter::Kinds(&[Kind::Field]) => Kind::Field,
            Filter::Kinds(&[Kind::Variant]) => Kind::Variant,
            Filter::Kinds(&[Kind::Const]) => Kind::Const,
            Filter::Kinds(&[Kind::Fn, Kind::Method]) => Kind::Method,
            Filter::Any if link.suffix == Suffix::Parens => Kind::Method,
            Filter::Any if parent == Kind::Enum && name.starts_with(|c: char| c.is_uppercase()) => {
                Kind::Variant
            }
            Filter::Any => {
                let source = self.read(&module.path)?.unwrap_or_default();

                if !defines_method(source, name) {
                    return Err(other(format!(
                        "cannot tell what kind of member `{name}` is, add a disambiguator such as `method@` or `field@`"
                    )));
                }

                Kind::Method
            }
            _ => return Err(other("unsupported member disambiguator")),
        };

        let anchor = match (parent, kind) {
            (_, Kind::Method) => "method",
            (_, Kind::Const) => "associatedconstant",
            (Kind::Enum, Kind::Variant) => "variant",
            (Kind::Struct | Kind::Union, Kind::Field) => "structfield",
            _ => return Err(other("unsupported member for this kind of item")),
        };

        Ok(format!("{anchor}.{name}"))
    }
}

enum Dest<'a> {
    Item(ItemLink<'a>),
    Relative,
    Other,
}

fn classify(dest: &str) -> Dest<'_> {
    if let Some(link) = ItemLink::parse(dest) {
        return Dest::Item(link);
    }

    if dest.starts_with('#') || dest.is_empty() {
        return Dest::Other;
    }

    match str::parse::<Url>(dest) {
        Err(url::ParseError::RelativeUrlWithoutBase) => Dest::Relative,
        _ => Dest::Other,
    }
}

const RELATIVE: &str = "relative links do not work from the README, use an absolute URL";

/// Find the range of the link destination token in `source`, which starts
/// right after a `(` or `]:`.
fn dest_token(source: &str, start: usize) -> Option<Range<usize>> {
    let rest = source.get(start..)?;
    let start = start.checked_add(rest.len().saturating_sub(rest.trim_start().len()))?;
    let rest = source.get(start..)?;

    let len = if rest.starts_with('<') {
        rest.find('>')?.checked_add(1)?
    } else {
        rest.find(|c: char| c.is_whitespace() || c == ')')
            .unwrap_or(rest.len())
    };

    Some(start..start.checked_add(len)?)
}

fn strip_angle(token: &str) -> &str {
    token
        .strip_prefix('<')
        .and_then(|t| t.strip_suffix('>'))
        .unwrap_or(token)
}

fn item_callback(link: BrokenLink<'_>) -> Option<(CowStr<'_>, CowStr<'_>)> {
    ItemLink::parse(&link.reference)?;
    Some((link.reference, CowStr::Borrowed("")))
}

/// Rewrite intra-doc links in `markdown` into absolute docs.rs links.
pub(super) fn rewrite(
    markdown: &str,
    docs: &DocsRs<'_>,
    sources: &dyn Sources,
) -> Result<String, Vec<LinkError>> {
    let mut resolver = Resolver {
        docs,
        sources,
        cache: HashMap::new(),
    };

    let mut edits = Vec::<(Range<usize>, String)>::new();
    let mut errors = Vec::new();

    let parser =
        Parser::new_with_broken_link_callback(markdown, Options::empty(), Some(item_callback));

    let mut defs = parser
        .reference_definitions()
        .iter()
        .map(|(_, def)| (def.span.clone(), def.dest.to_string()))
        .collect::<Vec<_>>();

    defs.sort_by_key(|(span, _)| span.start);

    for (span, dest) in defs {
        let link = match classify(&dest) {
            Dest::Item(link) => link,
            Dest::Relative => {
                errors.push(LinkError {
                    offset: span.start,
                    link: dest,
                    reason: RELATIVE.to_owned(),
                });
                continue;
            }
            Dest::Other => continue,
        };

        let result = resolver
            .resolve(&link)
            .map_err(Unresolved::into_reason)
            .and_then(|url| {
                let token = markdown
                    .get(span.clone())
                    .and_then(|s| s.find("]:"))
                    .and_then(|colon| {
                        dest_token(markdown, span.start.checked_add(colon)?.checked_add(2)?)
                    })
                    .filter(|token| {
                        markdown.get(token.clone()).map(strip_angle) == Some(dest.as_str())
                    })
                    .ok_or_else(|| {
                        "the link definition is too complicated to rewrite".to_owned()
                    })?;

                edits.push((token, url));
                Ok(())
            });

        if let Err(reason) = result {
            errors.push(LinkError {
                offset: span.start,
                link: dest,
                reason,
            });
        }
    }

    for (event, range) in parser.into_offset_iter() {
        let Event::Start(Tag::Link {
            link_type,
            dest_url,
            ..
        }) = event
        else {
            continue;
        };

        let Some(source) = markdown.get(range.clone()) else {
            continue;
        };

        let link = match (link_type, classify(&dest_url)) {
            (
                LinkType::Inline
                | LinkType::ShortcutUnknown
                | LinkType::CollapsedUnknown
                | LinkType::ReferenceUnknown,
                Dest::Item(link),
            ) => link,
            (LinkType::Inline, Dest::Relative) => {
                errors.push(LinkError {
                    offset: range.start,
                    link: dest_url.to_string(),
                    reason: RELATIVE.to_owned(),
                });
                continue;
            }
            _ => continue,
        };

        let url = match resolver.resolve(&link) {
            Ok(url) => url,
            Err(Unresolved::NotFound(..)) if !link.explicit && link_type != LinkType::Inline => {
                continue;
            }
            Err(e) => {
                errors.push(LinkError {
                    offset: range.start,
                    link: source.to_owned(),
                    reason: e.into_reason(),
                });
                continue;
            }
        };

        let edit = match link_type {
            LinkType::Inline => source.rfind("](").and_then(|index| {
                let token = dest_token(markdown, range.start.checked_add(index)?.checked_add(2)?)?;
                (markdown.get(token.clone()).map(strip_angle) == Some(&*dest_url))
                    .then_some((token, url))
            }),
            LinkType::ShortcutUnknown => Some((range.end..range.end, format!("({url})"))),
            // The range of a collapsed link might not include the trailing `[]`.
            LinkType::CollapsedUnknown => match source.strip_suffix("[]") {
                Some(rest) => Some(range.start.saturating_add(rest.len())..range.end),
                None => markdown
                    .get(range.end..)
                    .is_some_and(|rest| rest.starts_with("[]"))
                    .then(|| range.end..range.end.saturating_add(2)),
            }
            .map(|range| (range, format!("({url})"))),
            _ => source.rfind("][").map(|index| {
                let start = range.start.saturating_add(index).saturating_add(1);
                (start..range.end, format!("({url})"))
            }),
        };

        match edit {
            Some(edit) => edits.push(edit),
            None => errors.push(LinkError {
                offset: range.start,
                link: source.to_owned(),
                reason: "the link is too complicated to rewrite".to_owned(),
            }),
        }
    }

    if !errors.is_empty() {
        errors.sort_by_key(|e| e.offset);
        return Err(errors);
    }

    edits.sort_by_key(|(range, _)| range.start);

    let mut out = String::with_capacity(markdown.len());
    let mut at = 0;

    for (range, replacement) in edits {
        let Some(before) = markdown.get(at..range.start) else {
            continue;
        };

        out.push_str(before);
        out.push_str(&replacement);
        at = range.end;
    }

    out.push_str(markdown.get(at..).unwrap_or_default());
    Ok(out)
}
