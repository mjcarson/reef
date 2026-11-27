//! Each captured file's outline: the items of its code its change touched, parsed from both of
//! its sides
//!
//! The service keeps a file's hunks with three lines of context and never reads git again, so
//! only the capture, which still has both blobs, can tell which struct or function a changed line
//! is in. Each file a [`Grammar`] reads is parsed on both sides with tree-sitter, its items
//! matched across them, and the items its hunks changed sent with it as its [`Outline`].
//!
//! - **A grammar a language.** Rust's is the only one yet. Another is a tree-sitter grammar
//!   crate and a [`Grammar`] that says which of its nodes are items and which own others.
//! - **A file that doesn't parse has no outline.** tree-sitter recovers from errors, but its
//!   recovery can end an impl's body early and show its methods removed and added again, so an
//!   error on either side leaves the file to its plain diff. So does a binary or symlinked side,
//!   one over [`SIDE_MAX`] or not UTF-8, and more than [`ITEMS_MAX`] changed items.
//! - **Items are matched by what they are**: their owner, kind, name and the trait an impl
//!   implements, the nth of each on its side to the nth on the other. One on one side alone is
//!   new or removed, if it holds a changed line, and one on both with a changed line is changed.
//! - **Counted as the app draws them.** Which items changed is decided over every item found,
//!   then the kept ones' lines are counted again with [`attribute`] over the kept ones alone, the
//!   function the app draws them by, so the capture and the app agree.

mod rust;

use crate::git::Checkout;
use reef_api::outline::{Item, ItemChange, ItemKind, Outline, Owner, Span, attribute};
use reef_api::revision::{FileDiff, Hunk, LineKind};
use std::collections::HashMap;

/// The largest side of a file that is outlined, in bytes, since git's post-commit hook captures
/// while the commit waits
pub const SIDE_MAX: usize = 1 << 20;

/// The most changed items a file's outline keeps, beyond which the file has none, so a
/// revision's listing stays small
pub const ITEMS_MAX: usize = 500;

/// The mode git gives a symlink, whose blob is the path it points at
const SYMLINK_MODE: &str = "120000";

/// Every grammar files are outlined with
static GRAMMARS: &[&(dyn Grammar + Sync)] = &[&rust::Rust];

/// One item as a grammar finds it on one side, before the sides are matched
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// What kind of code it is
    pub kind: ItemKind,
    /// Its name: a type's after the inline modules it is in, an impl block's its type's
    pub name: String,
    /// The name of the type, trait or module it belongs to, if any
    pub owner: Option<String>,
    /// The trait the impl block it is in implements, as written
    pub via: Option<String>,
    /// How many items it is nested in
    pub depth: u32,
    /// The lines it spans, its attributes and the comments above it included
    pub span: Span,
}

/// A language's grammar: which files it reads, and the items it finds in one
pub trait Grammar {
    /// The language's name, as an outline says it, such as `rust`
    fn name(&self) -> &'static str;

    /// Whether the grammar reads a file at a path
    ///
    /// # Arguments
    ///
    /// * `path` - The file's path
    fn reads(&self, path: &str) -> bool;

    /// Every item of one side of a file, in the order they are written, or `None` if it doesn't
    /// parse cleanly
    ///
    /// # Arguments
    ///
    /// * `text` - The side's text
    fn items(&self, text: &str) -> Option<Vec<Found>>;
}

/// One side of a file, as it is read to be outlined
enum Blob {
    /// The file isn't on this side
    Absent,
    /// Its text
    Text(String),
    /// Too large, or not UTF-8, to outline
    Unfit,
}

/// Outlines each file of a capture that a grammar reads, from both of its sides
///
/// # Arguments
///
/// * `checkout` - The checkout the files' blobs are in
/// * `files` - The capture's files, each given its outline or none
pub fn outline_files(checkout: &Checkout, files: &mut [FileDiff]) -> anyhow::Result<()> {
    for file in files.iter_mut() {
        file.outline = outline_file(checkout, file)?;
    }
    Ok(())
}

/// The grammar that reads both of a file's sides, if one does
///
/// # Arguments
///
/// * `file` - The file
fn grammar_of(file: &FileDiff) -> Option<&'static (dyn Grammar + Sync)> {
    // each path the file has must be one the grammar reads, so a rename across languages has none
    let paths: Vec<&str> = [file.old_path.as_deref(), file.new_path.as_deref()]
        .into_iter()
        .flatten()
        .collect();
    GRAMMARS
        .iter()
        .copied()
        .find(|grammar| !paths.is_empty() && paths.iter().all(|path| grammar.reads(path)))
}

