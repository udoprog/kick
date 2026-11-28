use super::*;

fn file(source: &str) -> File {
    parse(source.to_owned()).unwrap()
}

fn names(scope: Scope<'_>) -> Vec<(Syntax, String, bool)> {
    scope
        .items()
        .filter_map(|item| Some((item.kind(), item.name()?.to_owned(), item.is_public())))
        .collect()
}

#[test]
fn item_kinds() {
    let file = file(
        r#"
pub struct A;
pub(crate) enum B { X }
pub union C { a: u32 }
pub trait D {}
pub type E = u32;
pub const F: u32 = 1;
pub static mut G: u32 = 0;
pub mod h;
pub const unsafe extern "C" fn i() {}
pub async fn j() {}
pub unsafe auto trait K {}
fn private() {}
const _: () = ();
extern crate alloc;
macro_rules! m { () => {} }
"#,
    );

    let root = file.scope::<&str>(&[]).unwrap();

    assert_eq!(
        names(root),
        [
            (Struct, "A".to_owned(), true),
            (Enum, "B".to_owned(), false),
            (Union, "C".to_owned(), true),
            (Trait, "D".to_owned(), true),
            (TypeAlias, "E".to_owned(), true),
            (Const, "F".to_owned(), true),
            (Static, "G".to_owned(), true),
            (Mod, "h".to_owned(), true),
            (Fn, "i".to_owned(), true),
            (Fn, "j".to_owned(), true),
            (Trait, "K".to_owned(), true),
            (Fn, "private".to_owned(), false),
            (Const, "_".to_owned(), false),
            (MacroRules, "m".to_owned(), false),
        ]
    );
}

#[test]
fn nested_inline_modules() {
    let file = file(
        "
pub mod a {
    pub fn outer() {}

    pub mod b {
        pub struct Inner;

        mod c {
            pub fn deepest() {}
        }
    }
}

pub fn after() {}
",
    );

    let b = file.scope(&["a", "b"]).unwrap();
    assert_eq!(names(b)[0], (Struct, "Inner".to_owned(), true));

    let c = file.scope(&["a", "b", "c"]).unwrap();
    assert_eq!(names(c), [(Fn, "deepest".to_owned(), true)]);

    let root = file.scope::<&str>(&[]).unwrap();
    assert_eq!(
        names(root),
        [(Mod, "a".to_owned(), true), (Fn, "after".to_owned(), true)]
    );

    assert!(file.scope(&["a", "missing"]).is_none());
}

#[test]
fn impl_attribution() {
    let file = file(
        "
pub struct Foo { pub field: HashMap<u32, String>, private: u32, pub r#type: u32 }
pub struct Bar;
pub enum Color { Red, Green = 1 << 2, Blue(u32, u32), Named { a: u32 } }

impl<T: Fn() -> u32, const N: usize> Foo {
    pub fn method(&self) {}
    fn hidden(&self) {}
    pub const MAX: u32 = 1;
    pub fn field(&self) {}
}

impl<'a> fmt::Display for &'a Bar<'a> {
    fn fmt(&self) {}
    type Output = ();
}

impl crate::Color where Self: Sized {
    pub fn paint() {}
}
",
    );

    let root = file.scope::<&str>(&[]).unwrap();

    assert_eq!(root.members("Foo", "method"), [Member::Method]);
    assert_eq!(root.members("Foo", "MAX"), [Member::Const]);
    assert_eq!(root.members("Foo", "hidden"), []);
    assert_eq!(root.members("Foo", "private"), []);
    assert_eq!(root.members("Foo", "type"), [Member::Field]);
    assert_eq!(
        root.members("Foo", "field"),
        [Member::Field, Member::Method]
    );
    assert_eq!(root.members("Bar", "method"), []);
    assert_eq!(root.members("Bar", "fmt"), [Member::Method]);
    assert_eq!(root.members("Bar", "Output"), [Member::Type]);
    assert_eq!(root.members("Color", "paint"), [Member::Method]);

    for variant in ["Red", "Green", "Blue", "Named"] {
        assert_eq!(
            root.members("Color", variant),
            [Member::Variant],
            "{variant}"
        );
    }

    assert_eq!(root.members("Color", "a"), []);
}

