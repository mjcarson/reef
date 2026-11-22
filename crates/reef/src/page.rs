//! reef's page commands: a plan's own markdown pages listed, got, put and removed through the
//! JSON API
//!
//! ```text
//! reef page ls                  # list the plan's pages, each at its version
//! reef page get <PATH>          # print a page's markdown, after the version it was read at
//! reef page put <PATH> [FILE]   # put a page's markdown, from FILE or standard input
//! reef page rm <PATH>           # take a page out of the plan
//! ```
//!
//! A page is named by its path, as a node's doc link names it: a plan's pages lie over its
//! repo's files, and a page hides the repo's file at its path.
//!
//! - **The version rides in a comment.** `get` prints `<!-- version: N -->` on the line before
//!   the markdown, which markdown renders as nothing. `put` reads that first line back as the
//!   version its writer read, and leaves it out of the page, so a put made after another
//!   writer's is refused as stale. Markdown without it is a new page, and is refused as stale
//!   if the path is taken.
//! - **Every edit is checked.** A page that a node's doc link names, or whose heading one
//!   names, can't be taken out or lose that heading while the link stands.
//!
//! An edit that is stale, or that would add a problem to the plan, writes nothing and fails with
//! what was wrong.

use reef_api::{PageEdit, PageEdited, PageWrite, Refusal, StoredPage, check_page_path};
use reef_model::PlanName;
use std::io::Write;

use crate::client::Client;

/// What opens the comment `get` prints a page's version in
const VERSION_OPEN: &str = "<!-- version:";

/// What closes it
const VERSION_CLOSE: &str = "-->";

/// Lists a plan's pages by path, each at the version an edit of it names
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan whose pages to list
/// * `out` - Where to print them
pub async fn ls<W: Write>(client: &Client, plan: &PlanName, out: &mut W) -> anyhow::Result<()> {
    // every page as it stands, by path
    let listing = client.pages(plan).await?;
    if listing.pages.is_empty() {
        writeln!(out, "`{plan}` keeps no pages")?;
        return Ok(());
    }
    for page in &listing.pages {
        writeln!(out, "{} (version {})", page.path, page.version)?;
    }
    Ok(())
}

/// Prints a page's markdown, after the version it was read at
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan the page is in
/// * `path` - The page's path
/// * `out` - Where to print it
pub async fn get<W: Write>(
    client: &Client,
    plan: &PlanName,
    path: &str,
    out: &mut W,
) -> anyhow::Result<()> {
    // the page as it stands, or the service's error naming it
    let page = client.page(plan, path).await?;
    out.write_all(printed(&page).as_bytes())?;
    Ok(())
}

/// Puts a page's markdown, unless it is stale or breaks the plan
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan to put it in
/// * `path` - The page's path
/// * `text` - The markdown, after the version line `get` printed, if it was got
/// * `out` - Where to print the page written, at its new version
pub async fn put<W: Write>(
    client: &Client,
    plan: &PlanName,
    path: &str,
    text: &str,
    out: &mut W,
) -> anyhow::Result<()> {
    // a path no page can have is refused before anything is sent
    check_page_path(path).map_err(|why| anyhow::anyhow!("{why}"))?;
    // the version its writer read, and the markdown without the line that says it
    let (read, text) = read_version(text)?;
    let edit = PageEdit::Put {
        page: PageWrite::new(path, text),
        read,
    };
    // checked against the whole plan
    let edited = client.edit_page(plan, &edit).await?;
    let version = settled(edited, plan, path, "put")?;
    writeln!(
        out,
        "put page `{path}` at version {}",
        version.unwrap_or_default()
    )?;
    // a page hides the repo's file at its path, which is worth knowing when there is one
    let projects = client.projects().await?;
    let repo = projects
        .into_iter()
        .find(|project| project.name == plan.project)
        .map(|project| project.repo);
    if repo.is_some_and(|repo| repo.join(path).is_file()) {
        writeln!(
            out,
            "note: the page hides the file `{path}` in `{}`'s repo, which its links led to",
            plan.project
        )?;
    }
    Ok(())
}

/// Takes a page out of a plan, unless it has moved on or that breaks the plan
///
/// # Arguments
///
/// * `client` - The service's client
/// * `plan` - The plan to take it out of
/// * `path` - The page's path
/// * `out` - Where to print the page taken out
pub async fn rm<W: Write>(
    client: &Client,
    plan: &PlanName,
    path: &str,
    out: &mut W,
) -> anyhow::Result<()> {
    // the page at the version it stands at, so one that moves on before the removal is kept
    let page = client.page(plan, path).await?;
    let edit = PageEdit::Remove {
        path: path.to_string(),
        read: page.version,
    };
    // checked against the whole plan
    let edited = client.edit_page(plan, &edit).await?;
    settled(edited, plan, path, "removal")?;
    writeln!(out, "removed page `{path}`")?;
    Ok(())
}

/// A page as `get` prints it: the version it was read at, as a comment, then its markdown
///
/// # Arguments
///
/// * `page` - The page as it was read
///
/// # Examples
///
/// ```
/// use reef::page::printed;
/// use reef_api::StoredPage;
///
/// let page = StoredPage { path: "plan.md".into(), text: "# Plan\n".into(), version: 3 };
/// assert_eq!(printed(&page), "<!-- version: 3 -->\n# Plan\n");
/// ```
pub fn printed(page: &StoredPage) -> String {
    // the version first, as a comment markdown renders as nothing, then the page as written
    format!(
        "{VERSION_OPEN} {} {VERSION_CLOSE}\n{}",
        page.version, page.text
    )
}

