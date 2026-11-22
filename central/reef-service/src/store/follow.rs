//! Following a line from one revision of a node's changes to the next
//!
//! The store keeps no file whole, only each revision's hunks, so a line is followed by comparing
//! two revisions' hunks, never by reading git. Each side of a file is named by its blob, which
//! ties the two revisions together where they share one:
//!
//! - **The same worktree file** holds the line where it was.
//! - **A worktree file that is the next revision's base**, as a round of fixes after the node's
//!   work landed has, holds the line as that base's line of the same number.
//! - **Otherwise the two revisions' sides are aligned** as a patience diff aligns them: a line
//!   of a base both share by its number there, any other by its text, and the lines between two
//!   hunks matching nothing, since neither revision holds them.
//!
//! A line is followed only to a line the next revision's hunks hold on the same side, so its
//! thread can be drawn there. A line deleted or edited, or one the next revision no longer
//! shows, is lost, and its thread is outdated.

use reef_api::{Anchor, FileDiff, FileSummary, Line, LineKind, Side};
use std::collections::HashMap;

/// The most cells the fallback alignment fills, for two runs that share no line found once in
/// each; longer runs are left unpaired
const LCS_CELLS: usize = 1 << 20;

/// One line of one side of a file's hunks, as two revisions' sides are aligned
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Token<'a> {
    /// A line of a base both revisions share, by its number there
    Base(u32),
    /// A line by its text alone
    Text(&'a str),
    /// The lines between two hunks, which neither revision holds, and which match nothing
    Gap,
}

impl Token<'_> {
    /// Whether this token is the same line as another
    ///
    /// # Arguments
    ///
    /// * `other` - The other token
    fn matches(&self, other: &Token) -> bool {
        // a gap is no line, so not even another gap is it
        *self != Token::Gap && self == other
    }
}

/// The file of the next revision that a file of a revision became, if the next one lists it
///
/// It is the one shown under the same path, else one moved from the same base path, else one
/// holding the same worktree blob.
///
/// # Arguments
///
/// * `from` - The file, as its revision lists it
/// * `to` - The next revision's files
pub(super) fn next_file<'a>(from: &FileSummary, to: &'a [FileSummary]) -> Option<&'a FileSummary> {
    // the file shown under the same path
    if let Some(file) = to.iter().find(|file| file.path() == from.path()) {
        return Some(file);
    }
    // one moved away from the path it had in the base
    if let Some(old) = &from.old_path
        && let Some(file) = to.iter().find(|file| file.old_path.as_ref() == Some(old))
    {
        return Some(file);
    }
    // or the same contents, moved
    let id = from.new_id.as_ref()?;
    to.iter().find(|file| file.new_id.as_ref() == Some(id))
}

/// Follows a line of a file in one revision to the same line of the file it became in the
/// next, if the next revision's hunks hold it on the same side
///
/// # Arguments
///
/// * `anchor` - The line, in the revision it is in
/// * `from` - Its file in that revision
/// * `revision` - The next revision's number
/// * `to` - The file it became in the next revision
pub(super) fn follow_line(
    anchor: &Anchor,
    from: &FileDiff,
    revision: u32,
    to: &FileDiff,
) -> Option<Anchor> {
    // a file kept without its lines holds none to follow to
    if to.binary || to.too_large {
        return None;
    }
    // the line's number in the next revision, on the side it was on
    let line = match anchor.side {
        Side::New => follow_new(anchor.line, from, to)?,
        Side::Old => follow_old(anchor.line, from, to)?,
    };
    Some(Anchor {
        revision,
        path: to.path().to_string(),
        side: anchor.side,
        line,
    })
}

/// Follows a line of one revision's worktree to its number in the next one's
///
/// # Arguments
///
/// * `number` - The line's number in the worktree
/// * `from` - Its file in its revision
/// * `to` - The file it became in the next revision
fn follow_new(number: u32, from: &FileDiff, to: &FileDiff) -> Option<u32> {
    // the same worktree file holds it where it was
    if same(&from.new_id, &to.new_id) {
        return holds(to, Side::New, number).then_some(number);
    }
    // a worktree the next revision starts from holds it as that base's line, which is still
    // there only as context
    if same(&from.new_id, &to.old_id) {
        let line = to
            .hunks
            .iter()
            .flat_map(|hunk| &hunk.lines)
            .find(|line| line.kind != LineKind::Added && line.old == Some(number))?;
        return match line.kind {
            LineKind::Context => line.new,
            LineKind::Added | LineKind::Removed => None,
        };
    }
    // otherwise its side of the hunks, aligned with the next revision's
    aligned(number, Side::New, from, to)
}