/// One side of a file, read from its blob
///
/// # Arguments
///
/// * `checkout` - The checkout the blob is in
/// * `blob` - The blob's id, if the file is on this side
fn read_side(checkout: &Checkout, blob: Option<&str>) -> anyhow::Result<Blob> {
    let Some(blob) = blob else {
        return Ok(Blob::Absent);
    };
    let bytes = checkout.blob(blob)?;
    // a side too large to parse while a commit waits, or not text, isn't outlined
    if bytes.len() > SIDE_MAX {
        return Ok(Blob::Unfit);
    }
    Ok(String::from_utf8(bytes).map_or(Blob::Unfit, Blob::Text))
}

/// One file's outline, if a grammar reads it and both its sides parse
///
/// # Arguments
///
/// * `checkout` - The checkout its blobs are in
/// * `file` - The file, its hunks and all
fn outline_file(checkout: &Checkout, file: &FileDiff) -> anyhow::Result<Option<Outline>> {
    // a binary file, or one whose lines didn't change, has nothing to outline
    if file.binary || file.hunks.is_empty() {
        return Ok(None);
    }
    let Some(grammar) = grammar_of(file) else {
        return Ok(None);
    };
    // a symlink's blob is the path it points at
    let modes = [file.old_mode.as_deref(), file.new_mode.as_deref()];
    if modes.contains(&Some(SYMLINK_MODE)) {
        return Ok(None);
    }
    // both sides as text, the side the file isn't on as none
    let text = |blob: Blob| match blob {
        Blob::Absent => Some(None),
        Blob::Text(text) => Some(Some(text)),
        Blob::Unfit => None,
    };
    let (Some(old), Some(new)) = (
        text(read_side(checkout, file.old_id.as_deref())?),
        text(read_side(checkout, file.new_id.as_deref())?),
    ) else {
        return Ok(None);
    };
    Ok(outline_texts(
        grammar,
        old.as_deref(),
        new.as_deref(),
        &file.hunks,
    ))
}

/// The key two items are matched across sides by: their owner, kind, name and trait
type Key<'a> = (Option<&'a str>, ItemKind, &'a str, Option<&'a str>);

/// An item's key, for matching it with its other side
///
/// # Arguments
///
/// * `found` - The item
fn key(found: &Found) -> Key<'_> {
    (
        found.owner.as_deref(),
        found.kind,
        found.name.as_str(),
        found.via.as_deref(),
    )
}

/// Each item's key with how many items before it on its side had the same, so the nth of a key
/// on one side is matched to the nth on the other
///
/// # Arguments
///
/// * `side` - One side's items, in the order they are written
fn keyed(side: &[Found]) -> Vec<(Key<'_>, usize)> {
    let mut seen: HashMap<Key<'_>, usize> = HashMap::new();
    side.iter()
        .map(|found| {
            // the count so far of this key, then one more
            let count = seen.entry(key(found)).or_default();
            let nth = *count;
            *count += 1;
            (key(found), nth)
        })
        .collect()
}

