//! `mdbook-reef install`: put the viewer beside `book.toml` and wire it into the config.

use anyhow::{Context, Result};
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Table, value};

pub const JS: &[u8] = include_bytes!("../assets/reef.js");
pub const CSS: &[u8] = include_bytes!("../assets/reef.css");
/// The Claude skill that teaches agents to keep the plan in step with the work.
pub const SKILL: &[(&str, &[u8])] = &[
    ("SKILL.md", include_bytes!("../assets/skill/SKILL.md")),
    ("reference.md", include_bytes!("../assets/skill/reference.md")),
];

pub fn install(dir: &Path, skill: bool) -> Result<()> {
    let book_toml = dir.join("book.toml");
    let text = std::fs::read_to_string(&book_toml).with_context(|| format!("reading {}", book_toml.display()))?;
    let mut doc: DocumentMut = text.parse().with_context(|| format!("parsing {}", book_toml.display()))?;

    let pre = doc.entry("preprocessor").or_insert_with(implicit_table).as_table_mut().context("[preprocessor] is not a table")?;
    pre.set_implicit(true);
    if !pre.contains_key("reef") {
        let mut t = Table::new();
        t["command"] = value("mdbook-reef");
        pre.insert("reef", Item::Table(t));
        eprintln!("added [preprocessor.reef]");
    }
    let output = doc.entry("output").or_insert_with(implicit_table).as_table_mut().context("[output] is not a table")?;
    output.set_implicit(true);
    let html = output.entry("html").or_insert(Item::Table(Table::new())).as_table_mut().context("[output.html] is not a table")?;
    for (key, file) in [("additional-js", "reef.js"), ("additional-css", "reef.css")] {
        let arr = html.entry(key).or_insert(value(Array::new())).as_array_mut().with_context(|| format!("{key} is not a list"))?;
        if !arr.iter().any(|v| v.as_str() == Some(file)) {
            arr.push(file);
            eprintln!("added {file} to output.html.{key}");
        }
    }
    std::fs::write(&book_toml, doc.to_string())?;
    std::fs::write(dir.join("reef.js"), JS)?;
    std::fs::write(dir.join("reef.css"), CSS)?;
    eprintln!("wrote reef.js ({} KiB) and reef.css to {}", JS.len() / 1024, dir.display());
    if skill {
        write_skill(dir)?;
    }
    Ok(())
}

/// Write the skill to `.claude/skills/reef/` at the root of the repository that holds `dir`
/// (the first ancestor with a `.git`), or in `dir` itself when it is in none.
pub fn write_skill(dir: &Path) -> Result<()> {
    let dir = dir.canonicalize().with_context(|| format!("finding {}", dir.display()))?;
    let root = dir.ancestors().find(|d| d.join(".git").exists()).unwrap_or(&dir);
    let out = root.join(".claude/skills/reef");
    std::fs::create_dir_all(&out).with_context(|| format!("creating {}", out.display()))?;
    for (name, bytes) in SKILL {
        std::fs::write(out.join(name), bytes)?;
    }
    eprintln!("wrote the reef skill to {}", out.display());
    Ok(())
}

fn implicit_table() -> Item {
    let mut t = Table::new();
    t.set_implicit(true);
    Item::Table(t)
}
