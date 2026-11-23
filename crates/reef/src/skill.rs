//! The Claude skill for central reef, written into a repo
//!
//! ```text
//! reef skill [DIR] [--print]
//! ```
//!
//! - **The same rules as the mdbook skill, worked through reef.** Draw new work as a node before
//!   starting it, mark it done and its criteria met in the change that finishes it, record
//!   benchmark figures and check after each edit, through reef's MCP tools and its CLI rather
//!   than a book's files.
//! - **It ships in the binary,** so the skill a repo gets names the tools and commands of the
//!   reef that wrote it. Its text lives in `crates/reef/assets/skill`.
//! - **It takes the mdbook skill's name and place,** `.claude/skills/reef/` at the root of the
//!   repo that holds the directory, so a repo holds one or the other: `mdbook-reef skill` writes
//!   the static mode's over it, and `reef skill` writes this one back.
//! - **It needs no service,** since it only writes files.

use anyhow::Context;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The skill's files: each one's name within the skill's directory, and its text
pub const FILES: &[(&str, &str)] = &[
    ("SKILL.md", include_str!("../assets/skill/SKILL.md")),
    ("reference.md", include_str!("../assets/skill/reference.md")),
];

/// Where a skill is written, from the root of the repo it is written in
pub const PLACE: &str = ".claude/skills/reef";

/// The directory the skill goes in, for a directory in a repo or outside every repo
///
/// It is `.claude/skills/reef/` at the root of the repo that holds `dir`, the first ancestor
/// with a `.git`, which is a file in a linked worktree, else in `dir` itself.
///
/// # Arguments
///
/// * `dir` - A directory in the repo, absolute
pub fn place(dir: &Path) -> PathBuf {
    // the first ancestor git keeps a repo or a worktree in, else the directory itself
    let root = dir
        .ancestors()
        .find(|ancestor| ancestor.join(".git").exists())
        .unwrap_or(dir);
    root.join(PLACE)
}

/// Writes the skill into the repo that holds a directory, over any skill of the same name
///
/// Returns the directory it was written in.
///
/// # Arguments
///
/// * `dir` - A directory in the repo, relative to the working directory or absolute
/// * `out` - Where to say where it was written
///
/// # Examples
///
/// ```no_run
/// use std::path::Path;
///
/// # fn example() -> anyhow::Result<()> {
/// let written = reef::skill::write(Path::new("."), &mut std::io::stdout())?;
/// assert!(written.ends_with(".claude/skills/reef"));
/// # Ok(())
/// # }
/// ```
pub fn write<W: Write>(dir: &Path, out: &mut W) -> anyhow::Result<PathBuf> {
    // the directory as it is on disk, so its ancestors are the repo's
    let dir = dir
        .canonicalize()
        .with_context(|| format!("finding {}", dir.display()))?;
    let skill = place(&dir);
    std::fs::create_dir_all(&skill).with_context(|| format!("creating {}", skill.display()))?;
    // each file whole, so a second run writes the same bytes
    for (name, text) in FILES {
        let path = skill.join(name);
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    }
    writeln!(out, "wrote the reef skill to {}", skill.display())?;
    Ok(skill)
}

/// Prints the skill's SKILL.md
///
/// # Arguments
///
/// * `out` - Where to print it
pub fn print<W: Write>(out: &mut W) -> anyhow::Result<()> {
    // the first file is the skill itself, which links the rest
    let (_, text) = FILES[0];
    out.write_all(text.as_bytes())?;
    Ok(())
}
