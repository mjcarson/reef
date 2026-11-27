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

/// Whether a skill's directory holds this reef's skill already, each of its files as written
///
/// # Arguments
///
/// * `skill` - The skill's directory, as [`place`] names it
pub fn is_written(skill: &Path) -> bool {
    // every file there, and each the same bytes
    FILES.iter().all(|(name, text)| {
        std::fs::read_to_string(skill.join(name)).is_ok_and(|kept| kept == *text)
    })
}

/// What to tell a session whose repo's skill isn't the one this `reef` writes, as after this
/// `reef` updated itself: that it may be out of step, and how to write it again; or nothing for
/// a repo with this skill, or with none at all
///
/// # Arguments
///
/// * `dir` - A directory in the repo, such as the session's
pub fn stale(dir: &Path) -> Option<String> {
    // a repo set up for reef, whose skill this `reef` would write otherwise
    let skill = place(dir);
    if !skill.join(FILES[0].0).is_file() || is_written(&skill) {
        return None;
    }
    Some(format!(
        "reef's skill in {} isn't the one this `reef` ({}) writes, so it may name tools or \
         commands that have changed. Suggest the user run `reef claude install` from the repo's \
         root, which writes it again with reef's hooks.",
        skill.display(),
        crate::update::describe(&crate::update::BUILD)
    ))
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

#[cfg(test)]
mod tests {
    use super::{place, stale, write};

    /// A session is told to write the skill again only where the repo holds a skill this `reef`
    /// would write otherwise
    #[test]
    fn a_skill_out_of_step_is_said() {
        // a repo of this test's own, with no skill yet
        let repo = std::env::temp_dir().join(format!("reef-skill-{}", std::process::id()));
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        assert_eq!(stale(&repo), None);
        // this reef's skill, as written
        write(&repo, &mut Vec::new()).unwrap();
        assert_eq!(stale(&repo), None);
        // another reef's, as after this one updated itself
        let skill = place(&repo);
        std::fs::write(skill.join("SKILL.md"), "# an older skill\n").unwrap();
        let note = stale(&repo).expect("a skill out of step is said");
        assert!(note.contains("`reef claude install`"), "{note}");
        assert!(note.contains(&skill.display().to_string()), "{note}");
        std::fs::remove_dir_all(&repo).unwrap();
    }
}
