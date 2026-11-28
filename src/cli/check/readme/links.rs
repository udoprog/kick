//! Rewriting of rustdoc intra-doc links into absolute docs.rs links for the
//! generated README.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::ops::Range;
use std::rc::Rc;

use anyhow::Result;
use pulldown_cmark::{BrokenLink, CowStr, Event, LinkType, Options, Parser, Tag};
use relative_path::{RelativePath, RelativePathBuf};
use reqwest::Url;

use super::items::{self, File, Member, Syntax};

/// Upper bound on the number of module files visited when looking for
/// exported macros.
const MAX_MODULES: usize = 1024;

/// Upper bound on the number of re-exports followed to find an item.
const MAX_REEXPORTS: usize = 16;

const UNSUPPORTED: &str = "re-exports other than `pub use self::..` or `pub use crate::..`, external crates and items defined by macros are not supported";

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

fn item_kind(syntax: Syntax) -> Option<Kind> {
    Some(match syntax {
        Syntax::Fn => Kind::Fn,
        Syntax::Struct => Kind::Struct,
        Syntax::Enum => Kind::Enum,
        Syntax::Union => Kind::Union,
        Syntax::Trait => Kind::Trait,
        Syntax::TypeAlias => Kind::Type,
        Syntax::Const => Kind::Const,
        Syntax::Static => Kind::Static,
        Syntax::Mod => Kind::Mod,
        _ => return None,
    })
}

/// A module, which is a file and a path of inline modules inside of it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Module {
    path: RelativePathBuf,
    /// Whether child modules are found next to this file, as for `lib.rs`
    /// and `mod.rs`.
    is_root: bool,
    inline: Vec<String>,
}

impl Module {
    /// The directory of child module files.
    fn child_dir(&self) -> Option<RelativePathBuf> {
        let parent = self.path.parent()?;

        let mut dir = if self.is_root {
            parent.to_owned()
        } else {
            parent.join(self.path.file_stem()?)
        };

        for name in &self.inline {
            dir.push(name);
        }

        Some(dir)
    }

    /// The directory `#[path]` attributes are relative to.
    fn path_dir(&self) -> Option<RelativePathBuf> {
        if self.inline.is_empty() {
            Some(self.path.parent()?.to_owned())
        } else {
            self.child_dir()
        }
    }

    fn inline(&self, name: &str) -> Self {
        let mut inline = self.inline.clone();
        inline.push(name.to_owned());

        Self {
            path: self.path.clone(),
            is_root: self.is_root,
            inline,
        }
    }
}