/// Every item of both sides, each matched with its other side if it has one: the worktree's in
/// the order they are written, then the base's that are gone
///
/// # Arguments
///
/// * `before` - The base's items
/// * `after` - The worktree's items
fn matched(before: &[Found], after: &[Found]) -> Vec<Item> {
    // the kinds of the types, traits and modules the file defines on either side
    let kinds: HashMap<&str, ItemKind> = before
        .iter()
        .chain(after)
        .filter(|found| found.owner.is_none() && found.kind.owns())
        .map(|found| (found.name.as_str(), found.kind))
        .collect();
    // an item as the outline holds it, from the sides it is on
    let item = |found: &Found, old: Option<Span>, new: Option<Span>| Item {
        kind: found.kind,
        name: found.name.clone(),
        owner: found.owner.as_ref().map(|name| Owner {
            name: name.clone(),
            kind: kinds.get(name.as_str()).copied(),
        }),
        via: found.via.clone(),
        change: match (old, new) {
            (None, _) => ItemChange::Added,
            (_, None) => ItemChange::Removed,
            _ => ItemChange::Changed,
        },
        depth: found.depth,
        old,
        new,
        added: 0,
        removed: 0,
    };
    // the base's items by their key and count
    let old_keys = keyed(before);
    let mut gone: HashMap<(Key<'_>, usize), usize> = old_keys
        .iter()
        .enumerate()
        .map(|(index, keyed)| (*keyed, index))
        .collect();
    // each worktree item, with its base side if it has one
    let mut items: Vec<Item> = Vec::with_capacity(after.len());
    for (found, keyed) in after.iter().zip(keyed(after)) {
        let old = gone.remove(&keyed).map(|index| before[index].span);
        items.push(item(found, old, Some(found.span)));
    }
    // then the base's items the worktree lacks, in the base's order
    for (found, keyed) in before.iter().zip(&old_keys) {
        if gone.contains_key(keyed) {
            items.push(item(found, Some(found.span), None));
        }
    }
    items
}

/// How many lines each item added and removed, and where its first changed line is in the diff
///
/// # Arguments
///
/// * `held` - Which item each line of each hunk goes to
/// * `hunks` - The file's hunks
/// * `count` - How many items there are
fn tally(held: &[Vec<Option<usize>>], hunks: &[Hunk], count: usize) -> Vec<(u32, u32, usize)> {
    // nothing counted, and a first change past every line
    let mut tallies = vec![(0, 0, usize::MAX); count];
    let mut position = 0;
    for (hunk, held) in hunks.iter().zip(held) {
        for (line, at) in hunk.lines.iter().zip(held) {
            // a changed line counts for its item, and marks the item's first change
            if let Some(index) = at
                && line.kind != LineKind::Context
            {
                let tally = &mut tallies[*index];
                match line.kind {
                    LineKind::Added => tally.0 += 1,
                    LineKind::Removed => tally.1 += 1,
                    LineKind::Context => {}
                }
                tally.2 = tally.2.min(position);
            }
            position += 1;
        }
    }
    tallies
}

/// A file's outline from its two sides' texts and its hunks, if each side it has parses
///
/// # Arguments
///
/// * `grammar` - The grammar both sides are read with
/// * `old` - The base's text, if the file is there
/// * `new` - The worktree's text, if the file is there
/// * `hunks` - The file's hunks between them
pub fn outline_texts(
    grammar: &dyn Grammar,
    old: Option<&str>,
    new: Option<&str>,
    hunks: &[Hunk],
) -> Option<Outline> {
    // each side's items, or no outline if a side doesn't parse
    let before = match old {
        Some(text) => grammar.items(text)?,
        None => Vec::new(),
    };
    let after = match new {
        Some(text) => grammar.items(text)?,
        None => Vec::new(),
    };
    // every item, matched across sides, and which of them hold a changed line
    let whole = matched(&before, &after);
    let tallies = tally(&attribute(&whole, hunks), hunks, whole.len());
    let changed: Vec<Item> = whole
        .into_iter()
        .zip(&tallies)
        .filter(|(_, (added, removed, _))| added + removed > 0)
        .map(|(item, _)| item)
        .collect();
    // too many to list, as a generated file's would be
    if changed.len() > ITEMS_MAX {
        return None;
    }
    // put in the order the diff reaches them
    let firsts = tally(&attribute(&changed, hunks), hunks, changed.len());
    let mut order: Vec<usize> = (0..changed.len()).collect();
    order.sort_by_key(|&index| firsts[index].2);
    let mut items: Vec<Item> = order
        .into_iter()
        .map(|index| changed[index].clone())
        .collect();
    // and counted as the app will attribute their lines, over the kept items alone
    let counts = tally(&attribute(&items, hunks), hunks, items.len());
    for (item, (added, removed, _)) in items.iter_mut().zip(counts) {
        item.added = added;
        item.removed = removed;
    }
    Some(Outline {
        language: grammar.name().to_string(),
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::{ITEMS_MAX, outline_texts, rust::Rust};
    use crate::git::line_hunks;
    use reef_api::outline::{ItemChange, ItemKind, Outline, attribute, groups};
    use reef_api::revision::{Hunk, LineKind};

    /// The outline of a Rust file between two texts, as a capture would send it
    ///
    /// # Arguments
    ///
    /// * `old` - The base's text, if the file is there
    /// * `new` - The worktree's text, if the file is there
    fn outline(old: Option<&str>, new: Option<&str>) -> (Option<Outline>, Vec<Hunk>) {
        let hunks = line_hunks(
            old.unwrap_or_default().as_bytes(),
            new.unwrap_or_default().as_bytes(),
        )
        .unwrap();
        (outline_texts(&Rust, old, new, &hunks), hunks)
    }

    /// Each item of an outline as its kind, label, owner, change and counts
    ///
    /// # Arguments
    ///
    /// * `outline` - The outline
    fn shape(outline: &Outline) -> Vec<(ItemKind, String, Option<String>, ItemChange, u32, u32)> {
        outline
            .items
            .iter()
            .map(|item| {
                (
                    item.kind,
                    item.label(),
                    item.owner.as_ref().map(|owner| owner.name.clone()),
                    item.change,
                    item.added,
                    item.removed,
                )
            })
            .collect()
    }

    /// The base the edits below start from: a struct, its impl, a trait impl and two loose fns
    const BASE: &str = r#"use std::fmt;

/// A scaler
pub struct Scaler {
    /// How many
    count: u32,
}

impl Scaler {
    /// Runs it
    pub fn run(&self) -> u32 {
        self.count + 1
    }
}

impl fmt::Display for Scaler {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.count)
    }
}

/// Says when
fn when() -> u32 {
    1
}

fn gone() {}
"#;

    /// The base edited: a field added, a method changed, one added, a fn removed and a struct
    /// added, with an import
    const EDITED: &str = r#"use std::fmt;
use std::io;

/// A scaler
pub struct Scaler {
    /// How many
    count: u32,
    /// How fast
    rate: u32,
}

impl Scaler {
    /// Runs it
    pub fn run(&self) -> u32 {
        self.count + self.rate
    }

    /// Stops it
    pub fn stop(&self) {}
}

impl fmt::Display for Scaler {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.count)
    }
}

