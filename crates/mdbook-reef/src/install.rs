//! `mdbook-reef install`: put the viewer beside `book.toml` and wire it into the config.

use anyhow::{Context, Result};
use std::path::Path;
use toml_edit::{Array, DocumentMut, Item, Table, value};

pub const JS: &[u8] = include_bytes!("../assets/reef.js");
pub const CSS: &[u8] = include_bytes!("../assets/reef.css");

pub fn install(dir: &Path) -> Result<()> {
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
    Ok(())
}

fn implicit_table() -> Item {
    let mut t = Table::new();
    t.set_implicit(true);
    Item::Table(t)
}
