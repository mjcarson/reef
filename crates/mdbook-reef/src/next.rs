//! `mdbook-reef next`: the work ready to start, in plan order, and what the rest waits on.

use crate::check::open_book;
use crate::plan::Plan;
use anyhow::Result;
use reef_model::Next;
use std::path::Path;

/// Print a book's work that is ready to start, in plan order, then each piece of work that
/// waits and what it waits on
///
/// # Arguments
///
/// * `dir` - The book's directory, which holds its `book.toml`
pub fn next(dir: &Path) -> Result<()> {
    // load the book's plan
    let (_, src, config) = open_book(dir)?;
    let plan = Plan::load(&src, &config);
    // the plan still has an order with problems in it, but say that they are there
    if !plan.diags.is_empty() {
        eprintln!("warning: the plan has {} problem(s); `mdbook-reef check` lists them", plan.diags.len());
    }
    // the report `reef next` prints too
    print!("{}", Next::of(&plan.graph, plan.diags.len()));
    Ok(())
}