/// Follows a removed line of one revision's base to its number in the next one's, if the next
/// revision still removes it
///
/// # Arguments
///
/// * `number` - The line's number in the base
/// * `from` - Its file in its revision
/// * `to` - The file it became in the next revision
fn follow_old(number: u32, from: &FileDiff, to: &FileDiff) -> Option<u32> {
    // the same base holds it where it was, if it is still removed
    if same(&from.old_id, &to.old_id) {
        return holds(to, Side::Old, number).then_some(number);
    }
    // a base that is the worktree it was removed from doesn't have it at all
    if same(&from.new_id, &to.old_id) {
        return None;
    }
    // otherwise the two bases' removed and context lines, aligned by their text
    aligned(number, Side::Old, from, to)
}

/// Follows a line by aligning its side of a file's hunks with the same side of the next
/// revision's
///
/// # Arguments
///
/// * `number` - The line's number on its side
/// * `side` - The side it is on
/// * `from` - Its file in its revision
/// * `to` - The file it became in the next revision
fn aligned(number: u32, side: Side, from: &FileDiff, to: &FileDiff) -> Option<u32> {
    // a context line is named by its number in the base where both revisions share the base
    let by_base = same(&from.old_id, &to.old_id);
    let (a, a_lines) = tokens(from, side, by_base);
    let (b, b_lines) = tokens(to, side, by_base);
    // the line's place among its side's
    let at = a_lines
        .iter()
        .position(|line| line.is_some_and(|line| anchored(line, side) == Some(number)))?;
    // and the line it is paired with, which must be anchored on the same side
    let pairs = align(&a, &b);
    let line = b_lines[pairs[at]?]?;
    anchored(line, side)
}

/// Whether two blobs are known and the same
///
/// # Arguments
///
/// * `a` - One blob, if the side has one
/// * `b` - The other
fn same(a: &Option<String>, b: &Option<String>) -> bool {
    a.is_some() && a == b
}

/// A line's number on a side, if a comment there anchors to it: a removed line on the base's
/// side, any other on the worktree's
///
/// # Arguments
///
/// * `line` - The line
/// * `side` - The side
fn anchored(line: &Line, side: Side) -> Option<u32> {
    match (side, line.kind) {
        (Side::Old, LineKind::Removed) => line.old,
        (Side::New, LineKind::Added | LineKind::Context) => line.new,
        _ => None,
    }
}

/// Whether a file's hunks hold a line a comment can anchor to on a side, of a number
///
/// # Arguments
///
/// * `file` - The file
/// * `side` - The side
/// * `number` - The line's number there
fn holds(file: &FileDiff, side: Side, number: u32) -> bool {
    file.hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .any(|line| anchored(line, side) == Some(number))
}

/// One side of a file's hunks as tokens, a gap between each two hunks, with the line each
/// token is
///
/// # Arguments
///
/// * `file` - The file
/// * `side` - The side whose lines to take: the worktree's added and context lines, or the
///   base's removed and context lines
/// * `by_base` - Whether a line of the base is named by its number there, rather than its text
fn tokens(file: &FileDiff, side: Side, by_base: bool) -> (Vec<Token<'_>>, Vec<Option<&Line>>) {
    let mut tokens = Vec::new();
    let mut lines = Vec::new();
    for (index, hunk) in file.hunks.iter().enumerate() {
        // the lines between this hunk and the last are in neither's hunks
        if index > 0 {
            tokens.push(Token::Gap);
            lines.push(None);
        }
        for line in &hunk.lines {
            // each side holds its own lines and the context
            let token = match (side, line.kind, line.old) {
                (Side::New, LineKind::Removed, _) | (Side::Old, LineKind::Added, _) => continue,
                (_, LineKind::Context | LineKind::Removed, Some(old)) if by_base => {
                    Token::Base(old)
                }
                _ => Token::Text(&line.text),
            };
            tokens.push(token);
            lines.push(Some(line));
        }
    }
    (tokens, lines)
}

