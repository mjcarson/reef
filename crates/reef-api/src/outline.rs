//! A file's outline: the items of its code a revision changed, and which lines are each one's
//!
//! `reef node capture` parses both sides of each file it has a grammar for, finds the items a
//! change touched, and sends them with the file as its [`Outline`]: what kind of item each is,
//! its name, the type or module it belongs to, the lines it spans on each side, whether it is
//! new, changed or removed, and how many lines it added and removed. The service keeps the
//! outline as it keeps the file's hunks, and the app draws a file's changes item by item from it.
//!
//! - **A line goes to its deepest item** ([`attribute`]): a method before its impl, an impl
//!   before its module. A blank line added or removed goes to the change beside it, so adding a
//!   method doesn't count a line against its impl.
//! - **The capture and the app attribute lines alike.** The capture counts each item's lines with
//!   [`attribute`] over the items it keeps, and the app draws them with the same, so the two
//!   agree.
//! - **Items are grouped by owner** ([`groups`]): a struct's definition with its impl blocks'
//!   methods, a trait's with its own, an inline module's with its loose functions. Every other
//!   item is a group alone, and the lines no item holds are the file's other lines, last.

use serde::{Deserialize, Serialize};

use crate::revision::{Hunk, Line, LineKind, Side};

/// The key of the group of a file's lines that no item holds, such as its imports
pub const OTHER_KEY: &str = "other";

/// What kind of code an item is
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemKind {
    /// A struct's definition
    Struct,
    /// An enum's definition
    Enum,
    /// A union's definition
    Union,
    /// A trait's definition
    Trait,
    /// An impl block's own lines: its head, its closing brace and any line no member holds
    Impl,
    /// A function, or a method of an impl or a trait
    Fn,
    /// A constant
    Const,
    /// A static
    Static,
    /// A type alias, or an associated type
    Type,
    /// A macro's definition, or a macro called where an item goes
    Macro,
    /// An inline module's own lines
    Mod,
    /// A kind a newer grammar names, which this build doesn't know
    #[serde(other)]
    Other,
}

impl ItemKind {
    /// The kind as a file's outline labels it
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::outline::ItemKind;
    ///
    /// assert_eq!(ItemKind::Struct.label(), "struct");
    /// assert_eq!(ItemKind::Fn.label(), "fn");
    /// ```
    pub fn label(self) -> &'static str {
        match self {
            ItemKind::Struct => "struct",
            ItemKind::Enum => "enum",
            ItemKind::Union => "union",
            ItemKind::Trait => "trait",
            ItemKind::Impl => "impl",
            ItemKind::Fn => "fn",
            ItemKind::Const => "const",
            ItemKind::Static => "static",
            ItemKind::Type => "type",
            ItemKind::Macro => "macro",
            ItemKind::Mod => "mod",
            ItemKind::Other => "item",
        }
    }

    /// Whether items of this kind head a group of their own members: a type, a trait or a
    /// module
    pub fn owns(self) -> bool {
        matches!(
            self,
            ItemKind::Struct | ItemKind::Enum | ItemKind::Union | ItemKind::Trait | ItemKind::Mod
        )
    }
}

/// What a revision did to an item
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ItemChange {
    /// The item is new in the worktree
    Added,
    /// The item is gone from the worktree
    Removed,
    /// The item is on both sides, with lines changed, or a change this build doesn't know
    #[serde(other)]
    Changed,
}

impl ItemChange {
    /// The change as a file's outline tags it
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::outline::ItemChange;
    ///
    /// assert_eq!(ItemChange::Added.label(), "new");
    /// assert_eq!(ItemChange::Removed.label(), "removed");
    /// ```
    pub fn label(self) -> &'static str {
        match self {
            ItemChange::Added => "new",
            ItemChange::Changed => "changed",
            ItemChange::Removed => "removed",
        }
    }
}

/// The lines an item spans on one side, 1-based and inclusive
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    /// Its first line, its attributes and the comments above it included
    pub start: u32,
    /// Its last line
    pub end: u32,
}

impl Span {
    /// Whether a line is within the span
    ///
    /// # Arguments
    ///
    /// * `line` - The line's 1-based number on the span's side
    pub fn holds(&self, line: u32) -> bool {
        self.start <= line && line <= self.end
    }
}