/// Says when
fn when() -> u32 {
    1
}

/// A new one
struct Fresh;
"#;

    /// A field added, a method changed and one added sit under their struct, a fn removed and a
    /// struct added stand alone, and the import is no item's
    #[test]
    fn edits_outline_by_their_items() {
        let (outline, hunks) = outline(Some(BASE), Some(EDITED));
        let outline = outline.expect("both sides parse");
        assert_eq!(outline.language, "rust");
        let scaler = Some("Scaler".to_string());
        assert_eq!(
            shape(&outline),
            [
                (
                    ItemKind::Struct,
                    "struct Scaler".to_string(),
                    None,
                    ItemChange::Changed,
                    2,
                    0
                ),
                (
                    ItemKind::Fn,
                    "fn run".to_string(),
                    scaler.clone(),
                    ItemChange::Changed,
                    1,
                    1
                ),
                (
                    ItemKind::Fn,
                    "fn stop".to_string(),
                    scaler,
                    ItemChange::Added,
                    3,
                    0
                ),
                (
                    ItemKind::Fn,
                    "fn gone".to_string(),
                    None,
                    ItemChange::Removed,
                    0,
                    1
                ),
                (
                    ItemKind::Struct,
                    "struct Fresh".to_string(),
                    None,
                    ItemChange::Added,
                    2,
                    0
                ),
            ]
        );
        // the trait impl and `when` didn't change, so they aren't listed
        assert!(outline.items.iter().all(|item| item.name != "when"));
        // `run`'s owner is the struct the file defines
        assert_eq!(
            outline.items[1].owner.as_ref().and_then(|owner| owner.kind),
            Some(ItemKind::Struct)
        );
        // grouped: the struct with its methods, the removed fn, the new struct, then the import
        let (added, removed) =
            hunks
                .iter()
                .flat_map(|hunk| &hunk.lines)
                .fold((0, 0), |(a, r), line| match line.kind {
                    LineKind::Added => (a + 1, r),
                    LineKind::Removed => (a, r + 1),
                    LineKind::Context => (a, r),
                });
        let groups = groups(&outline, added, removed);
        let labels: Vec<(String, ItemChange)> = groups
            .iter()
            .map(|group| (group.label(&outline), group.change))
            .collect();
        assert_eq!(
            labels,
            [
                ("struct Scaler".to_string(), ItemChange::Changed),
                ("fn gone".to_string(), ItemChange::Removed),
                ("struct Fresh".to_string(), ItemChange::Added),
                ("Other lines".to_string(), ItemChange::Changed),
            ]
        );
        assert_eq!((groups[3].added, groups[3].removed), (1, 0));
        // and the capture's counts are what the app attributes over the same items
        let held = attribute(&outline.items, &hunks);
        let mut counted = vec![(0, 0); outline.items.len()];
        for (hunk, held) in hunks.iter().zip(&held) {
            for (line, at) in hunk.lines.iter().zip(held) {
                match (at, line.kind) {
                    (Some(k), LineKind::Added) => counted[*k].0 += 1,
                    (Some(k), LineKind::Removed) => counted[*k].1 += 1,
                    _ => {}
                }
            }
        }
        let listed: Vec<(u32, u32)> = outline.items.iter().map(|i| (i.added, i.removed)).collect();
        assert_eq!(counted, listed);
    }

    /// A file added outlines its items as new, and one deleted as removed
    #[test]
    fn added_and_deleted_files_outline_whole() {
        let (added, _) = outline(None, Some("fn main() {}\n"));
        assert_eq!(
            shape(&added.unwrap()),
            [(
                ItemKind::Fn,
                "fn main".to_string(),
                None,
                ItemChange::Added,
                1,
                0
            )]
        );
        let (deleted, _) = outline(Some("struct A;\nstruct B;\n"), None);
        assert_eq!(
            shape(&deleted.unwrap()),
            [
                (
                    ItemKind::Struct,
                    "struct A".to_string(),
                    None,
                    ItemChange::Removed,
                    0,
                    1
                ),
                (
                    ItemKind::Struct,
                    "struct B".to_string(),
                    None,
                    ItemChange::Removed,
                    0,
                    1
                ),
            ]
        );
    }

    /// A side that doesn't parse, on either side, leaves the file with no outline
    #[test]
    fn a_side_that_doesnt_parse_has_no_outline() {
        // a brace left open in the worktree
        let (open, _) = outline(Some("fn a() {}\n"), Some("fn a() {\n"));
        assert!(open.is_none());
        // and an error in the base alone
        let (base, _) = outline(Some("fn a( {}\n"), Some("fn a() {}\n"));
        assert!(base.is_none());
    }

    /// From<A> and From<B> for one type are two impls, each method matched with its own
    #[test]
    fn impls_of_one_trait_with_other_arguments_stay_apart() {
        let old = "struct E;\n\nimpl From<A> for E {\n    fn from(a: A) -> E {\n        E\n    }\n}\n\nimpl From<B> for E {\n    fn from(b: B) -> E {\n        E\n    }\n}\n";
        let new = old.replace(
            "fn from(b: B) -> E {\n        E",
            "fn from(b: B) -> E {\n        let _ = b;\n        E",
        );
        let (outline, _) = outline(Some(old), Some(&new));
        let outline = outline.unwrap();
        // only B's `from` changed, and it is matched, not new
        assert_eq!(outline.items.len(), 1);
        let item = &outline.items[0];
        assert_eq!(
            (item.change, item.via.as_deref()),
            (ItemChange::Changed, Some("From<B>"))
        );
        assert_eq!(item.label(), "fn from");
    }

    /// An item found on a side alone with no changed line, as a renamed type's methods are, is
    /// left out
    #[test]
    fn a_renamed_types_methods_arent_new() {
        let old = "struct A;\n\nimpl A {\n    fn one() {}\n\n    fn two() {}\n}\n";
        let new = old
            .replace("struct A;", "struct B;")
            .replace("impl A {", "impl B {");
        let (outline, _) = outline(Some(old), Some(&new));
        let labels: Vec<(String, ItemChange)> = outline
            .unwrap()
            .items
            .iter()
            .map(|item| (item.label(), item.change))
            .collect();
        // the structs and the impls' heads changed, in the diff's order, the methods' lines didn't
        assert_eq!(
            labels,
            [
                ("struct A".to_string(), ItemChange::Removed),
                ("struct B".to_string(), ItemChange::Added),
                ("impl A".to_string(), ItemChange::Removed),
                ("impl B".to_string(), ItemChange::Added),
            ]
        );
    }

    /// A file with more changed items than an outline keeps has none
    #[test]
    fn too_many_items_leave_no_outline() {
        let many: String = (0..=ITEMS_MAX)
            .map(|n| format!("fn f{n}() {{}}\n"))
            .collect();
        let (outline, _) = outline(None, Some(&many));
        assert!(outline.is_none());
    }
}
