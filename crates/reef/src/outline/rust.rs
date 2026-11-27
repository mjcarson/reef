//! Rust's grammar: tree-sitter-rust's nodes as items, and the types, traits and modules that own
//! them
//!
//! - **Types and traits head their own groups**: `struct_item`, `enum_item`, `union_item` and
//!   `trait_item`, a trait owning the functions, types and constants in its body.
//! - **An impl block is owned by its type**: the `type` field's last name, its generics and
//!   references stripped, so `impl<T> Foo<T>` and `impl Display for &Foo` are both `Foo`'s. The
//!   block's own lines are an item of its own, and its members are owned by the type, with the
//!   trait it implements.
//! - **An inline module heads a group too**, owning its loose functions and items, while its
//!   types are named after it, such as `tests::Fixture`, and head groups of their own.
//! - **Blocks and token trees are never walked into**, so a function's body, a macro's rules
//!   and a macro call's arguments are their item's lines.
//! - **An item starts at what is written for it above it**: its attributes and doc comments,
//!   and the plain comments directly above it, but not a comment that trails the item before.

use super::{Found, Grammar};
use reef_api::outline::{ItemKind, Span};
use tree_sitter::{Node, Parser};

/// Rust, through tree-sitter-rust
pub struct Rust;

impl Grammar for Rust {
    /// The language's name, as an outline says it
    fn name(&self) -> &'static str {
        "rust"
    }

    /// Whether a path is a Rust source file
    ///
    /// # Arguments
    ///
    /// * `path` - The file's path
    fn reads(&self, path: &str) -> bool {
        path.ends_with(".rs")
    }

    /// Every item of one side of a Rust file, in the order they are written, or `None` if it
    /// holds a syntax error
    ///
    /// # Arguments
    ///
    /// * `text` - The side's text
    fn items(&self, text: &str) -> Option<Vec<Found>> {
        // a parser for Rust, which only fails on a grammar whose ABI the runtime can't read
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .ok()?;
        let tree = parser.parse(text, None)?;
        let root = tree.root_node();
        // an error anywhere, or a node tree-sitter had to make up, leaves the file unoutlined
        if root.has_error() {
            return None;
        }
        let mut found = Vec::new();
        let top = Scope {
            module: None,
            owner: None,
            via: None,
            depth: 0,
        };
        walk(root, text.as_bytes(), &top, &mut found);
        Some(found)
    }
}

/// Where in a file the items being walked are
struct Scope {
    /// The inline module they are in, after its own, such as `tests` or `a::b`
    module: Option<String>,
    /// The type, trait or module that owns them, if any
    owner: Option<String>,
    /// The trait the impl block they are in implements
    via: Option<String>,
    /// How many items they are nested in
    depth: u32,
}

impl Scope {
    /// A name as it is written in this scope's module: `Fixture` in `tests` is `tests::Fixture`
    ///
    /// # Arguments
    ///
    /// * `name` - The name
    fn qualify(&self, name: &str) -> String {
        match &self.module {
            Some(module) => format!("{module}::{name}"),
            None => name.to_string(),
        }
    }

    /// The scope of an item's members: owned by it, one deeper
    ///
    /// # Arguments
    ///
    /// * `owner` - The name of the item that owns them
    /// * `via` - The trait they implement, for an impl block's
    fn within(&self, owner: String, via: Option<String>) -> Scope {
        Scope {
            module: self.module.clone(),
            owner: Some(owner),
            via,
            depth: self.depth + 1,
        }
    }
}