/// The type, trait or inline module an item belongs to
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Owner {
    /// Its name, after the inline modules it is in, such as `Scaler` or `tests::Fixture`
    pub name: String,
    /// Its kind, if the file defines it on either side
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ItemKind>,
}

/// One item of a file that a revision changed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    /// What kind of code it is
    pub kind: ItemKind,
    /// Its name: a type's after the inline modules it is in, an impl block's its type's, a
    /// macro call's with its `!`
    pub name: String,
    /// The type, trait or module it belongs to, if any
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<Owner>,
    /// The trait an impl block implements, for the block and its members, as written
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// Whether it is new, changed or gone
    pub change: ItemChange,
    /// How many items it is nested in, which breaks a tie between items on the same lines
    #[serde(default, skip_serializing_if = "is_zero")]
    pub depth: u32,
    /// The lines it spans in the base, if it is there
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub old: Option<Span>,
    /// The lines it spans in the worktree, if it is there
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub new: Option<Span>,
    /// How many of its lines were added
    #[serde(default)]
    pub added: u32,
    /// How many of its lines were removed
    #[serde(default)]
    pub removed: u32,
}

impl Item {
    /// The span it has on a side, if it is there
    ///
    /// # Arguments
    ///
    /// * `side` - The side
    pub fn span(&self, side: Side) -> Option<Span> {
        match side {
            Side::Old => self.old,
            Side::New => self.new,
        }
    }

    /// The trait it implements as a label shows it: the trait's last path segment with its
    /// arguments, such as `Display` for `fmt::Display`
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_api::outline::{Item, ItemChange, ItemKind};
    ///
    /// let mut item = Item {
    ///     kind: ItemKind::Fn,
    ///     name: "from".to_string(),
    ///     owner: None,
    ///     via: Some("std::convert::From<io::Error>".to_string()),
    ///     change: ItemChange::Added,
    ///     depth: 1,
    ///     old: None,
    ///     new: None,
    ///     added: 3,
    ///     removed: 0,
    /// };
    /// assert_eq!(item.via_label(), Some("From<io::Error>"));
    /// item.via = Some("fmt::Display".to_string());
    /// assert_eq!(item.via_label(), Some("Display"));
    /// ```
    pub fn via_label(&self) -> Option<&str> {
        let via = self.via.as_deref()?;
        // the path ends where its arguments start
        let path_end = via.find('<').unwrap_or(via.len());
        // the last segment starts after the path's last separator
        let start = via[..path_end].rfind("::").map_or(0, |at| at + 2);
        Some(&via[start..])
    }

    /// The item as a file's outline names it, such as `fn new`, `impl Display for Scaler` or
    /// `macro thread_local!`
    pub fn label(&self) -> String {
        match (self.kind, self.via_label()) {
            // an impl block is named by what it implements for its type
            (ItemKind::Impl, Some(via)) => format!("impl {via} for {}", self.name),
            (ItemKind::Impl, None) => format!("impl {}", self.name),
            (kind, _) => format!("{} {}", kind.label(), self.name),
        }
    }
}

/// The items of a file a revision changed, in the order the diff reaches them
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Outline {
    /// The grammar the file was parsed with, such as `rust`
    pub language: String,
    /// The changed items, in the order of each one's first changed line in the diff
    pub items: Vec<Item>,
}

/// One group of a file's outline: a type, trait or module with its members, an item alone, or
/// the lines no item holds
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Group {
    /// Its key within the file, which stays the same across a revision's reads
    pub key: String,
    /// The kind of its owner or its item, or `None` for the file's other lines
    pub kind: Option<ItemKind>,
    /// Its name: its owner's, its item's, or "Other lines"
    pub name: String,
    /// Whether it is new, changed or gone as a whole
    pub change: ItemChange,
    /// The index of its owner's own item, or its item's, if that changed
    pub head: Option<usize>,
    /// The indices of its other items, in the outline's order
    pub members: Vec<usize>,
    /// How many of its lines were added
    pub added: u32,
    /// How many of its lines were removed
    pub removed: u32,
}