/// Pairs each token of `a` with the token of `b` that is the same line, if one is, as a
/// patience diff pairs them
///
/// What both runs start and end with is paired first. Then the tokens found once in each run
/// anchor it, as many as keep their order in both, and the runs between them are paired the
/// same way. Runs that share no token found once in each are paired by their longest common
/// subsequence, if they are short enough.
///
/// # Arguments
///
/// * `a` - One side's tokens
/// * `b` - The other's
fn align(a: &[Token], b: &[Token]) -> Vec<Option<usize>> {
    let mut pairs = vec![None; a.len()];
    // the runs left to pair, as the ranges of each they cover
    let mut runs = vec![(0, a.len(), 0, b.len())];
    while let Some((mut a0, mut a1, mut b0, mut b1)) = runs.pop() {
        // what both runs start with
        while a0 < a1 && b0 < b1 && a[a0].matches(&b[b0]) {
            pairs[a0] = Some(b0);
            a0 += 1;
            b0 += 1;
        }
        // and what both end with
        while a0 < a1 && b0 < b1 && a[a1 - 1].matches(&b[b1 - 1]) {
            a1 -= 1;
            b1 -= 1;
            pairs[a1] = Some(b1);
        }
        if a0 == a1 || b0 == b1 {
            continue;
        }
        // the tokens found once in each, as many as keep their order
        let anchors = anchors(&a[a0..a1], &b[b0..b1]);
        if anchors.is_empty() {
            common(a, b, (a0, a1), (b0, b1), &mut pairs);
            continue;
        }
        // each anchor paired, and the runs between them left to pair in turn
        let (mut from_a, mut from_b) = (a0, b0);
        for (i, j) in anchors {
            let (i, j) = (a0 + i, b0 + j);
            pairs[i] = Some(j);
            runs.push((from_a, i, from_b, j));
            from_a = i + 1;
            from_b = j + 1;
        }
        runs.push((from_a, a1, from_b, b1));
    }
    pairs
}

/// The tokens found once in each of two runs, paired by where they are in each, the most of
/// them that keep their order in both
///
/// # Arguments
///
/// * `a` - One run
/// * `b` - The other
fn anchors(a: &[Token], b: &[Token]) -> Vec<(usize, usize)> {
    // how often each token is in each run, and where it was last
    let mut seen: HashMap<Token, (u32, usize, u32, usize)> = HashMap::new();
    for (i, token) in a.iter().enumerate() {
        if *token == Token::Gap {
            continue;
        }
        let entry = seen.entry(*token).or_insert((0, i, 0, 0));
        entry.0 += 1;
        entry.1 = i;
    }
    for (j, token) in b.iter().enumerate() {
        if let Some(entry) = seen.get_mut(token) {
            entry.2 += 1;
            entry.3 = j;
        }
    }
    // those found once in each, in the first run's order
    let mut unique: Vec<(usize, usize)> = seen
        .into_values()
        .filter(|(in_a, _, in_b, _)| *in_a == 1 && *in_b == 1)
        .map(|(_, i, _, j)| (i, j))
        .collect();
    unique.sort_unstable();
    increasing(&unique)
}

/// The longest run of pairs whose second halves increase, of pairs sorted by their first
///
/// # Arguments
///
/// * `pairs` - The pairs, sorted by their first halves
fn increasing(pairs: &[(usize, usize)]) -> Vec<(usize, usize)> {
    // the last pair of the best run found of each length, and the pair before each in its run
    let mut tails: Vec<usize> = Vec::new();
    let mut before: Vec<Option<usize>> = vec![None; pairs.len()];
    for (k, &(_, j)) in pairs.iter().enumerate() {
        // the shortest run whose last pair this one can follow none of, which it ends instead
        let at = tails.partition_point(|&tail| pairs[tail].1 < j);
        before[k] = at.checked_sub(1).map(|shorter| tails[shorter]);
        match tails.get_mut(at) {
            Some(tail) => *tail = k,
            None => tails.push(k),
        }
    }
    // walked back from the longest run's last pair
    let mut run = Vec::with_capacity(tails.len());
    let mut at = tails.last().copied();
    while let Some(k) = at {
        run.push(pairs[k]);
        at = before[k];
    }
    run.reverse();
    run
}