/// Reads the version a page was got at from its first line, and the markdown after it, or
/// `None` and the whole markdown for a new page
///
/// Only a first line that is the comment `get` prints is read, and left out of the page.
///
/// # Arguments
///
/// * `text` - The markdown, as `get` printed it or as written by hand
///
/// # Examples
///
/// ```
/// use reef::page::read_version;
///
/// assert_eq!(read_version("<!-- version: 3 -->\n# Plan\n").unwrap(), (Some(3), "# Plan\n"));
/// assert_eq!(read_version("# Plan\n").unwrap(), (None, "# Plan\n"));
/// ```
pub fn read_version(text: &str) -> anyhow::Result<(Option<u64>, &str)> {
    // the first line, and the markdown after it
    let (first, rest) = match text.split_once('\n') {
        Some((first, rest)) => (first, rest),
        None => (text, ""),
    };
    // a first line that isn't the version's comment is the page's own
    let Some(value) = first
        .trim()
        .strip_prefix(VERSION_OPEN)
        .and_then(|value| value.strip_suffix(VERSION_CLOSE))
    else {
        return Ok((None, text));
    };
    // the version is the number `get` printed
    let value = value.trim();
    let read: u64 = value.parse().map_err(|_| {
        anyhow::anyhow!(
            "`{VERSION_OPEN} … {VERSION_CLOSE}` takes the number `reef page get` printed, not \
             `{value}`"
        )
    })?;
    Ok((Some(read), rest))
}

/// The version an applied edit left its page at, or an error saying why it wrote nothing
///
/// # Arguments
///
/// * `edited` - What came of the edit
/// * `plan` - The plan it was made to
/// * `path` - The page's path
/// * `action` - What the edit was, such as `put`, for the message
fn settled(
    edited: PageEdited,
    plan: &PlanName,
    path: &str,
    action: &str,
) -> anyhow::Result<Option<u64>> {
    match edited {
        PageEdited::Applied { version } => Ok(version),
        // the page wasn't as its writer read it, and what reads it again
        PageEdited::Stale { refusal, .. } => {
            let hint = match refusal {
                Refusal::Moved => format!("`reef page get {path}` prints it as it stands"),
                Refusal::Exists => format!(
                    "to change it, put what `reef page get {path}` prints, keeping its \
                     `{VERSION_OPEN} … {VERSION_CLOSE}` line"
                ),
                Refusal::Missing => format!(
                    "leave out the `{VERSION_OPEN} … {VERSION_CLOSE}` line to put it as new"
                ),
            };
            anyhow::bail!(
                "the {action} was refused, since page `{path}` of `{plan}` is not as it was \
                 read: {refusal}: {hint}"
            )
        }
        // check's own messages, one a line
        PageEdited::Refused(problems) => {
            let lines: Vec<String> = problems
                .iter()
                .map(|problem| format!("  {problem}"))
                .collect();
            anyhow::bail!(
                "the {action} was refused, since it would add these problems to `{plan}`:\n{}",
                lines.join("\n")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{printed, read_version, settled};
    use reef_api::{PageEdited, Refusal, StoredPage};
    use reef_model::{Diagnostic, PlanName};

    /// What `get` prints, `put` reads back: the version, and the same markdown
    #[test]
    fn printed_page_reads_back() {
        // a page, its markdown opening with a comment of its own
        let page = StoredPage {
            path: "design/app.md".to_string(),
            text: "<!-- the app -->\n# The app\n\n## Rows\n".to_string(),
            version: 7,
        };
        let text = printed(&page);
        assert_eq!(read_version(&text).unwrap(), (Some(7), page.text.as_str()));
        // only the first line is read, so the page's own comment is kept as written
        assert_eq!(
            read_version(&page.text).unwrap(),
            (None, page.text.as_str())
        );
        // a page of one line, the version's, is empty
        assert_eq!(read_version("<!-- version: 2 -->").unwrap(), (Some(2), ""));
        // a version that isn't a number is refused, naming what it should be
        let error = read_version("<!-- version: two -->\n# A\n")
            .unwrap_err()
            .to_string();
        assert!(error.contains("not `two`"), "{error}");
    }

    /// A stale or refused edit fails saying what to do, and an applied one its version
    #[test]
    fn edits_settled_into_messages() {
        let plan = PlanName::new("reef", "central").unwrap();
        // an applied put says the version the page is now at
        let applied = PageEdited::Applied { version: Some(2) };
        assert_eq!(settled(applied, &plan, "a.md", "put").unwrap(), Some(2));
        // a stale one says how to read the page again
        let stale = PageEdited::Stale {
            path: "a.md".to_string(),
            refusal: Refusal::Moved,
        };
        let error = settled(stale, &plan, "a.md", "put")
            .unwrap_err()
            .to_string();
        assert_eq!(
            error,
            "the put was refused, since page `a.md` of `reef/central` is not as it was read: it \
             changed since it was read: `reef page get a.md` prints it as it stands"
        );
        // and a refused one lists check's messages
        let refused = PageEdited::Refused(vec![Diagnostic {
            source: "reef/central/C1".to_string(),
            line: None,
            message: "doc `a.md` not found (looked for a.md)".to_string(),
        }]);
        let error = settled(refused, &plan, "a.md", "removal")
            .unwrap_err()
            .to_string();
        assert!(
            error.starts_with(
                "the removal was refused, since it would add these problems to `reef/central`:\n  "
            ),
            "{error}"
        );
        assert!(
            error.ends_with("doc `a.md` not found (looked for a.md)"),
            "{error}"
        );
    }
}