impl Group {
    /// Whether this is the group of the file's lines that no item holds
    pub fn is_other(&self) -> bool {
        self.key == OTHER_KEY
    }

    /// The group as a file's outline names it, such as `struct Scaler` or `fn when`
    pub fn label(&self, outline: &Outline) -> String {
        match (self.head, self.members.is_empty(), self.kind) {
            // an item alone is named as the item is
            (Some(head), true, _) => outline.items[head].label(),
            // a type's group by its kind and name, an impl's of a type defined elsewhere as impl
            (_, _, Some(kind)) => format!("{} {}", kind.label(), self.name),
            (_, _, None) => self.name.clone(),
        }
    }
}

/// One run of a hunk's lines that a group's item holds: the hunk, and its lines from `start` up
/// to `end`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Segment {
    /// The index of the hunk the run is in
    pub hunk: usize,
    /// The index of its first line in the hunk
    pub start: usize,
    /// The index after its last line in the hunk
    pub end: usize,
}

/// Whether a count is zero, for serde to leave it out
///
/// # Arguments
///
/// * `n` - The count
fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// The deepest item that holds a line on a side, the first in the outline among equals
///
/// # Arguments
///
/// * `items` - The items to look through
/// * `side` - The side the line is numbered on
/// * `line` - Its number there
fn deepest(items: &[Item], side: Side, line: u32) -> Option<usize> {
    let mut found: Option<usize> = None;
    for (index, item) in items.iter().enumerate() {
        // only an item whose span on this side holds the line
        if !item.span(side).is_some_and(|span| span.holds(line)) {
            continue;
        }
        // a deeper item wins, and an equal one leaves the first found
        if found.is_none_or(|best| item.depth > items[best].depth) {
            found = Some(index);
        }
    }
    found
}

/// The item one line of a hunk falls in: a removed line by its number in the base, an added
/// one by its number in the worktree, and a context line by the worktree's, else the base's
///
/// # Arguments
///
/// * `items` - The items to look through
/// * `line` - The line
fn holder(items: &[Item], line: &Line) -> Option<usize> {
    // the deepest item on one side, if the line is numbered there
    let on = |side: Side, number: Option<u32>| number.and_then(|n| deepest(items, side, n));
    match line.kind {
        LineKind::Removed => on(Side::Old, line.old),
        LineKind::Added => on(Side::New, line.new),
        LineKind::Context => on(Side::New, line.new).or_else(|| on(Side::Old, line.old)),
    }
}

/// Whether a line was added or removed, not kept as context
///
/// # Arguments
///
/// * `line` - The line
fn changed(line: &Line) -> bool {
    line.kind != LineKind::Context
}

/// Whether a changed line holds nothing but whitespace
///
/// # Arguments
///
/// * `line` - The line
fn blank_change(line: &Line) -> bool {
    changed(line) && line.text.trim().is_empty()
}

/// Which item each line of each hunk is attributed to, if any
///
/// A line goes to the deepest item that holds it, the first among equals. A blank line added or
/// removed goes instead where the nearest changed line that isn't blank in the same run of
/// changes goes, the one before it first, so a method added with a blank line before it counts
/// the blank line as the method's, not its impl's.
///
/// # Arguments
///
/// * `items` - The items lines can go to
/// * `hunks` - The file's hunks
///
/// # Examples
///
/// ```
/// use reef_api::outline::{Item, ItemChange, ItemKind, Span, attribute};
/// use reef_api::revision::{Hunk, Line, LineKind};
///
/// let added = |new: u32, text: &str| Line {
///     kind: LineKind::Added,
///     old: None,
///     new: Some(new),
///     text: text.to_string(),
///     no_newline: false,
/// };
/// let item = Item {
///     kind: ItemKind::Fn,
///     name: "main".to_string(),
///     owner: None,
///     via: None,
///     change: ItemChange::Added,
///     depth: 0,
///     old: None,
///     new: Some(Span { start: 2, end: 3 }),
///     added: 0,
///     removed: 0,
/// };
/// let hunk = Hunk {
///     old_start: 0,
///     old_lines: 0,
///     new_start: 1,
///     new_lines: 3,
///     lines: vec![added(1, "use std::io;"), added(2, "fn main() {"), added(3, "}")],
/// };
/// assert_eq!(attribute(&[item], &[hunk]), [vec![None, Some(0), Some(0)]]);
/// ```
pub fn attribute(items: &[Item], hunks: &[Hunk]) -> Vec<Vec<Option<usize>>> {
    hunks
        .iter()
        .map(|hunk| {
            // each line's deepest item
            let mut held: Vec<Option<usize>> =
                hunk.lines.iter().map(|line| holder(items, line)).collect();
            // each blank change takes the item of the nearest change beside it in its run
            let lines = &hunk.lines;
            for at in 0..lines.len() {
                if !blank_change(&lines[at]) {
                    continue;
                }
                // the nearest change before it in the run that isn't blank, else after it
                let before = (0..at)
                    .rev()
                    .take_while(|&k| changed(&lines[k]))
                    .find(|&k| !blank_change(&lines[k]));
                let after = (at + 1..lines.len())
                    .take_while(|&k| changed(&lines[k]))
                    .find(|&k| !blank_change(&lines[k]));
                if let Some(beside) = before.or(after) {
                    held[at] = held[beside];
                }
            }
            held
        })
        .collect()
}

