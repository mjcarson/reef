//! `mdbook-reef next`: the work ready to start, in plan order, and what the rest waits on.

use crate::check::open_book;
use crate::plan::Plan;
use anyhow::Result;
use reef_model::{Graph, Unmet};
use std::fmt::Write as _;
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
    print!("{}", report(&plan.graph));
    Ok(())
}

/// The work ready to start, in plan order, then each piece of work that waits, with a line
/// for its own needs and one for each of its groups' that are not done
///
/// # Arguments
///
/// * `g` - The plan
pub fn report(g: &Graph) -> String {
    let ready = g.ready_in_order();
    let waiting = g.waiting_in_order();
    // ids are padded to the widest, so titles line up
    let width = ready.iter().chain(waiting.iter().map(|(i, _)| i)).map(|&i| g.node(i).id.chars().count()).max().unwrap_or(0);
    let mut out = String::new();
    // the ready work first, in the order it is to be taken
    let _ = writeln!(out, "Ready to start ({}), in plan order:", ready.len());
    for &i in &ready {
        let _ = writeln!(out, "  {:width$}  {}", g.node(i).id, g.node(i).title);
    }
    if ready.is_empty() {
        let _ = writeln!(out, "  nothing");
    }
    // then the work that waits, each with what holds it back
    let _ = writeln!(out, "\nWaiting ({}):", waiting.len());
    for (i, unmet) in &waiting {
        let _ = writeln!(out, "  {:width$}  {}", g.node(*i).id, g.node(*i).title);
        for (through, needs) in by_group(unmet) {
            // each need with how far along it is
            let list: Vec<String> = needs.iter().map(|&d| format!("{} ({})", g.node(d).id, g.status(d).as_str())).collect();
            // a group's need is named for the group
            let whose = through.map_or(String::new(), |t| format!("{} ", g.node(t).id));
            let _ = writeln!(out, "  {:width$}    {whose}needs {}", "", list.join(", "));
        }
    }
    if waiting.is_empty() {
        let _ = writeln!(out, "  nothing");
    }
    out
}

/// Unmet needs gathered by whose they are, in the order each is first met: the node's own,
/// then each of its groups'
///
/// # Arguments
///
/// * `unmet` - A node's unmet needs
fn by_group(unmet: &[Unmet]) -> Vec<(Option<usize>, Vec<usize>)> {
    let mut out: Vec<(Option<usize>, Vec<usize>)> = Vec::new();
    for u in unmet {
        // add the need to its group's line, or start a line for that group
        match out.iter_mut().find(|(through, _)| *through == u.through) {
            Some((_, needs)) => needs.push(u.need),
            None => out.push((u.through, vec![u.need])),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use reef_model::{Node, Status};

    /// A node with a title, a group, needs and a status
    fn node(id: &str, title: &str, parent: Option<&str>, needs: &[&str], status: Option<Status>) -> Node {
        Node {
            id: id.into(),
            title: title.into(),
            parent: parent.map(Into::into),
            needs: needs.iter().map(|s| s.to_string()).collect(),
            status,
            ..Node::default()
        }
    }

    #[test]
    fn ready_then_waiting() {
        let (g, d) = Graph::build(vec![
            node("X1", "X1 A spike", None, &[], Some(Status::InProgress)),
            node("C1", "C1 First", None, &[], None),
            node("M2", "M2 Store", None, &["X1"], None),
            node("C6", "C6 Schema", Some("M2"), &["C1"], None),
            node("C10", "C10 Config", None, &[], Some(Status::Done)),
        ]);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(
            report(&g),
            "Ready to start (1), in plan order:\n  C1  C1 First\n\nWaiting (1):\n  C6  C6 Schema\n        needs C1 (todo)\n        M2 needs X1 (in-progress)\n"
        );
        // a plan with nothing to do says so
        let (g, _) = Graph::build(vec![node("a", "A", None, &[], Some(Status::Done))]);
        assert_eq!(report(&g), "Ready to start (0), in plan order:\n  nothing\n\nWaiting (0):\n  nothing\n");
    }
}