#[test]
fn trait_members() {
    let file = file(
        "
pub trait Run<T>: Sized where T: Fn() -> [u8; 3] {
    type Output;
    const LIMIT: usize = 1;
    const REQUIRED: u32;
    fn run(&self) -> Self::Output;
    fn array() -> [u8; 3];
    fn provided(&self) -> u32 { 1 }
    #[doc(hidden)]
    unsafe fn both(&self) where Self: Sized {}
    fn r#async();
}

pub trait Alias = Run<u32>;

impl dyn Run<u32> {
    pub fn inherent(&self) {}
    fn hidden(&self) {}
}

pub fn after() {}
",
    );

    let root = file.scope::<&str>(&[]).unwrap();

    assert_eq!(
        names(root),
        [
            (Trait, "Run".to_owned(), true),
            (Trait, "Alias".to_owned(), true),
            (Fn, "after".to_owned(), true),
        ]
    );

    assert_eq!(root.members("Run", "Output"), [Member::Type]);
    assert_eq!(root.members("Run", "LIMIT"), [Member::Const]);
    assert_eq!(root.members("Run", "REQUIRED"), [Member::Const]);
    assert_eq!(root.members("Run", "run"), [Member::RequiredMethod]);
    assert_eq!(root.members("Run", "array"), [Member::RequiredMethod]);
    assert_eq!(root.members("Run", "provided"), [Member::Method]);
    assert_eq!(root.members("Run", "both"), [Member::Method]);
    assert_eq!(root.members("Run", "async"), [Member::RequiredMethod]);
    assert_eq!(root.members("Run", "inherent"), [Member::Method]);
    assert_eq!(root.members("Run", "hidden"), []);
    assert_eq!(root.members("Run", "missing"), []);
    assert_eq!(root.members("Alias", "run"), []);
}

#[test]
fn attributes() {
    let file = file(
        r#"
#![doc = "crate"]

#[cfg(feature = "x")]
#[path = "other/file.rs"]
pub mod a;

/// Docs.
#[macro_export]
// A comment.
macro_rules! exported { () => {} }

#[macro_exports]
macro_rules! not_exported { () => {} }

#[doc(hidden)]
pub fn hidden() {}
"#,
    );

    let root = file.scope::<&str>(&[]).unwrap();
    let items = root.items().collect::<Vec<_>>();
    let [_, a, exported, not_exported, hidden] = items.as_slice() else {
        panic!("{:?}", names(root));
    };

    assert_eq!(a.name(), Some("a"));
    assert_eq!(a.path_attribute(), Some("other/file.rs"));
    assert!(a.block().is_none());
    assert!(exported.is_macro_export());
    assert!(!not_exported.is_macro_export());
    assert_eq!(hidden.attribute("doc"), Some("(hidden)"));
}

#[test]
fn literals_and_comments() {
    let file = file(
        r####"
pub fn a() {
    let s = "} pub fn fake() {";
    let c = '}';
    let b = b'{';
    let r = r#"} "# pub fn fake2() {"#;
    let r2 = r##"}"#"##;
    let l: &'static str = br"}";
    // }
    /* } /* nested } */ pub fn fake3() {} */
}

/* pub fn commented() {} */
// pub fn commented2() {}
pub fn b<'a>(x: &'a str) -> &'a str { x }

pub const C: &str = "pub fn fake4() {}";
pub struct D;
"####,
    );

    let root = file.scope::<&str>(&[]).unwrap();
    assert_eq!(
        names(root),
        [
            (Fn, "a".to_owned(), true),
            (Fn, "b".to_owned(), true),
            (Const, "C".to_owned(), true),
            (Struct, "D".to_owned(), true),
        ]
    );
}

#[test]
fn use_trees() {
    let file = file(
        "
pub use self::a::B;
pub use crate::c::D as E;
pub use f::*;
pub use g::{H, I};
use j::K as _;
",
    );

    let root = file.scope::<&str>(&[]).unwrap();
    let trees = root
        .items()
        .filter_map(|item| item.use_tree())
        .collect::<Vec<_>>();

    assert_eq!(
        trees.iter().map(UseTree::name).collect::<Vec<_>>(),
        [Some("B"), Some("E"), None, None, None]
    );

    assert_eq!(trees[1].path, ["crate", "c", "D"]);
    assert_eq!(trees[1].rename, Some("E"));
}

#[test]
fn recovers_from_unknown_input() {
    let file = file(
        "
some_macro! { pub fn inside() {} }
other_macro!(x);
) ] } garbage tokens ;
#[attr] ;
impl {
pub struct After { a: u32
",
    );

    let root = file.scope::<&str>(&[]).unwrap();
    assert!(names(root).is_empty(), "{:?}", names(root));

    let file = self::file("pub struct A { pub a: Vec<u32, }\npub fn b() {}\n");
    let root = file.scope::<&str>(&[]).unwrap();
    assert_eq!(names(root)[0].1, "A");

    for source in [
        "", "'", "r#\"", "/*", "\"", "pub", "pub mod", "impl<", "#", "b'",
    ] {
        parse(source.to_owned()).unwrap();
    }
}