/// The item a line on a side falls in, as [`attribute`] would put it, for a comment's line to be
/// found in a file's outline
///
/// A blank line is put where its own number falls, since the lines beside it aren't known here.
///
/// # Arguments
///
/// * `outline` - The file's outline
/// * `side` - The side the line is numbered on
/// * `line` - Its number there
pub fn attribute_one(outline: &Outline, side: Side, line: u32) -> Option<usize> {
    deepest(&outline.items, side, line)
}

/// How many of a file's added and removed lines no item of its outline holds
///
/// # Arguments
///
/// * `outline` - The file's outline
/// * `added` - How many lines the file added
/// * `removed` - How many lines it removed
pub fn other_counts(outline: &Outline, added: u32, removed: u32) -> (u32, u32) {
    // every changed line is counted against one item, or against none
    let (held_added, held_removed) = outline
        .items
        .iter()
        .fold((0, 0), |(a, r), item| (a + item.added, r + item.removed));
    (
        added.saturating_sub(held_added),
        removed.saturating_sub(held_removed),
    )
}

/// The key of the group an item belongs to, and whether it heads that group
///
/// # Arguments
///
/// * `items` - The outline's items
/// * `index` - The item's index
fn group_of(items: &[Item], index: usize) -> (String, bool) {
    let item = &items[index];
    match &item.owner {
        // a member goes with its owner
        Some(owner) => (format!("owner:{}", owner.name), false),
        // a type, trait or module heads its own
        None if item.kind.owns() => (format!("owner:{}", item.name), true),
        // and anything else is alone, the nth of its kind and name
        None => {
            let nth = items[..index]
                .iter()
                .filter(|other| {
                    other.owner.is_none() && other.kind == item.kind && other.name == item.name
                })
                .count();
            (
                format!("item:{}:{}:{nth}", item.kind.label(), item.name),
                true,
            )
        }
    }
}