/// A node's text, its runs of whitespace made single spaces
///
/// # Arguments
///
/// * `node` - The node
/// * `src` - The file's bytes
fn text(node: Node, src: &[u8]) -> String {
    let raw = node.utf8_text(src).unwrap_or_default();
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The text of a node's field, if it has one
///
/// # Arguments
///
/// * `node` - The node
/// * `field` - The field's name
/// * `src` - The file's bytes
fn field(node: Node, field: &str, src: &[u8]) -> Option<String> {
    node.child_by_field_name(field)
        .map(|child| text(child, src))
}

/// The name of the type an impl block is for: its last name, without generics or references,
/// or its text for a type with no one name, such as a tuple
///
/// # Arguments
///
/// * `node` - The `type` field's node
/// * `src` - The file's bytes
fn type_name(node: Node, src: &[u8]) -> String {
    // the field that holds the type's name within a wrapping type
    let inner = match node.kind() {
        "generic_type" | "reference_type" | "pointer_type" => node.child_by_field_name("type"),
        "scoped_type_identifier" | "scoped_identifier" => node.child_by_field_name("name"),
        _ => None,
    };
    match inner {
        Some(inner) => type_name(inner, src),
        None => text(node, src),
    }
}

/// The last line a node is on, 0-based: a node that ends at the start of a line, as a `///`
/// comment does with its newline, ends on the line before
///
/// # Arguments
///
/// * `node` - The node
fn last_row(node: Node) -> usize {
    let (start, end) = (node.start_position(), node.end_position());
    if end.column == 0 && end.row > start.row {
        end.row - 1
    } else {
        end.row
    }
}

/// Whether a node is a comment
///
/// # Arguments
///
/// * `node` - The node
fn is_comment(node: Node) -> bool {
    matches!(node.kind(), "line_comment" | "block_comment")
}

/// The first line of what is written for an item, 0-based: its attributes and doc comments
/// above it, and the plain comments directly above it
///
/// An attribute or an outer doc comment is the item's across blank lines too. A plain comment
/// is the item's only with no blank line under it, and not if it trails the line before it, as
/// a comment after the last item's semicolon does.
///
/// # Arguments
///
/// * `node` - The item's node
fn start_row(node: Node) -> usize {
    let mut start = node.start_position().row;
    let mut current = node;
    while let Some(above) = current.prev_named_sibling() {
        let doc = is_comment(above) && above.child_by_field_name("outer").is_some();
        let glued = if above.kind() == "attribute_item" || doc {
            // an attribute or an outer doc comment always belongs to what follows it
            true
        } else if is_comment(above) && above.child_by_field_name("inner").is_none() {
            // a plain comment, if nothing blank comes between, and it doesn't trail a line
            let touching = last_row(above) + 1 >= start;
            let trailing = above
                .prev_named_sibling()
                .is_some_and(|before| last_row(before) == above.start_position().row);
            touching && !trailing
        } else {
            false
        };
        if !glued {
            break;
        }
        start = above.start_position().row;
        current = above;
    }
    start
}

/// The span of an item's node, what is written above it included, as 1-based lines
///
/// # Arguments
///
/// * `node` - The item's node, or the statement that wraps it
fn span(node: Node) -> Span {
    Span {
        start: start_row(node) as u32 + 1,
        end: last_row(node) as u32 + 1,
    }
}

/// Adds an item found in a scope
///
/// # Arguments
///
/// * `found` - The items found so far
/// * `scope` - The scope it is in
/// * `kind` - Its kind
/// * `name` - Its name
/// * `owner` - Its owner, overriding the scope's
/// * `node` - The node its lines are
fn push(
    found: &mut Vec<Found>,
    scope: &Scope,
    kind: ItemKind,
    name: String,
    owner: Option<String>,
    node: Node,
) {
    found.push(Found {
        kind,
        name,
        owner,
        via: scope.via.clone(),
        depth: scope.depth,
        span: span(node),
    });
}

/// Walks the items of a file, a module's body, an impl block's or a trait's, in the order they
/// are written
///
/// # Arguments
///
/// * `container` - The node whose children are items
/// * `src` - The file's bytes
/// * `scope` - Where the items are
/// * `found` - The items found so far
fn walk(container: Node, src: &[u8], scope: &Scope, found: &mut Vec<Found>) {
    let mut cursor = container.walk();
    for node in container.named_children(&mut cursor) {
        // a macro called where an item goes may be wrapped as a statement, whose lines it takes
        let item = match node.kind() {
            "expression_statement" => match node.named_child(0) {
                Some(inner) if inner.kind() == "macro_invocation" => inner,
                _ => continue,
            },
            _ => node,
        };
        let name = |field_name: &str| field(item, field_name, src).unwrap_or_default();
        match item.kind() {
            // a type heads its own group, named after the module it is in
            "struct_item" | "enum_item" | "union_item" => {
                let kind = match item.kind() {
                    "struct_item" => ItemKind::Struct,
                    "enum_item" => ItemKind::Enum,
                    _ => ItemKind::Union,
                };
                push(found, scope, kind, scope.qualify(&name("name")), None, node);
            }
            // a trait heads its group, and owns what its body declares
            "trait_item" => {
                let trait_name = scope.qualify(&name("name"));
                push(
                    found,
                    scope,
                    ItemKind::Trait,
                    trait_name.clone(),
                    None,
                    node,
                );
                if let Some(body) = item.child_by_field_name("body") {
                    walk(body, src, &scope.within(trait_name, None), found);
                }
            }
            // an impl block is its type's, and so are its members, with the trait it implements
            "impl_item" => {
                let Some(ty) = item.child_by_field_name("type") else {
                    continue;
                };
                let owner = scope.qualify(&type_name(ty, src));
                let via = field(item, "trait", src);
                let block = scope.within(owner.clone(), via.clone());
                found.push(Found {
                    kind: ItemKind::Impl,
                    name: owner.clone(),
                    owner: Some(owner),
                    via,
                    depth: scope.depth,
                    span: span(node),
                });
                if let Some(body) = item.child_by_field_name("body") {
                    walk(body, src, &block, found);
                }
            }
            // an inline module heads its group, and owns its loose items
            "mod_item" => {
                // `mod foo;` is a line of its file, not an item
                let Some(body) = item.child_by_field_name("body") else {
                    continue;
                };
                let module = scope.qualify(&name("name"));
                push(found, scope, ItemKind::Mod, module.clone(), None, node);
                let inner = Scope {
                    module: Some(module.clone()),
                    ..scope.within(module, None)
                };
                walk(body, src, &inner, found);
            }
            // functions, methods and the rest go to whatever owns the scope
            "function_item" | "function_signature_item" => {
                push(
                    found,
                    scope,
                    ItemKind::Fn,
                    name("name"),
                    scope.owner.clone(),
                    node,
                );
            }
            "const_item" => {
                push(
                    found,
                    scope,
                    ItemKind::Const,
                    name("name"),
                    scope.owner.clone(),
                    node,
                );
            }
            "static_item" => {
                push(
                    found,
                    scope,
                    ItemKind::Static,
                    name("name"),
                    scope.owner.clone(),
                    node,
                );
            }
            "type_item" | "associated_type" => {
                push(
                    found,
                    scope,
                    ItemKind::Type,
                    name("name"),
                    scope.owner.clone(),
                    node,
                );
            }
            "macro_definition" => {
                push(
                    found,
                    scope,
                    ItemKind::Macro,
                    name("name"),
                    scope.owner.clone(),
                    node,
                );
            }
            "macro_invocation" => {
                let called = format!("{}!", name("macro"));
                push(
                    found,
                    scope,
                    ItemKind::Macro,
                    called,
                    scope.owner.clone(),
                    node,
                );
            }
            // imports, `extern crate`, inner attributes and the rest are the file's other lines
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Rust;
    use crate::outline::{Found, Grammar};
    use reef_api::outline::ItemKind;
    use std::path::Path;
    use std::process::Command;

    /// An item as the tests compare it: its kind, name, owner, trait, depth and lines
    type Shape = (
        ItemKind,
        String,
        Option<String>,
        Option<String>,
        u32,
        (u32, u32),
    );

    /// Each item of a text as its kind, name, owner, trait, depth and lines
    ///
    /// # Arguments
    ///
    /// * `text` - The Rust to read
    fn items(text: &str) -> Vec<Shape> {
        Rust.items(text)
            .expect("the text parses")
            .into_iter()
            .map(
                |Found {
                     kind,
                     name,
                     owner,
                     via,
                     depth,
                     span,
                 }| { (kind, name, owner, via, depth, (span.start, span.end)) },
            )
            .collect()
    }

    /// A string, owned
    ///
    /// # Arguments
    ///
    /// * `s` - The string
    fn s(s: &str) -> String {
        s.to_string()
    }

    /// Impl blocks are their type's, generics and references stripped, and their members carry
    /// the trait they implement
    #[test]
    fn impls_belong_to_their_type() {
        let text = "\
impl<T> Foo<T> {
    fn a() {}
}
impl fmt::Display for &Foo {
    fn fmt(&self) {}
}
impl crate::x::Bar {
    const N: u32 = 1;
    type Out = u8;
}
";
        assert_eq!(
            items(text),
            [
                (ItemKind::Impl, s("Foo"), Some(s("Foo")), None, 0, (1, 3)),
                (ItemKind::Fn, s("a"), Some(s("Foo")), None, 1, (2, 2)),
                (
                    ItemKind::Impl,
                    s("Foo"),
                    Some(s("Foo")),
                    Some(s("fmt::Display")),
                    0,
                    (4, 6)
                ),
                (
                    ItemKind::Fn,
                    s("fmt"),
                    Some(s("Foo")),
                    Some(s("fmt::Display")),
                    1,
                    (5, 5)
                ),
                (ItemKind::Impl, s("Bar"), Some(s("Bar")), None, 0, (7, 10)),
                (ItemKind::Const, s("N"), Some(s("Bar")), None, 1, (8, 8)),
                (ItemKind::Type, s("Out"), Some(s("Bar")), None, 1, (9, 9)),
            ]
        );
    }

    /// An inline module owns its loose items, names its types after it, and a trait owns what
    /// its body declares
    #[test]
    fn modules_and_traits_own_their_members() {
        let text = "\
#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture;

    impl Fixture {
        fn new() {}
    }

    fn works() {}
}

trait Draw {
    type Out;
    fn draw(&self);
    fn twice(&self) {}
}

macro_rules! twice {
    () => {};
}

thread_local! { static X: u8 = 0; }
";
        assert_eq!(
            items(text),
            [
                // the module's attribute is its own
                (ItemKind::Mod, s("tests"), None, None, 0, (1, 12)),
                (ItemKind::Struct, s("tests::Fixture"), None, None, 1, (5, 5)),
                (
                    ItemKind::Impl,
                    s("tests::Fixture"),
                    Some(s("tests::Fixture")),
                    None,
                    1,
                    (7, 9)
                ),
                (
                    ItemKind::Fn,
                    s("new"),
                    Some(s("tests::Fixture")),
                    None,
                    2,
                    (8, 8)
                ),
                (
                    ItemKind::Fn,
                    s("works"),
                    Some(s("tests")),
                    None,
                    1,
                    (11, 11)
                ),
                (ItemKind::Trait, s("Draw"), None, None, 0, (14, 18)),
                (ItemKind::Type, s("Out"), Some(s("Draw")), None, 1, (15, 15)),
                (ItemKind::Fn, s("draw"), Some(s("Draw")), None, 1, (16, 16)),
                (ItemKind::Fn, s("twice"), Some(s("Draw")), None, 1, (17, 17)),
                (ItemKind::Macro, s("twice"), None, None, 0, (20, 22)),
                (ItemKind::Macro, s("thread_local!"), None, None, 0, (24, 24)),
            ]
        );
    }

    /// Attributes and doc comments above an item are its own across a blank line, a plain
    /// comment only right above it, and a comment trailing the item before never
    #[test]
    fn what_is_written_above_an_item_is_its_own() {
        let text = "\
const A: u32 = 1; // A's own

// about B, with a blank line under it

// about B, right above it
/// B's doc
#[derive(Debug)]

#[allow(dead_code)]
struct B;
//! not an outer comment
fn c() {}
/** C's block doc */
fn d() {}
";
        assert_eq!(
            items(text),
            [
                (ItemKind::Const, s("A"), None, None, 0, (1, 1)),
                // from the comment right above it, through its doc and both attributes
                (ItemKind::Struct, s("B"), None, None, 0, (5, 10)),
                // an inner doc comment isn't what follows it's
                (ItemKind::Fn, s("c"), None, None, 0, (12, 12)),
                (ItemKind::Fn, s("d"), None, None, 0, (13, 14)),
            ]
        );
    }

    /// A `///` comment ends with its newline, so the item it heads starts on the comment's line,
    /// not the one after, and one that ends the file ends on its last line
    #[test]
    fn a_doc_comments_newline_isnt_a_line_of_its_own() {
        assert_eq!(
            items("/// one\n/// two\nfn f() {}\n"),
            [(ItemKind::Fn, s("f"), None, None, 0, (1, 3))]
        );
        assert_eq!(
            items("fn f() {\n}"),
            [(ItemKind::Fn, s("f"), None, None, 0, (1, 2))]
        );
    }

    /// Code with a syntax error isn't outlined, and let-chains and `use<..>` bounds parse
    #[test]
    fn errors_fail_and_new_syntax_parses() {
        assert!(Rust.items("fn a() {\n").is_none());
        assert!(Rust.items("struct S {\n    a: u32,,\n}\n").is_none());
        let modern = "\
fn f(x: Option<u8>) -> impl Sized + use<> {
    if let Some(a) = x && a > 1 {
        let Some(b) = x else { return };
    }
}
";
        assert_eq!(items(modern).len(), 1);
        // and a file that isn't Rust isn't read as Rust
        assert!(!Rust.reads("README.md"));
        assert!(Rust.reads("src/lib.rs"));
    }

    /// Every Rust file this repo tracks parses with no error, so the grammar keeps up with the
    /// Rust the repo is written in
    #[test]
    fn every_rust_file_here_parses() {
        // the repo's root, two levels above this crate
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let listed = Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["ls-files", "-z", "--", "*.rs"])
            .output()
            .expect("git lists the repo's files");
        // a checkout without git's history, as a packaged crate is, has nothing to check
        if !listed.status.success() {
            return;
        }
        let mut failed = Vec::new();
        let mut read = 0;
        for path in listed.stdout.split(|&b| b == 0).filter(|p| !p.is_empty()) {
            let path = String::from_utf8_lossy(path).to_string();
            let Ok(text) = std::fs::read_to_string(root.join(&path)) else {
                continue;
            };
            read += 1;
            if Rust.items(&text).is_none() {
                failed.push(path);
            }
        }
        assert!(read > 100, "only {read} Rust files read");
        assert!(failed.is_empty(), "these don't parse: {failed:?}");
    }
}