/// Pairs two runs' tokens by their longest common subsequence, if the runs are short enough to
/// fill a table for
///
/// # Arguments
///
/// * `a` - One side's tokens
/// * `b` - The other's
/// * `in_a` - The run of `a` to pair
/// * `in_b` - The run of `b` to pair
/// * `pairs` - Where each token of `a` is paired
fn common(
    a: &[Token],
    b: &[Token],
    (a0, a1): (usize, usize),
    (b0, b1): (usize, usize),
    pairs: &mut [Option<usize>],
) {
    // runs too long to fill a table for are left unpaired
    let (n, m) = (a1 - a0, b1 - b0);
    if n.saturating_mul(m) > LCS_CELLS {
        return;
    }
    // the length of the longest common subsequence of each pair of the runs' tails
    let width = m + 1;
    let mut lengths = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lengths[i * width + j] = if a[a0 + i].matches(&b[b0 + j]) {
                lengths[(i + 1) * width + j + 1] + 1
            } else {
                lengths[(i + 1) * width + j].max(lengths[i * width + j + 1])
            };
        }
    }
    // walked forward, pairing each token the subsequence takes
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[a0 + i].matches(&b[b0 + j]) {
            pairs[a0 + i] = Some(b0 + j);
            i += 1;
            j += 1;
        } else if lengths[(i + 1) * width + j] >= lengths[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Token, align, follow_line, increasing, next_file};
    use reef_api::{Anchor, FileDiff, FileStatus, Hunk, Line, LineKind, Side};

    /// A file's diff from a base to a worktree, each given whole as its lines, with its hunks
    /// taken as git takes them with no context, then given the context asked for
    ///
    /// # Arguments
    ///
    /// * `base` - The base's lines, or `None` for a file the base doesn't have
    /// * `worktree` - The worktree's lines
    /// * `context` - How many lines of context each hunk takes on each side
    fn diff(base: Option<&[&str]>, worktree: &[&str], context: usize) -> FileDiff {
        let old = base.unwrap_or_default();
        // the longest common subsequence of the two, as each line's partner
        let (n, m) = (old.len(), worktree.len());
        let mut lengths = vec![vec![0usize; m + 1]; n + 1];
        for i in (0..n).rev() {
            for j in (0..m).rev() {
                lengths[i][j] = if old[i] == worktree[j] {
                    lengths[i + 1][j + 1] + 1
                } else {
                    lengths[i + 1][j].max(lengths[i][j + 1])
                };
            }
        }
        // every line of either, in the diff's order, removed lines before added ones
        let mut all = Vec::new();
        let (mut i, mut j) = (0, 0);
        while i < n || j < m {
            if i < n && j < m && old[i] == worktree[j] {
                all.push(line(LineKind::Context, Some(i + 1), Some(j + 1), old[i]));
                i += 1;
                j += 1;
            } else if i < n && (j == m || lengths[i + 1][j] >= lengths[i][j + 1]) {
                all.push(line(LineKind::Removed, Some(i + 1), None, old[i]));
                i += 1;
            } else {
                all.push(line(LineKind::Added, None, Some(j + 1), worktree[j]));
                j += 1;
            }
        }
        // the lines a hunk shows: each change and the context around it
        let shown: Vec<bool> = (0..all.len())
            .map(|k| {
                let lo = k.saturating_sub(context);
                let hi = (k + context + 1).min(all.len());
                all[lo..hi]
                    .iter()
                    .any(|line| line.kind != LineKind::Context)
            })
            .collect();
        let mut hunks: Vec<Hunk> = Vec::new();
        let mut open = false;
        for (k, line) in all.into_iter().enumerate() {
            if !shown[k] {
                open = false;
                continue;
            }
            if !open {
                hunks.push(Hunk {
                    old_start: 0,
                    old_lines: 0,
                    new_start: 0,
                    new_lines: 0,
                    lines: Vec::new(),
                });
                open = true;
            }
            hunks.last_mut().unwrap().lines.push(line);
        }
        // each hunk's header, as git counts it
        for hunk in &mut hunks {
            hunk.old_lines = hunk.lines.iter().filter(|l| l.old.is_some()).count() as u32;
            hunk.new_lines = hunk.lines.iter().filter(|l| l.new.is_some()).count() as u32;
            hunk.old_start = hunk.lines.iter().find_map(|l| l.old).unwrap_or(0);
            hunk.new_start = hunk.lines.iter().find_map(|l| l.new).unwrap_or(0);
        }
        FileDiff {
            status: if base.is_some() {
                FileStatus::Modified
            } else {
                FileStatus::Added
            },
            old_path: base.map(|_| "a.rs".to_string()),
            new_path: Some("a.rs".to_string()),
            old_mode: base.map(|_| "100644".to_string()),
            new_mode: Some("100644".to_string()),
            old_id: base.map(blob),
            new_id: Some(blob(worktree)),
            binary: false,
            too_large: false,
            hunks,
        }
    }

    /// A blob id for some lines: the same lines, the same id
    ///
    /// # Arguments
    ///
    /// * `lines` - The lines
    fn blob(lines: &[&str]) -> String {
        use std::hash::{DefaultHasher, Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        lines.hash(&mut hasher);
        format!("{:040x}", hasher.finish())
    }

    /// A line of a hunk
    ///
    /// # Arguments
    ///
    /// * `kind` - Its kind
    /// * `old` - Its number in the base
    /// * `new` - Its number in the worktree
    /// * `text` - Its text
    fn line(kind: LineKind, old: Option<usize>, new: Option<usize>, text: &str) -> Line {
        Line {
            kind,
            old: old.map(|n| n as u32),
            new: new.map(|n| n as u32),
            text: text.to_string(),
            no_newline: false,
        }
    }

    /// A line of revision 1's `a.rs`
    ///
    /// # Arguments
    ///
    /// * `side` - The side it is numbered on
    /// * `line` - Its number there
    fn at(side: Side, line: u32) -> Anchor {
        Anchor {
            revision: 1,
            path: "a.rs".to_string(),
            side,
            line,
        }
    }

    /// Follows a line from one diff to the next, answering its number in the next
    ///
    /// # Arguments
    ///
    /// * `side` - The side it is on
    /// * `number` - Its number there
    /// * `from` - Its file in revision 1
    /// * `to` - The file in revision 2
    fn followed(side: Side, number: u32, from: &FileDiff, to: &FileDiff) -> Option<u32> {
        // every followed line is one revision 2 holds, on the same side
        let anchor = follow_line(&at(side, number), from, 2, to)?;
        assert_eq!((anchor.revision, anchor.side), (2, side));
        assert_eq!(anchor.path, to.path());
        Some(anchor.line)
    }

    /// The base every case in this module starts from: ten numbered lines
    const BASE: [&str; 10] = [
        "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
    ];

    /// An added line moves with lines added and removed above it, and stays put for lines
    /// changed below it
    #[test]
    fn added_lines_follow_past_changes_around_them() {
        // revision 1 adds `new` after `five`, and revision 2 adds a line at the top and drops
        // `two`, then changes `nine`
        let first = diff(
            Some(&BASE),
            &[
                "one", "two", "three", "four", "five", "new", "six", "seven", "eight", "nine",
                "ten",
            ],
            3,
        );
        let second = diff(
            Some(&BASE),
            &[
                "zero", "one", "three", "four", "five", "new", "six", "seven", "eight", "NINE",
                "ten",
            ],
            3,
        );
        // `new` stays on line 6, one added and one removed above it, and `six` follows it
        assert_eq!(followed(Side::New, 6, &first, &second), Some(6));
        assert_eq!(followed(Side::New, 7, &first, &second), Some(7));
        // `three`, shown as context, is now line 3 of the worktree
        assert_eq!(followed(Side::New, 3, &first, &second), Some(3));
        // with two lines added above it instead, it moves down by two
        let third = diff(
            Some(&BASE),
            &[
                "zero", "half", "one", "two", "three", "four", "five", "new", "six", "seven",
                "eight", "nine", "ten",
            ],
            3,
        );
        assert_eq!(followed(Side::New, 6, &first, &third), Some(8));
        // and a line found twice is paired in order, the second `}` with the second
        let braces = diff(Some(&BASE), &["one", "}", "two", "}", "three"], 3);
        let more = diff(Some(&BASE), &["one", "}", "x", "two", "}", "three"], 3);
        assert_eq!(followed(Side::New, 4, &braces, &more), Some(5));
        assert_eq!(followed(Side::New, 2, &braces, &more), Some(2));
    }

    /// A line deleted, or edited, is lost, as is one the next revision no longer shows
    #[test]
    fn deleted_or_edited_lines_are_lost() {
        let first = diff(
            Some(&BASE),
            &[
                "one", "two", "three", "four", "five", "new", "six", "seven", "eight", "nine",
                "ten",
            ],
            3,
        );
        // `new` deleted again, so revision 2 changes nothing near it
        let reverted = diff(
            Some(&BASE),
            &[
                "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "TEN",
            ],
            3,
        );
        assert_eq!(followed(Side::New, 6, &first, &reverted), None);
        // and `five`, context in revision 1, isn't in revision 2's hunks any more
        assert_eq!(followed(Side::New, 5, &first, &reverted), None);
        // `new` edited is a line of other text, so the comment on it is lost too
        let edited = diff(
            Some(&BASE),
            &[
                "one", "two", "three", "four", "five", "NEW", "six", "seven", "eight", "nine",
                "ten",
            ],
            3,
        );
        assert_eq!(followed(Side::New, 6, &first, &edited), None);
        // a base line deleted in revision 2 that revision 1 kept as context is lost
        let dropped = diff(
            Some(&BASE),
            &[
                "one", "two", "three", "four", "new", "six", "seven", "eight", "nine", "ten",
            ],
            3,
        );
        assert_eq!(followed(Side::New, 5, &first, &dropped), None);
        // while `new`, kept, is followed
        assert_eq!(followed(Side::New, 6, &first, &dropped), Some(5));
    }

    /// A removed line is followed while the next revision still removes it, and lost once it
    /// keeps it
    #[test]
    fn removed_lines_follow_while_still_removed() {
        // revision 1 removes `four`, and revision 2 removes it and `eight` too
        let first = diff(
            Some(&BASE),
            &[
                "one", "two", "three", "five", "six", "seven", "eight", "nine", "ten",
            ],
            3,
        );
        let second = diff(
            Some(&BASE),
            &["one", "two", "three", "five", "six", "seven", "nine", "ten"],
            3,
        );
        assert_eq!(followed(Side::Old, 4, &first, &second), Some(4));
        // once revision 3 keeps `four`, the comment on removing it is lost
        let kept = diff(Some(&BASE), &["one", "two", "three", "four", "five"], 3);
        assert_eq!(followed(Side::Old, 4, &first, &kept), None);
    }

    /// A file the node adds is followed by its text alone, a whole file of it
    #[test]
    fn lines_of_an_added_file_follow_by_text() {
        let lines: Vec<String> = (1..=3000).map(|n| format!("line {n}")).collect();
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        let first = diff(None, &lines, 3);
        // a line added at its top, and line 2000 deleted
        let mut edited: Vec<&str> = vec!["header"];
        edited.extend(lines.iter().filter(|line| **line != "line 2000"));
        let second = diff(None, &edited, 3);
        assert_eq!(followed(Side::New, 10, &first, &second), Some(11));
        assert_eq!(followed(Side::New, 2500, &first, &second), Some(2500));
        assert_eq!(followed(Side::New, 2000, &first, &second), None);
    }

    /// A worktree file the next revision didn't change holds its lines where they were, and
    /// one whose base moved is followed by its text
    #[test]
    fn lines_follow_onto_a_moved_base() {
        // revision 1 adds `new` after `five`
        let worktree = [
            "one", "two", "three", "four", "five", "new", "six", "seven", "eight", "nine", "ten",
        ];
        let first = diff(Some(&BASE), &worktree, 3);
        // the base then took another node's line at its top, and the worktree was rebased onto it
        let moved: Vec<&str> = std::iter::once("other").chain(BASE).collect();
        let rebased: Vec<&str> = std::iter::once("other").chain(worktree).collect();
        let second = diff(Some(&moved), &rebased, 3);
        assert_eq!(followed(Side::New, 6, &first, &second), Some(7));
        // with the same worktree file under a new base, the line stays where it was
        let same = diff(Some(&BASE[..9]), &worktree, 3);
        assert_eq!(followed(Side::New, 6, &first, &same), Some(6));
        // and a removed line is followed by its text onto a moved base that still has it
        let removed = diff(Some(&BASE), &["one", "two", "three", "five"], 3);
        let then = diff(Some(&moved), &["other", "one", "two", "three", "five"], 3);
        assert_eq!(followed(Side::Old, 4, &removed, &then), Some(5));
    }

    /// Once a node's work lands in its base, a round of fixes after it holds its lines as the
    /// base's, shown where the fixes are near them
    #[test]
    fn lines_follow_onto_a_base_that_is_their_worktree() {
        // revision 1 adds `new` after `five`, and then lands, so revision 2's base is its
        // worktree, and the fixes change `six` and `ten`
        let worktree = [
            "one", "two", "three", "four", "five", "new", "six", "seven", "eight", "nine", "ten",
        ];
        let first = diff(Some(&BASE), &worktree, 3);
        let fixed = [
            "one", "two", "three", "four", "five", "new", "SIX", "seven", "eight", "nine", "TEN",
        ];
        let second = diff(Some(&worktree), &fixed, 1);
        assert_eq!(followed(Side::New, 6, &first, &second), Some(6));
        // `six` was changed, so it is lost, and `three` is in no hunk of the fixes
        assert_eq!(followed(Side::New, 7, &first, &second), None);
        assert_eq!(followed(Side::New, 3, &first, &second), None);
        // and a line revision 1 removed is in no base after it lands
        let removed = diff(Some(&BASE), &["one", "two", "three", "five"], 3);
        let after = diff(
            Some(&["one", "two", "three", "five"]),
            &["one", "two", "THREE", "five"],
            3,
        );
        assert_eq!(followed(Side::Old, 4, &removed, &after), None);
    }

    /// A binary file, or one too large to keep its lines, holds none to follow to
    #[test]
    fn files_without_lines_lose_them() {
        let first = diff(Some(&BASE), &["one", "new"], 3);
        let mut binary = diff(Some(&BASE), &["one", "new"], 3);
        binary.binary = true;
        binary.hunks.clear();
        assert_eq!(followed(Side::New, 2, &first, &binary), None);
        let mut large = diff(Some(&BASE), &["zero", "one", "new"], 3);
        large.too_large = true;
        large.hunks.clear();
        assert_eq!(followed(Side::New, 2, &first, &large), None);
    }

    /// The next revision's file is the one under the same path, else one moved from the same
    /// base path, else one with the same contents
    #[test]
    fn files_are_found_by_path_then_by_where_they_came_from() {
        let summary = |old: Option<&str>, new: Option<&str>, id: &str| {
            let mut file = diff(Some(&BASE), &["one"], 3);
            file.old_path = old.map(str::to_string);
            file.new_path = new.map(str::to_string);
            file.new_id = Some(id.to_string());
            file.summary()
        };
        let from = summary(Some("a.rs"), Some("a.rs"), "1");
        // the same path first
        let to = [
            summary(Some("a.rs"), Some("b.rs"), "1"),
            summary(Some("a.rs"), Some("a.rs"), "2"),
        ];
        assert_eq!(next_file(&from, &to).map(|f| f.path()), Some("a.rs"));
        // then a move from the same base path
        let to = [
            summary(None, Some("c.rs"), "1"),
            summary(Some("a.rs"), Some("b.rs"), "2"),
        ];
        assert_eq!(next_file(&from, &to).map(|f| f.path()), Some("b.rs"));
        // then the same contents
        let added = summary(None, Some("a.rs"), "1");
        let to = [summary(None, Some("c.rs"), "1")];
        assert_eq!(next_file(&added, &to).map(|f| f.path()), Some("c.rs"));
        assert!(next_file(&added, &[summary(None, Some("d.rs"), "9")]).is_none());
    }

    /// Unique lines anchor the alignment in order, a gap matches nothing, and the longest
    /// increasing run is found
    #[test]
    fn tokens_align_as_a_patience_diff_does() {
        use Token::{Gap, Text};
        // a gap in each never pairs, though both are gaps
        let pairs = align(&[Text("a"), Gap, Text("b")], &[Text("a"), Gap, Text("b")]);
        assert_eq!(pairs, [Some(0), None, Some(2)]);
        // moved lines pair as many as keep their order
        let pairs = align(
            &[Text("a"), Text("b"), Text("c"), Text("d")],
            &[Text("c"), Text("a"), Text("b"), Text("d")],
        );
        assert_eq!(pairs, [Some(1), Some(2), None, Some(3)]);
        // and the run kept is the longest one that increases
        assert_eq!(
            increasing(&[(0, 5), (1, 1), (2, 2), (3, 0), (4, 3)]),
            [(1, 1), (2, 2), (4, 3)]
        );
    }
}