/// A file's outline as groups, in the order the diff reaches each one, then the file's other
/// lines if any changed
///
/// A type's group is tagged as its definition is, if that changed. Otherwise it is new if each
/// member is, removed if each member is, and changed if not.
///
/// # Arguments
///
/// * `outline` - The file's outline
/// * `added` - How many lines the file added
/// * `removed` - How many lines it removed
pub fn groups(outline: &Outline, added: u32, removed: u32) -> Vec<Group> {
    let items = &outline.items;
    let mut groups: Vec<Group> = Vec::new();
    for index in 0..items.len() {
        let item = &items[index];
        let (key, heads) = group_of(items, index);
        // the group this item is the first of, made from what the item says of its owner
        let at = match groups.iter().position(|group| group.key == key) {
            Some(at) => at,
            None => {
                let (kind, name) = match (&item.owner, heads) {
                    (Some(owner), false) => (owner.kind.unwrap_or(ItemKind::Impl), &owner.name),
                    _ => (item.kind, &item.name),
                };
                groups.push(Group {
                    key,
                    kind: Some(kind),
                    name: name.clone(),
                    change: item.change,
                    head: None,
                    members: Vec::new(),
                    added: 0,
                    removed: 0,
                });
                groups.len() - 1
            }
        };
        let group = &mut groups[at];
        // the item heads its group, or is one of its members
        if heads {
            group.head = Some(index);
        } else {
            group.members.push(index);
        }
        group.added += item.added;
        group.removed += item.removed;
    }
    // each group's tag: its head's, else its members' if they agree, else changed
    for group in groups.iter_mut() {
        group.change = match group.head {
            Some(head) => items[head].change,
            None => {
                let mut changes = group.members.iter().map(|&k| items[k].change);
                let first = changes.next().unwrap_or(ItemChange::Changed);
                if changes.all(|change| change == first) {
                    first
                } else {
                    ItemChange::Changed
                }
            }
        };
    }
    // the lines no item holds, last, if any of them changed
    let (other_added, other_removed) = other_counts(outline, added, removed);
    if other_added + other_removed > 0 {
        groups.push(Group {
            key: OTHER_KEY.to_string(),
            kind: None,
            name: "Other lines".to_string(),
            change: ItemChange::Changed,
            head: None,
            members: Vec::new(),
            added: other_added,
            removed: other_removed,
        });
    }
    groups
}

/// The runs of a file's lines one item holds, or that no item holds, each kept only if it holds
/// a changed line or a line `keep` asks for, such as one a thread is on
///
/// A run ends where its hunk does, or where a line goes to another item.
///
/// # Arguments
///
/// * `held` - Which item each line of each hunk goes to, as [`attribute`] gives it
/// * `hunks` - The file's hunks
/// * `item` - The item whose runs to find, or `None` for the lines no item holds
/// * `keep` - Whether a line keeps its run though nothing in it changed
pub fn segments(
    held: &[Vec<Option<usize>>],
    hunks: &[Hunk],
    item: Option<usize>,
    keep: impl Fn(&Line) -> bool,
) -> Vec<Segment> {
    let mut runs = Vec::new();
    for (index, (hunk, held)) in hunks.iter().zip(held).enumerate() {
        let mut at = 0;
        while at < hunk.lines.len() {
            // skip the lines another item holds
            if held[at] != item {
                at += 1;
                continue;
            }
            // the run from here to the first line held otherwise
            let start = at;
            while at < hunk.lines.len() && held[at] == item {
                at += 1;
            }
            // kept if anything in it changed, or holds what is asked for
            let lines = &hunk.lines[start..at];
            if lines.iter().any(|line| changed(line) || keep(line)) {
                runs.push(Segment {
                    hunk: index,
                    start,
                    end: at,
                });
            }
        }
    }
    runs
}

