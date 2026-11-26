//! `reef node log`, `reef page log` and `reef queue log`: who made each edit, what it changed and
//! when, newest first
//!
//! Each line is one write: the version it left its node or page at, when it was made, who made
//! it, and what it changed, such as `v5  2026-11-26 05:47:07 +0000  Claude, for ada: status
//! in-progress → done`. A write sent with no token names nobody, so its line leaves who out, and
//! a whole plan's write says so.
//!
//! A history is read a page of 200 entries at a time. A page with older entries kept ends by
//! naming the cursor onto them, which `--older` hands back; `--all` reads every page.

use reef_api::{HistoryPage, Logged};
use reef_model::{NodeName, PlanName};
use std::future::Future;
use std::io::Write;

use crate::client::Client;
use crate::start::when;

/// Prints a page of a history, newest first, a line an entry, or says there is none, then the
/// cursor onto its older entries if any are kept
///
/// # Arguments
///
/// * `page` - The page, its entries newest first
/// * `none` - What to say of a history with no entry
/// * `out` - Where to print it
pub fn show<W: Write>(page: &HistoryPage, none: &str, out: &mut W) -> anyhow::Result<()> {
    // nothing kept is said plainly
    if page.edits.is_empty() {
        writeln!(out, "{none}")?;
    }
    for logged in &page.edits {
        writeln!(out, "{}", line(logged))?;
    }
    // and older entries, if any are kept, are a cursor away
    if let Some(older) = &page.older {
        writeln!(out, "older edits are kept: pass --older {older}")?;
    }
    Ok(())
}

/// One entry as a line: the version it left, when, who if known, and what it changed
///
/// # Arguments
///
/// * `logged` - The entry
fn line(logged: &Logged) -> String {
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
    line
}

/// Reads a history from the page asked for: that page alone, or it and every older one
///
/// # Arguments
///
/// * `older` - The cursor a page named, or `None` for the newest page
/// * `all` - Whether to read every page older than it too
/// * `read` - Reads one page, the one older than a cursor or the newest
pub async fn pages<F, Fut>(older: Option<&str>, all: bool, read: F) -> anyhow::Result<HistoryPage>
where
    F: Fn(Option<String>) -> Fut,
    Fut: Future<Output = anyhow::Result<HistoryPage>>,
{
    // the page asked for
    let mut page = read(older.map(str::to_string)).await?;
    // then, for all of it, each older page in turn until none is left
    while all && let Some(cursor) = page.older.take() {
        let next = read(Some(cursor)).await?;
        page.edits.extend(next.edits);
        page.older = next.older;
    }
    Ok(page)
}

/// Prints a node's history, newest first
///
/// # Arguments
///
/// * `client` - The service's client
/// * `node` - The node's full name
/// * `older` - The cursor a page named, to print the page older than it, or `None` for the newest
/// * `all` - Whether to print every page from there on
/// * `out` - Where to print it
pub async fn node<W: Write>(
    client: &Client,
    node: &NodeName,
    older: Option<&str>,
    all: bool,
    out: &mut W,
) -> anyhow::Result<()> {
    let page = pages(older, all, |older| async move {
        client.node_history(node, older.as_deref()).await
    })
    .await?;
    show(&page, &format!("no edit of `{node}` is kept"), out)
}

/// Prints one of a plan's own pages' history, newest first
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan's full name
/// * `path` - The page's path
/// * `older` - The cursor a page named, to print the page older than it, or `None` for the newest
/// * `all` - Whether to print every page from there on
/// * `out` - Where to print it
pub async fn page<W: Write>(
    client: &Client,
    plan: &PlanName,
    path: &str,
    older: Option<&str>,
    all: bool,
    out: &mut W,
) -> anyhow::Result<()> {
    let page = pages(older, all, |older| async move {
        client.page_history(plan, path, older.as_deref()).await
    })
    .await?;
    show(&page, &format!("no edit of page `{path}` is kept"), out)
}

/// Prints a project's queue's history, newest first
///
/// # Arguments
///
/// * `client` - The service's client
/// * `project` - The project's name
/// * `older` - The cursor a page named, to print the page older than it, or `None` for the newest
/// * `all` - Whether to print every page from there on
/// * `out` - Where to print it
pub async fn queue<W: Write>(
    client: &Client,
    project: &str,
    older: Option<&str>,
    all: bool,
    out: &mut W,
) -> anyhow::Result<()> {
    let page = pages(older, all, |older| async move {
        client.queue_history(project, older.as_deref()).await
    })
    .await?;
    show(
        &page,
        &format!("no edit of `{project}`'s queue is kept"),
        out,
    )
}

#[cfg(test)]
mod tests {
    use super::{pages, show};
    use reef_api::{Author, Edited, HistoryPage, Logged, Place, Writer};
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
        let page = HistoryPage {
            edits: history,
            older: None,
        };
        let mut out = Vec::new();
        show(&page, "none", &mut out).unwrap();
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "v5    2026-11-18 11:06:40 +0000  Claude, for ada: status in-progress → done\n\
             -     1970-01-01 00:00:00 +0000: queued reef/central/C50 at the top (the whole plan \
             written)\n"
        );
        // a page with older entries kept ends naming the cursor onto them
        let paged = HistoryPage {
            older: Some("00000000000000000001.00000000000000ab".to_string()),
            ..page
        };
        let mut out = Vec::new();
        show(&paged, "none", &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        assert!(
            out.ends_with(
                "older edits are kept: pass --older 00000000000000000001.00000000000000ab\n"
            ),
            "{out}"
        );
        // and a history with nothing kept says so
        let mut out = Vec::new();
        show(&HistoryPage::default(), "no edit of `C1` is kept", &mut out).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "no edit of `C1` is kept\n");
    }

    /// An entry at a time, as numbers, for a history read in pages
    ///
    /// # Arguments
    ///
    /// * `at` - The entry's time, which names it
    fn entry(at: u64) -> Logged {
        Logged {
            at,
            author: Author::default(),
            version: None,
            whole_plan: false,
            edits: vec![Edited::Body],
        }
    }

    /// One page is read unless every page is asked for, which reads each older page in turn
    #[tokio::test]
    async fn every_page_is_read_only_when_asked() {
        // three pages, newest first, each naming the next by the time it starts at
        let read = |older: Option<String>| async move {
            let page = match older.as_deref() {
                None => HistoryPage {
                    edits: vec![entry(6), entry(5)],
                    older: Some("4".to_string()),
                },
                Some("4") => HistoryPage {
                    edits: vec![entry(4), entry(3)],
                    older: Some("2".to_string()),
                },
                Some("2") => HistoryPage {
                    edits: vec![entry(2)],
                    older: None,
                },
                Some(other) => anyhow::bail!("no page after {other}"),
            };
            Ok(page)
        };
        let times = |page: &HistoryPage| page.edits.iter().map(|e| e.at).collect::<Vec<_>>();
        // the newest page alone keeps its cursor
        let page = pages(None, false, read).await.unwrap();
        assert_eq!(
            (times(&page), page.older.as_deref()),
            (vec![6, 5], Some("4"))
        );
        // a cursor reads the page older than it
        let page = pages(Some("4"), false, read).await.unwrap();
        assert_eq!(
            (times(&page), page.older.as_deref()),
            (vec![4, 3], Some("2"))
        );
        // and all of it reads to the end, naming no cursor
        let page = pages(None, true, read).await.unwrap();
        assert_eq!((times(&page), page.older), (vec![6, 5, 4, 3, 2], None));
    }
}
