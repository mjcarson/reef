//! `reef node log`, `reef page log` and `reef queue log`: who made each edit, what it changed and
//! when, newest first
//!
//! Each line is one write: the version it left its node or page at, when it was made, who made
//! it, and what it changed, such as `v5  2026-11-26 05:47:07 +0000  Claude, for ada: status
//! in-progress → done`. A write sent with no token names nobody, so its line leaves who out, and
//! a whole plan's write says so.

use reef_api::Logged;
use reef_model::{NodeName, PlanName};
use std::io::Write;

use crate::client::Client;
use crate::start::when;

/// Prints a history, newest first, a line an entry, or says there is none
///
/// # Arguments
///
/// * `history` - The entries, newest first
/// * `none` - What to say of a history with no entry
/// * `out` - Where to print it
pub fn show<W: Write>(history: &[Logged], none: &str, out: &mut W) -> anyhow::Result<()> {
    // nothing kept is said plainly
    if history.is_empty() {
        writeln!(out, "{none}")?;
        return Ok(());
    }
    for logged in history {
        // the version it left, if it left one, then when
        let version = logged
            .version
            .map_or_else(|| "-".to_string(), |version| format!("v{version}"));
        let mut line = format!("{version:<5} {}", when(logged.at));
        // who, if anyone is known
        if let Some(who) = logged.author.describe() {
            line.push_str("  ");
            line.push_str(&who);
        }
        // then what it changed, and whether it wrote the whole plan
        line.push_str(": ");
        line.push_str(&logged.describe());
        if logged.whole_plan {
            line.push_str(" (the whole plan written)");
        }
        writeln!(out, "{line}")?;
    }
    Ok(())
}

/// Prints a node's history, newest first
///
/// # Arguments
///
/// * `client` - The service's client
/// * `node` - The node's full name
/// * `out` - Where to print it
pub async fn node<W: Write>(client: &Client, node: &NodeName, out: &mut W) -> anyhow::Result<()> {
    let history = client.node_history(node).await?;
    show(&history, &format!("no edit of `{node}` is kept"), out)
}

/// Prints one of a plan's own pages' history, newest first
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan's full name
/// * `path` - The page's path
/// * `out` - Where to print it
pub async fn page<W: Write>(
    client: &Client,
    plan: &PlanName,
    path: &str,
    out: &mut W,
) -> anyhow::Result<()> {
    let history = client.page_history(plan, path).await?;
    show(&history, &format!("no edit of page `{path}` is kept"), out)
}

/// Prints a project's queue's history, newest first
///
/// # Arguments
///
/// * `client` - The service's client
/// * `project` - The project's name
/// * `out` - Where to print it
pub async fn queue<W: Write>(client: &Client, project: &str, out: &mut W) -> anyhow::Result<()> {
    let history = client.queue_history(project).await?;
    show(
        &history,
        &format!("no edit of `{project}`'s queue is kept"),
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::show;
    use reef_api::{Author, Edited, Logged, Place, Writer};
    use reef_model::Status;

    /// Each entry is a line: its version, when, who if known, and what it changed
    #[test]
    fn entries_print_a_line_each() {
        let ada = Author {
            user: Some("ada".to_string()),
            since: Some(1),
            by: Writer::Claude,
        };
        let history = vec![
            Logged {
                at: 1_795_000_000_123,
                author: ada,
                version: Some(5),
                whole_plan: false,
                edits: vec![Edited::Status {
                    from: Some(Status::InProgress),
                    to: Some(Status::Done),
                }],
            },
            Logged {
                at: 0,
                author: Author::default(),
                version: None,
                whole_plan: true,
                edits: vec![Edited::Queued {
                    node: "reef/central/C50".parse().unwrap(),
                    at: Place::Top,
                }],
            },
        ];
        let mut out = Vec::new();
        show(&history, "none", &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "v5    2026-11-18 11:06:40 +0000  Claude, for ada: status in-progress → done\n\
             -     1970-01-01 00:00:00 +0000: queued reef/central/C50 at the top (the whole plan \
             written)\n"
        );
        // and a history with nothing kept says so
        let mut out = Vec::new();
        show(&[], "no edit of `C1` is kept", &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "no edit of `C1` is kept\n");
    }
}