/// The items of a group whose lines are drawn in its body, its head first
///
/// # Arguments
///
/// * `group` - The group
pub fn drawn(group: &Group) -> Vec<Option<usize>> {
    // the lines no item holds are drawn as the group's one run of lines
    if group.is_other() {
        return vec![None];
    }
    group
        .head
        .into_iter()
        .chain(group.members.iter().copied())
        .map(Some)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        Item, ItemChange, ItemKind, OTHER_KEY, Outline, Owner, Segment, Span, attribute,
        attribute_one, groups, other_counts, segments,
    };
    use crate::revision::{FileDiff, FileStatus, FileSummary, Hunk, Line, LineKind, Side};

    /// A group as the tests compare it: its key, kind, change, head, members and counts
    type Shape<'a> = (
        &'a str,
        Option<ItemKind>,
        ItemChange,
        Option<usize>,
        &'a [usize],
        u32,
        u32,
    );

    /// A line of a kind, numbered on the sides that kind is on, holding some text
    ///
    /// # Arguments
    ///
    /// * `kind` - Its kind
    /// * `old` - Its number in the base
    /// * `new` - Its number in the worktree
    /// * `text` - What it holds
    fn line(kind: LineKind, old: Option<u32>, new: Option<u32>, text: &str) -> Line {
        Line {
            kind,
            old,
            new,
            text: text.to_string(),
            no_newline: false,
        }
    }

    /// An item of a kind, named, at a depth, on the sides given
    ///
    /// # Arguments
    ///
    /// * `kind` - Its kind
    /// * `name` - Its name
    /// * `owner` - Its owner's name and kind, if it has one
    /// * `depth` - How deep it is nested
    /// * `old` - Its span in the base
    /// * `new` - Its span in the worktree
    fn item(
        kind: ItemKind,
        name: &str,
        owner: Option<(&str, ItemKind)>,
        depth: u32,
        old: Option<(u32, u32)>,
        new: Option<(u32, u32)>,
    ) -> Item {
        // the change follows from the sides it is on
        let change = match (old, new) {
            (None, Some(_)) => ItemChange::Added,
            (Some(_), None) => ItemChange::Removed,
            _ => ItemChange::Changed,
        };
        Item {
            kind,
            name: name.to_string(),
            owner: owner.map(|(name, kind)| Owner {
                name: name.to_string(),
                kind: Some(kind),
            }),
            via: None,
            change,
            depth,
            old: old.map(|(start, end)| Span { start, end }),
            new: new.map(|(start, end)| Span { start, end }),
            added: 0,
            removed: 0,
        }
    }

    /// An impl of `Scaler` whose first method changed and a second one added after it with a
    /// blank line, and an import added above it
    ///
    /// The worktree reads:
    ///
    /// ```text
    ///  1 use std::io;            (added)
    ///  2 impl Scaler {
    ///  3     fn run(&self) {
    ///  4         go();            (go_old() before)
    ///  5     }
    ///  6                          (added)
    ///  7     fn stop(&self) {     (added)
    ///  8     }                    (added)
    ///  9 }
    /// ```
    fn scaler() -> (Vec<Item>, Vec<Hunk>) {
        let items = vec![
            item(
                ItemKind::Impl,
                "Scaler",
                Some(("Scaler", ItemKind::Struct)),
                0,
                Some((1, 7)),
                Some((2, 9)),
            ),
            item(
                ItemKind::Fn,
                "run",
                Some(("Scaler", ItemKind::Struct)),
                1,
                Some((2, 4)),
                Some((3, 5)),
            ),
            item(
                ItemKind::Fn,
                "stop",
                Some(("Scaler", ItemKind::Struct)),
                1,
                None,
                Some((7, 8)),
            ),
        ];
        let hunks = vec![Hunk {
            old_start: 1,
            old_lines: 6,
            new_start: 1,
            new_lines: 9,
            lines: vec![
                line(LineKind::Added, None, Some(1), "use std::io;"),
                line(LineKind::Context, Some(1), Some(2), "impl Scaler {"),
                line(LineKind::Context, Some(2), Some(3), "    fn run(&self) {"),
                line(LineKind::Removed, Some(3), None, "        go_old();"),
                line(LineKind::Added, None, Some(4), "        go();"),
                line(LineKind::Context, Some(4), Some(5), "    }"),
                line(LineKind::Added, None, Some(6), ""),
                line(LineKind::Added, None, Some(7), "    fn stop(&self) {"),
                line(LineKind::Added, None, Some(8), "    }"),
                line(LineKind::Context, Some(5), Some(9), "}"),
            ],
        }];
        (items, hunks)
    }

    /// A line goes to the deepest item holding it, a blank change to the change beside it, and
    /// a line no item holds to none
    #[test]
    fn lines_go_to_their_deepest_item() {
        let (items, hunks) = scaler();
        let held = attribute(&items, &hunks);
        assert_eq!(
            held,
            [vec![
                None,
                Some(0),
                Some(1),
                Some(1),
                Some(1),
                Some(1),
                // the blank line added goes with the method after it, not the impl
                Some(2),
                Some(2),
                Some(2),
                Some(0),
            ]]
        );
        // a removed line is found by its number in the base
        assert_eq!(
            attribute_one(
                &Outline {
                    language: "rust".to_string(),
                    items: items.clone()
                },
                Side::Old,
                3
            ),
            Some(1)
        );
    }

    /// Of two items on the same lines, as `impl X { fn a() {} }` on one line puts them, the
    /// deeper wins, and of equals the first
    #[test]
    fn the_deeper_of_two_items_on_a_line_wins() {
        let items = vec![
            item(
                ItemKind::Impl,
                "X",
                Some(("X", ItemKind::Struct)),
                0,
                None,
                Some((1, 1)),
            ),
            item(
                ItemKind::Fn,
                "a",
                Some(("X", ItemKind::Struct)),
                1,
                None,
                Some((1, 1)),
            ),
        ];
        let hunks = vec![Hunk {
            old_start: 0,
            old_lines: 0,
            new_start: 1,
            new_lines: 1,
            lines: vec![line(LineKind::Added, None, Some(1), "impl X { fn a() {} }")],
        }];
        assert_eq!(attribute(&items, &hunks), [vec![Some(1)]]);
        // reversed, the deeper still wins
        let reversed: Vec<Item> = items.into_iter().rev().collect();
        assert_eq!(attribute(&reversed, &hunks), [vec![Some(0)]]);
    }

    /// A blank line removed between two changes goes with the one before it, and a run of
    /// blank changes alone stays where its lines fall
    #[test]
    fn blank_changes_follow_the_change_before_them() {
        let items = vec![
            item(ItemKind::Fn, "a", None, 0, Some((1, 2)), Some((1, 2))),
            item(ItemKind::Fn, "b", None, 0, Some((4, 5)), Some((4, 5))),
        ];
        let hunks = vec![Hunk {
            old_start: 1,
            old_lines: 5,
            new_start: 1,
            new_lines: 5,
            lines: vec![
                line(LineKind::Removed, Some(2), None, "  a_old"),
                line(LineKind::Removed, Some(3), None, "   "),
                line(LineKind::Added, None, Some(2), "  a_new"),
                line(LineKind::Context, Some(4), Some(4), "fn b() {"),
                line(LineKind::Added, None, Some(3), ""),
            ],
        }];
        let held = attribute(&items, &hunks);
        // the blank line removed goes with `a`'s change before it, though no item holds line 3
        assert_eq!(held[0][1], Some(0));
        // a blank change with no change beside it stays where it falls
        assert_eq!(held[0][4], None);
    }

    /// Counting with only the items kept puts every changed line where the whole outline did
    #[test]
    fn the_kept_items_hold_the_changes_the_whole_outline_did() {
        let (mut items, hunks) = scaler();
        // the whole outline, an unchanged method `idle` after `stop` included
        items.push(item(
            ItemKind::Fn,
            "idle",
            Some(("Scaler", ItemKind::Struct)),
            1,
            Some((5, 6)),
            Some((10, 11)),
        ));
        let whole = attribute(&items, &hunks);
        let kept = attribute(&items[..3], &hunks);
        for (hunk, (whole, kept)) in hunks.iter().zip(whole.iter().zip(&kept)) {
            for (k, line) in hunk.lines.iter().enumerate() {
                if line.kind != LineKind::Context {
                    assert_eq!(whole[k], kept[k], "line {k}");
                }
            }
        }
    }

    /// A type's group takes its definition's tag, else its members' if they agree, and the
    /// lines no item holds come last
    #[test]
    fn groups_gather_members_under_their_owner() {
        let (mut items, hunks) = scaler();
        // counted as the capture counts them
        let held = attribute(&items, &hunks);
        for (k, item) in items.iter_mut().enumerate() {
            for (hunk, held) in hunks.iter().zip(&held) {
                for (line, at) in hunk.lines.iter().zip(held) {
                    if *at == Some(k) {
                        match line.kind {
                            LineKind::Added => item.added += 1,
                            LineKind::Removed => item.removed += 1,
                            LineKind::Context => {}
                        }
                    }
                }
            }
        }
        // a loose fn removed further down, alone in its group
        items.push(item(ItemKind::Fn, "when", None, 0, Some((40, 44)), None));
        items[3].removed = 5;
        let outline = Outline {
            language: "rust".to_string(),
            items,
        };
        let groups = groups(&outline, 5, 6);
        let shape: Vec<Shape> = groups
            .iter()
            .map(|g| {
                (
                    g.key.as_str(),
                    g.kind,
                    g.change,
                    g.head,
                    g.members.as_slice(),
                    g.added,
                    g.removed,
                )
            })
            .collect();
        assert_eq!(
            shape,
            [
                // the struct's definition didn't change, so its members' tags disagree: changed
                (
                    "owner:Scaler",
                    Some(ItemKind::Struct),
                    ItemChange::Changed,
                    None,
                    &[0, 1, 2][..],
                    4,
                    1
                ),
                (
                    "item:fn:when:0",
                    Some(ItemKind::Fn),
                    ItemChange::Removed,
                    Some(3),
                    &[][..],
                    0,
                    5
                ),
                // the import no item holds
                (OTHER_KEY, None, ItemChange::Changed, None, &[][..], 1, 0),
            ]
        );
        assert_eq!(groups[0].label(&outline), "struct Scaler");
        assert_eq!(groups[1].label(&outline), "fn when");
        assert_eq!(other_counts(&outline, 5, 6), (1, 0));
        // a new struct's group is new, as its definition is, whatever its members are
        let new = Outline {
            language: "rust".to_string(),
            items: vec![
                item(ItemKind::Struct, "Fresh", None, 0, None, Some((1, 3))),
                item(
                    ItemKind::Fn,
                    "go",
                    Some(("Fresh", ItemKind::Struct)),
                    1,
                    Some((9, 9)),
                    Some((9, 9)),
                ),
            ],
        };
        let fresh = &super::groups(&new, 0, 0)[0];
        assert_eq!(
            (fresh.change, fresh.head, fresh.members.as_slice()),
            (ItemChange::Added, Some(0), &[1][..])
        );
    }

    /// A run is cut where another item's lines start, and kept only if it changed or holds a
    /// line asked for
    #[test]
    fn segments_keep_changed_runs_and_asked_for_lines() {
        let (items, hunks) = scaler();
        let held = attribute(&items, &hunks);
        // the impl's own lines: its head, a context run, and its closing brace, also context
        assert_eq!(segments(&held, &hunks, Some(0), |_| false), []);
        // unless a thread is on its closing brace
        assert_eq!(
            segments(&held, &hunks, Some(0), |line| line.new == Some(9)),
            [Segment {
                hunk: 0,
                start: 9,
                end: 10
            }]
        );
        // `run`'s lines, its head's context included
        assert_eq!(
            segments(&held, &hunks, Some(1), |_| false),
            [Segment {
                hunk: 0,
                start: 2,
                end: 6
            }]
        );
        // and the import, which no item holds
        assert_eq!(
            segments(&held, &hunks, None, |_| false),
            [Segment {
                hunk: 0,
                start: 0,
                end: 1
            }]
        );
    }

    /// A file kept before outlines reads as one with none, one with none writes none, and a kind
    /// or change a newer build names reads as one this build knows
    #[test]
    fn outlines_read_back_and_old_rows_read_without_them() {
        // a file written with no outline says nothing of one
        let file = FileDiff {
            status: FileStatus::Added,
            old_path: None,
            new_path: Some("a.rs".to_string()),
            old_mode: None,
            new_mode: Some("100644".to_string()),
            old_id: None,
            new_id: Some("b".repeat(40)),
            binary: false,
            too_large: false,
            hunks: Vec::new(),
            outline: None,
        };
        let json = serde_json::to_value(&file).unwrap();
        assert!(json.get("outline").is_none());
        assert_eq!(serde_json::from_value::<FileDiff>(json).unwrap(), file);
        // one with an outline keeps it, through its summary too
        let outlined = FileDiff {
            outline: Some(Outline {
                language: "rust".to_string(),
                items: vec![item(ItemKind::Fn, "main", None, 0, None, Some((1, 1)))],
            }),
            ..file
        };
        let json = serde_json::to_value(outlined.summary()).unwrap();
        assert_eq!(json["outline"]["items"][0]["kind"], "fn");
        assert_eq!(json["outline"]["items"][0]["change"], "added");
        assert!(json["outline"]["items"][0].get("depth").is_none());
        assert_eq!(
            serde_json::from_value::<FileSummary>(json).unwrap().outline,
            outlined.outline
        );
        // a kind and a change a newer build names
        let newer: Item = serde_json::from_value(serde_json::json!({
            "kind": "class", "name": "Shape", "change": "moved", "added": 1, "removed": 0,
        }))
        .unwrap();
        assert_eq!(
            (newer.kind, newer.change),
            (ItemKind::Other, ItemChange::Changed)
        );
    }
}