impl fmt::Display for Module {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}`", self.path)?;

        if !self.inline.is_empty() {
            write!(f, " (inline module `{}`)", self.inline.join("::"))?;
        }

        Ok(())
    }
}

/// A module in which items are looked up.
#[derive(Clone)]
struct Place {
    module: Module,
    /// The directory of the module documentation relative to the crate root.
    url: String,
    /// Whether the module is reachable through public modules.
    public: bool,
}

/// An item found in a module.
#[derive(Clone)]
struct Found {
    kind: Kind,
    /// The directory of the documentation page of the item.
    url: String,
    /// The name of the documentation page of the item.
    name: String,
    /// Whether the item is reachable through public items.
    public: bool,
    /// The module the item is defined in, and its name there.
    def: Module,
    def_name: String,
    /// For modules, where their items are.
    target: Option<Result<Module, String>>,
}

impl Found {
    fn enter(&self) -> Result<Place, Unresolved> {
        let module = match &self.target {
            Some(Ok(module)) => module.clone(),
            Some(Err(reason)) => return Err(other(reason.clone())),
            None => return Err(other(format!("`{}` is not a module", self.name))),
        };

        Ok(Place {
            module,
            url: format!("{}{}/", self.url, self.name),
            public: self.public,
        })
    }
}

struct Resolver<'a> {
    docs: &'a DocsRs<'a>,
    sources: &'a dyn Sources,
    cache: HashMap<RelativePathBuf, Option<Rc<File>>>,
}

impl Resolver<'_> {
    fn file(&mut self, path: &RelativePath) -> Result<Option<Rc<File>>, Unresolved> {
        if let Some(file) = self.cache.get(path) {
            return Ok(file.clone());
        }

        let file = match self
            .sources
            .read(path)
            .map_err(|e| other(format!("{path}: {e}")))?
        {
            Some(source) => Some(Rc::new(
                items::parse(source).map_err(|e| other(format!("{path}: {e}")))?,
            )),
            None => None,
        };

        self.cache.insert(path.to_owned(), file.clone());
        Ok(file)
    }

    fn module_file(&mut self, module: &Module) -> Result<Rc<File>, Unresolved> {
        self.file(&module.path)?
            .ok_or_else(|| other(format!("cannot read the module source `{}`", module.path)))
    }

    fn root(&self) -> Module {
        Module {
            path: self.docs.entry.to_owned(),
            is_root: true,
            inline: Vec::new(),
        }
    }

    fn root_place(&self) -> Place {
        Place {
            module: self.root(),
            url: String::new(),
            public: true,
        }
    }

    fn child(
        &mut self,
        module: &Module,
        name: &str,
        path: Option<&str>,
    ) -> Result<Module, Unresolved> {
        if let Some(path) = path {
            let file = module
                .path_dir()
                .ok_or_else(|| other(format!("cannot find the directory of {module}")))?
                .join_normalized(path);

            if self.file(&file)?.is_none() {
                return Err(other(format!(
                    "cannot find the source of module `{name}` at `{file}`"
                )));
            }

            return Ok(Module {
                path: file,
                is_root: true,
                inline: Vec::new(),
            });
        }

        let dir = module
            .child_dir()
            .ok_or_else(|| other(format!("cannot find the directory of {module}")))?;

        for (file, is_root) in [
            (dir.join(format!("{name}.rs")), false),
            (dir.join(name).join("mod.rs"), true),
        ] {
            if self.file(&file)?.is_some() {
                return Ok(Module {
                    path: file,
                    is_root,
                    inline: Vec::new(),
                });
            }
        }

        Err(other(format!("cannot find the source of module `{name}`")))
    }

    /// Find the items named `name` in a module, of any visibility, including
    /// those re-exported with a simple `pub use`.
    fn find(&mut self, place: &Place, name: &str, depth: usize) -> Result<Vec<Found>, Unresolved> {
        let file = self.module_file(&place.module)?;

        let scope = file
            .scope(&place.module.inline)
            .ok_or_else(|| other(format!("cannot find {}", place.module)))?;

        let mut found = Vec::new();
        let mut reexports = Vec::new();

        for item in scope.items() {
            if item.kind() == Syntax::Use {
                if item.is_public()
                    && let Some(tree) = item.use_tree()
                    && tree.name() == Some(name)
                {
                    reexports.push(
                        tree.path
                            .iter()
                            .map(|s| (*s).to_owned())
                            .collect::<Vec<_>>(),
                    );
                }

                continue;
            }

            if item.name() != Some(name) {
                continue;
            }

            let Some(kind) = item_kind(item.kind()) else {
                continue;
            };

            let target = if kind == Kind::Mod {
                Some(match item.block() {
                    Some(..) => Ok(place.module.inline(name)),
                    None => self
                        .child(&place.module, name, item.path_attribute())
                        .map_err(Unresolved::into_reason),
                })
            } else {
                None
            };

            found.push(Found {
                kind,
                url: place.url.clone(),
                name: name.to_owned(),
                public: place.public && item.is_public(),
                def: place.module.clone(),
                def_name: name.to_owned(),
                target,
            });
        }

        for path in reexports {
            found.extend(self.reexport(place, name, &path, depth)?);
        }

        Ok(found)
    }

    /// Follow a `pub use self::..` or `pub use crate::..` re-export.
    fn reexport(
        &mut self,
        place: &Place,
        name: &str,
        path: &[String],
        depth: usize,
    ) -> Result<Vec<Found>, Unresolved> {
        let depth = depth.saturating_add(1);

        if depth > MAX_REEXPORTS {
            return Err(other(format!(
                "too many nested re-exports when looking for `{name}`"
            )));
        }

        let (mut current, rest) = match path.split_first() {
            Some((first, rest)) if first == "crate" => (self.root_place(), rest),
            Some((first, rest)) if first == "self" => (place.clone(), rest),
            _ => return Ok(Vec::new()),
        };

        let Some((last, modules)) = rest.split_last() else {
            return Ok(Vec::new());
        };

        for segment in modules {
            let found = self.find(&current, segment, depth)?;

            let Some(module) = found.iter().find(|f| f.kind == Kind::Mod) else {
                return Ok(Vec::new());
            };

            current = module.enter()?;
        }

        let targets = self.find(&current, last, depth)?;

        // Items which are not reachable where they are defined are documented
        // where they are re-exported.
        Ok(targets
            .into_iter()
            .map(|target| {
                if target.public {
                    return target;
                }

                Found {
                    url: place.url.clone(),
                    name: name.to_owned(),
                    public: place.public,
                    ..target
                }
            })
            .collect())
    }

    fn find_public(&mut self, place: &Place, name: &str) -> Result<Vec<Found>, Unresolved> {
        let mut found = self.find(place, name, 0)?;
        found.retain(|f| f.public);
        Ok(found)
    }

    fn find_macro(&mut self, name: &str) -> Result<bool, Unresolved> {
        let mut queue = vec![self.root()];
        let mut seen = HashSet::new();

        while let Some(module) = queue.pop() {
            if seen.len() >= MAX_MODULES || !seen.insert(module.path.clone()) {
                continue;
            }

            let Some(file) = self.file(&module.path)? else {
                continue;
            };

            let Some(root) = file.scope::<String>(&[]) else {
                continue;
            };

            let mut scopes = vec![(root, module)];

            while let Some((scope, module)) = scopes.pop() {
                for item in scope.items() {
                    match item.kind() {
                        Syntax::MacroRules
                            if item.name() == Some(name) && item.is_macro_export() =>
                        {
                            return Ok(true);
                        }
                        Syntax::Mod => {
                            let Some(child) = item.name() else {
                                continue;
                            };

                            if let Some(block) = item.block() {
                                scopes.push((block, module.inline(child)));
                            } else if let Ok(child) =
                                self.child(&module, child, item.path_attribute())
                            {
                                queue.push(child);
                            }
                        }
                        _ => {}
                    }
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

        let mut place = self.root_place();

        for (index, segment) in modules.iter().enumerate() {
            let found = self.find_public(&place, segment)?;

            if let Some(module) = found.iter().find(|f| f.kind == Kind::Mod) {
                place = module.enter()?;
                continue;
            }

            if index.checked_add(1) == Some(modules.len())
                && let Some(parent) = found.iter().find(|f| f.kind.is_type())
            {
                let page = parent
                    .kind
                    .page(&parent.name)
                    .ok_or_else(|| other("unsupported parent item"))?;
                let anchor = self.member(parent, name, link)?;
                return Ok(format!("{}{page}#{anchor}", parent.url));
            }

            return Err(Unresolved::NotFound(format!(
                "no public module or type `{segment}` declared in {} ({UNSUPPORTED})",
                place.module
            )));
        }

        let mut candidates = Vec::<Found>::new();

        for found in self.find_public(&place, name)? {
            if !link.filter.allows(found.kind)
                || (link.suffix == Suffix::Parens && found.kind != Kind::Fn)
            {
                continue;
            }

            if !candidates
                .iter()
                .any(|c| c.kind == found.kind && c.url == found.url && c.name == found.name)
            {
                candidates.push(found);
            }
        }

        let found = match candidates.as_slice() {
            [found] => found,
            [] => {
                return Err(Unresolved::NotFound(format!(
                    "no matching public item `{name}` declared in {} ({UNSUPPORTED})",
                    place.module
                )));
            }
            _ => {
                return Err(other(format!(
                    "`{name}` is ambiguous, add a disambiguator such as `struct@` or `fn@`"
                )));
            }
        };

        let page = found
            .kind
            .page(&found.name)
            .ok_or_else(|| other("unsupported item kind"))?;
        Ok(format!("{}{page}", found.url))
    }

    fn member(
        &mut self,
        parent: &Found,
        name: &str,
        link: &ItemLink<'_>,
    ) -> Result<String, Unresolved> {
        if parent.kind == Kind::Trait {
            return self.trait_member(parent, name, link);
        }

        let kind = match link.filter {
            Filter::Kinds(&[Kind::Field]) => Kind::Field,
            Filter::Kinds(&[Kind::Variant]) => Kind::Variant,
            Filter::Kinds(&[Kind::Const]) => Kind::Const,
            Filter::Kinds(&[Kind::Fn, Kind::Method]) => Kind::Method,
            Filter::Any if link.suffix == Suffix::Parens => Kind::Method,
            Filter::Any => {
                let file = self.module_file(&parent.def)?;

                let members = file
                    .scope(&parent.def.inline)
                    .map(|scope| scope.members(&parent.def_name, name))
                    .unwrap_or_default();

                match members.as_slice() {
                    [Member::Method] => Kind::Method,
                    [Member::Const] => Kind::Const,
                    [Member::Type] => Kind::Type,
                    [Member::Variant] => Kind::Variant,
                    [Member::Field] => Kind::Field,
                    [] => {
                        return Err(other(format!(
                            "cannot tell what kind of member `{name}` is, add a disambiguator such as `method@` or `field@`"
                        )));
                    }
                    _ => {
                        return Err(other(format!(
                            "`{name}` is ambiguous, add a disambiguator such as `method@` or `field@`"
                        )));
                    }
                }
            }
            _ => return Err(other("unsupported member disambiguator")),
        };

        let anchor = match (parent.kind, kind) {
            (_, Kind::Method) => "method",
            (_, Kind::Const) => "associatedconstant",
            (_, Kind::Type) => "associatedtype",
            (Kind::Enum, Kind::Variant) => "variant",
            (Kind::Struct | Kind::Union, Kind::Field) => "structfield",
            _ => return Err(other("unsupported member for this kind of item")),
        };

        Ok(format!("{anchor}.{name}"))
    }

    /// The anchor of the member `name` of a trait. Its body decides whether a
    /// method is required or provided, so it is always looked into.
    fn trait_member(
        &mut self,
        parent: &Found,
        name: &str,
        link: &ItemLink<'_>,
    ) -> Result<String, Unresolved> {
        let wanted: &[Member] = match link.filter {
            Filter::Kinds(&[Kind::Fn, Kind::Method]) => &[Member::RequiredMethod, Member::Method],
            Filter::Kinds(&[Kind::Const]) => &[Member::Const],
            Filter::Kinds(&[Kind::Type]) | Filter::Kinds(TYPE_NS) => &[Member::Type],
            Filter::Any if link.suffix == Suffix::Parens => {
                &[Member::RequiredMethod, Member::Method]
            }
            Filter::Any => &[
                Member::RequiredMethod,
                Member::Method,
                Member::Const,
                Member::Type,
            ],
            _ => return Err(other("unsupported member disambiguator")),
        };

        let file = self.module_file(&parent.def)?;

        let members = file
            .scope(&parent.def.inline)
            .map(|scope| scope.members(&parent.def_name, name))
            .unwrap_or_default();

        let mut members = members.into_iter().filter(|m| wanted.contains(m));

        let anchor = match (members.next(), members.next()) {
            (Some(member), None) => match member {
                Member::RequiredMethod => "tymethod",
                Member::Method => "method",
                Member::Const => "associatedconstant",
                Member::Type => "associatedtype",
                Member::Variant | Member::Field => {
                    return Err(other("unsupported member for this kind of item"));
                }
            },
            (None, _) => {
                return Err(Unresolved::NotFound(format!(
                    "no matching member `{name}` declared in trait `{}`",
                    parent.def_name
                )));
            }
            (Some(_), Some(_)) => {
                return Err(other(format!(
                    "`{name}` is ambiguous, add a disambiguator such as `method@` or `type@`"
                )));
            }
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
