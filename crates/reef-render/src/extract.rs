//! Brief forms of a node's docs for the sidebar: why the work is needed, what it does, and the
//! design choices, taken from the doc sections a node's `docs` link to.
//!
//! A doc link points at a file or at one heading of it. The why is the first paragraph or list
//! of a subsection named like "Context", or a table row named like "Impact". A list gives each
//! item's bold lead, or the whole item where it has none. The what is the same from a
//! subsection or row named like "What it does". The design choices are a subsection named like
//! "Design choices". Struck-through text is left out, since a brief form says what is true now.

use crate::markdown::{heading_spans, md_options, render};
use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// The text below the heading `anchor` names, up to the next heading at its level or above.
/// With no anchor, the whole file below its title.
///
/// # Arguments
///
/// * `md` - The doc to look in
/// * `anchor` - The heading id the link names, if any
pub fn section<'a>(md: &'a str, anchor: Option<&str>) -> Option<&'a str> {
    let hs = heading_spans(md);
    match anchor {
        Some(a) => {
            let k = hs.iter().position(|h| h.id == a)?;
            let end = hs[k + 1..].iter().find(|h| h.level <= hs[k].level).map_or(md.len(), |h| h.start);
            Some(&md[hs[k].end..end])
        }
        None => {
            let start = hs.first().filter(|h| h.level == 1 && md[..h.start].trim().is_empty()).map_or(0, |h| h.end);
            Some(&md[start..])
        }
    }
}

/// The first subsection of `md` titled one of `names` (ignoring case): its title and its text.
///
/// # Arguments
///
/// * `md` - The section to look in
/// * `names` - The titles a subsection may have
pub fn subsection<'a>(md: &'a str, names: &[String]) -> Option<(String, &'a str)> {
    let hs = heading_spans(md);
    let k = hs.iter().position(|h| names.iter().any(|n| n.trim().eq_ignore_ascii_case(h.text.trim())))?;
    let end = hs[k + 1..].iter().find(|h| h.level <= hs[k].level).map_or(md.len(), |h| h.start);
    Some((hs[k].text.trim().to_string(), &md[hs[k].end..end]))
}

/// A block of a doc section that stands on its own, which a brief part may be taken from.
#[derive(Debug, PartialEq)]
enum Block<'a> {
    /// A paragraph that is not in a list, quote, table or footnote
    Para(&'a str),
    /// A list that is not in a quote, table or footnote, as the text of each of its own items
    List(Vec<&'a str>),
}

/// The paragraphs and lists of `md` that stand on their own, in order: not in a quote, table
/// or footnote, and no list nested in another's item.
///
/// # Arguments
///
/// * `md` - The section to look in
fn top_blocks(md: &str) -> Vec<Block<'_>> {
    let mut out = Vec::new();
    // how deep in lists, quotes, tables and footnotes the parser is
    let mut depth = 0usize;
    for (ev, range) in Parser::new_ext(md, md_options()).into_offset_iter() {
        match ev {
            // a list at the top opens a block of its own
            Event::Start(Tag::List(_)) if depth == 0 => {
                out.push(Block::List(Vec::new()));
                depth += 1;
            }
            // an item of that list is kept whole, with any list nested in it
            Event::Start(Tag::Item) if depth == 1 => {
                if let Some(Block::List(items)) = out.last_mut() {
                    items.push(&md[range]);
                }
            }
            // anything else that holds blocks is gone into
            Event::Start(Tag::List(_) | Tag::BlockQuote(_) | Tag::Table(_) | Tag::FootnoteDefinition(_)) => depth += 1,
            // and come back out of
            Event::End(TagEnd::List(_) | TagEnd::BlockQuote(_) | TagEnd::Table | TagEnd::FootnoteDefinition) => {
                depth = depth.saturating_sub(1)
            }
            // a paragraph at the top is a block of its own
            Event::Start(Tag::Paragraph) if depth == 0 => out.push(Block::Para(&md[range])),
            _ => {}
        }
    }
    out
}

/// A block's brief form, if any text is left once struck-through text is dropped: a paragraph
/// whole, or a list of each item's brief form.
///
/// # Arguments
///
/// * `block` - The paragraph or list to brief
/// * `source` - The doc's file, for its links
fn brief_block(block: &Block<'_>, source: &str) -> Option<String> {
    let html = match block {
        // a paragraph is taken whole
        Block::Para(p) => render(p, source, None, true),
        // a list gives a line an item that has any text
        Block::List(items) => {
            let lines: String = items
                .iter()
                .filter_map(|item| item_brief(item, source))
                .map(|line| format!("<li>{line}</li>"))
                .collect();
            if lines.is_empty() { lines } else { format!("<ul>{lines}</ul>") }
        }
    };
    has_text(&html).then_some(html)
}

/// A list item's brief form: the bold lead it opens with, as in "- **Probed once a second**,
/// off the loop", or the whole item where it opens with none. Struck-through text is dropped
/// first, so a lead struck through gives way to the lead written after it.
///
/// # Arguments
///
/// * `item` - The list item, with its marker
/// * `source` - The doc's file, for its links
fn item_brief(item: &str, source: &str) -> Option<String> {
    // render the item without its marker or its struck-through text
    let html = render(&item_text(item), source, None, true);
    // the bold text it opens with, if it opens with any
    let open = html.trim_start();
    let open = open.strip_prefix("<p>").unwrap_or(open).trim_start();
    let lead = open.strip_prefix("<strong>").and_then(|rest| rest.find("</strong>").map(|end| &rest[..end]));
    // else the whole item
    let brief = lead.unwrap_or(&html).trim();
    has_text(brief).then(|| brief.to_string())
}

/// Whether rendered html holds any text.
///
/// # Arguments
///
/// * `html` - The rendered html to look in
pub fn has_text(html: &str) -> bool {
    let mut in_tag = false;
    html.chars().any(|c| match c {
        '<' => {
            in_tag = true;
            false
        }
        '>' => {
            in_tag = false;
            false
        }
        c => !in_tag && !c.is_whitespace(),
    })
}

/// The first table row of `md` whose first cell is one of `names`, as in `| **Impact** | ... |`,
/// the way an entry in a register of optimizations or issues opens: the first cell's text and
/// the second cell.
///
/// # Arguments
///
/// * `md` - The section to look in
/// * `names` - The first cells a row may have
fn table_row<'a>(md: &'a str, names: &[String]) -> Option<(String, &'a str)> {
    let mut cell = 0usize;
    let mut first = String::new();
    let mut in_first = false;
    for (ev, range) in Parser::new_ext(md, md_options()).into_offset_iter() {
        match ev {
            Event::Start(Tag::TableRow | Tag::TableHead) => {
                cell = 0;
                first.clear();
            }
            Event::Start(Tag::TableCell) => {
                cell += 1;
                in_first = cell == 1;
                let name = first.trim().trim_end_matches(':');
                if cell == 2 && names.iter().any(|n| n.trim().eq_ignore_ascii_case(name)) {
                    return Some((name.to_string(), md[range].trim().trim_matches('|').trim()));
                }
            }
            Event::End(TagEnd::TableCell) => in_first = false,
            Event::Text(t) | Event::Code(t) if in_first => first.push_str(&t),
            _ => {}
        }
    }
    None
}

/// One part of a node's brief form in a doc section, such as its why or its what: the first
/// paragraph or list with text left once struck-through text is dropped, from a subsection
/// named in `names`, else a table row named in `names`. A list gives each item's bold lead, or
/// the whole item where it has none.
///
/// A section's own opening is never taken, since it rarely says either plainly. Returns the
/// subsection's title or the row's name it was found under, and the rendered html.
///
/// # Arguments
///
/// * `sec` - The doc section to look in
/// * `names` - The subsection titles and table row names that hold this part
/// * `source` - The doc's file, for its links
pub fn brief_part(sec: &str, names: &[String], source: &str) -> Option<(String, String)> {
    // a subsection named for this part comes first
    if let Some((title, text)) = subsection(sec, names) {
        // its first paragraph or list with text
        return top_blocks(text).iter().find_map(|b| brief_block(b, source)).map(|html| (title, html));
    }
    // then a register entry's table row
    let (name, cell) = table_row(sec, names)?;
    let html = render(cell, source, None, true);
    has_text(&html).then_some((name, html))
}

/// The design choices in a doc section: the subsection named in `names`, rendered, with the
/// subsection's title.
///
/// # Arguments
///
/// * `sec` - The doc section to look in
/// * `names` - The titles the design choices' subsection may have
/// * `source` - The doc's file, for its links
pub fn design(sec: &str, names: &[String], source: &str) -> Option<(String, String)> {
    let (title, text) = subsection(sec, names)?;
    let html = render_choices(text, source);
    has_text(&html).then_some((title, html))
}

/// The tables of `md` that stand on their own and have a header row, which is where a doc's
/// measurements are: a register entry's opening `| | |` table, whose header is empty, is left
/// out. Each comes with its caption, the bold text that opens the paragraph before it, as in
/// "**The cell alone** (2026-11-09). ...".
///
/// # Arguments
///
/// * `md` - The section to look in
pub fn measured_tables(md: &str) -> Vec<Option<String>> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut last_para: Option<std::ops::Range<usize>> = None;
    let mut in_head = false;
    let mut head_text = String::new();
    for (ev, range) in Parser::new_ext(md, md_options()).into_offset_iter() {
        match ev {
            Event::Start(Tag::Table(_)) if depth == 0 => {
                depth += 1;
                head_text.clear();
            }
            Event::End(TagEnd::Table) if depth == 1 => {
                depth = 0;
                if !head_text.trim().is_empty() {
                    let caption = last_para.take().and_then(|r| {
                        let p = md[r].trim_start();
                        let body = p.strip_prefix("**")?;
                        let end = body.find("**")?;
                        let c = body[..end].trim().trim_end_matches(['.', ':']).trim();
                        (!c.is_empty() && !c.contains("~~")).then(|| c.to_string())
                    });
                    out.push(caption);
                }
                last_para = None;
            }
            Event::Start(Tag::TableHead) => in_head = true,
            Event::End(TagEnd::TableHead) => in_head = false,
            Event::Text(t) | Event::Code(t) if in_head => head_text.push_str(&t),
            Event::Start(Tag::List(_) | Tag::BlockQuote(_) | Tag::FootnoteDefinition(_) | Tag::Table(_)) => depth += 1,
            Event::End(TagEnd::List(_) | TagEnd::BlockQuote(_) | TagEnd::FootnoteDefinition | TagEnd::Table) => {
                depth = depth.saturating_sub(1)
            }
            Event::Start(Tag::Paragraph) if depth == 0 => last_para = Some(range),
            Event::Start(Tag::Heading { .. }) => last_para = None,
            _ => {}
        }
    }
    out
}

/// Render design choices. A list whose items open with a bold lead, as in
/// "- **Runs cut in disk order.** Because ...", becomes one `<details>` an item, its lead the
/// summary, so a long list reads at a glance. Everything else renders as it is.
///
/// # Arguments
///
/// * `md` - The design choices as markdown
/// * `source` - The doc's file, for its links
pub fn render_choices(md: &str, source: &str) -> String {
    let mut out = String::new();
    let mut depth = 0usize;
    let mut last = 0usize;
    let mut items: Vec<std::ops::Range<usize>> = Vec::new();
    let mut list_start = 0usize;
    for (ev, range) in Parser::new_ext(md, md_options()).into_offset_iter() {
        match ev {
            Event::Start(Tag::List(_)) => {
                if depth == 0 {
                    list_start = range.start;
                    items.clear();
                }
                depth += 1;
            }
            Event::Start(Tag::Item) if depth == 1 => items.push(range),
            Event::End(TagEnd::List(_)) => {
                depth -= 1;
                if depth == 0 {
                    let leads: Vec<Option<(String, String)>> = items.iter().map(|r| bold_lead(&md[r.clone()])).collect();
                    if !leads.is_empty() && leads.iter().filter(|l| l.is_some()).count() * 2 >= leads.len() {
                        out.push_str(&render(&md[last..list_start], source, None, true));
                        out.push_str(r#"<div class="reef-choices">"#);
                        for (r, lead) in items.iter().zip(leads) {
                            out.push_str(&choice(&md[r.clone()], lead, source));
                        }
                        out.push_str("</div>");
                        last = range.end;
                    }
                }
            }
            _ => {}
        }
    }
    out.push_str(&render(&md[last..], source, None, true));
    out
}

/// One list item's text with its marker taken off and its lines brought back to the margin.
///
/// # Arguments
///
/// * `item` - The list item, with its marker
fn item_text(item: &str) -> String {
    let t = item.trim_start();
    let marker = t
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '-' | '*' | '+' | '.' | ')')))
        .unwrap_or(t.len());
    let rest = &t[marker..];
    let width = (item.len() - t.len()) + marker + (rest.len() - rest.trim_start_matches(' ').len());
    let mut out = String::new();
    for (k, line) in rest.trim_start_matches(' ').lines().enumerate() {
        if k > 0 {
            out.push('\n');
            let pad = line.len() - line.trim_start_matches(' ').len();
            out.push_str(&line[pad.min(width)..]);
        } else {
            out.push_str(line);
        }
    }
    out
}

/// An item's bold lead and the rest, if it opens with one.
///
/// # Arguments
///
/// * `item` - The list item, with its marker
fn bold_lead(item: &str) -> Option<(String, String)> {
    let text = item_text(item);
    let body = text.strip_prefix("**")?;
    let end = body.find("**")?;
    Some((body[..end].to_string(), body[end + 2..].trim().to_string()))
}

/// One design choice: a `<details>` whose summary is its bold lead, or the item as it is
/// where it has none. A choice struck through whole renders as nothing.
///
/// # Arguments
///
/// * `item` - The list item, with its marker
/// * `lead` - The item's bold lead and the rest, if it opens with one
/// * `source` - The doc's file, for its links
fn choice(item: &str, lead: Option<(String, String)>, source: &str) -> String {
    let text = item_text(item);
    match lead {
        Some((lead, rest)) => {
            let summary = render(&lead, source, None, true);
            let summary = summary.trim().trim_start_matches("<p>").trim_end_matches("</p>");
            let rest = render(&rest, source, None, true);
            if !has_text(summary) {
                return String::new();
            }
            format!("<details><summary>{summary}</summary>{rest}</details>")
        }
        None => {
            // A choice struck through whole, or one with no lead.
            let html = render(&text, source, None, true);
            if has_text(&html) { format!(r#"<div class="reef-choice">{html}</div>"#) } else { String::new() }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Section names as the config holds them
    fn names(n: &[&str]) -> Vec<String> {
        n.iter().map(|s| s.to_string()).collect()
    }

    const FEATURE: &str = "# F1. A thing\n\n## Context\n\nWhy it exists.\n\n## What it does\n\nIt does ~~that~~ this, see [F2](f2.md).\n\nMore.\n\n## Design choices\n\nIntro.\n\n- **Runs in disk order.** Because\n  the device likes it.\n- ~~**Old choice.** Gone.~~\n- **Waves drawn whole.** Once.\n\n## Tests\n\nNone.\n";

    #[test]
    fn a_feature_page() {
        let sec = section(FEATURE, None).unwrap();
        // the what comes from its subsection, which it names
        let (from, d) = brief_part(sec, &names(&["What it does"]), "features/f1.md").unwrap();
        assert_eq!(from, "What it does");
        assert!(d.contains("It does  this") || d.contains("It does this"), "{d}");
        assert!(!d.contains("that"), "struck text dropped: {d}");
        assert!(d.contains("href=\"reef-root:/features/f2.html\""), "links rebased: {d}");
        // the why comes from the context
        let (from, why) = brief_part(sec, &names(&["Why", "Context"]), "features/f1.md").unwrap();
        assert_eq!((from.as_str(), why.trim()), ("Context", "<p>Why it exists.</p>"));
        // with no subsection or row of its names, nothing, not the section's own opening
        assert!(brief_part(sec, &names(&["Motivation"]), "features/f1.md").is_none());
        let (from, html) = design(sec, &names(&["design choices"]), "features/f1.md").unwrap();
        assert_eq!(from, "Design choices");
        assert!(html.contains("<p>Intro.</p>"), "{html}");
        assert!(html.contains("<summary>Runs in disk order.</summary>"), "{html}");
        assert!(html.contains("the device likes it"), "{html}");
        assert!(!html.contains("Old choice"), "a struck choice is dropped: {html}");
        assert!(!html.contains("None."), "stops at the next section: {html}");
        assert_eq!(html.matches("<details>").count(), 2, "{html}");
    }

    #[test]
    fn an_anchored_entry_skips_its_table() {
        let md = "# Optimizations\n\n### O1. First\n\n| | |\n|---|---|\n| **Rank** | high |\n\nThe first entry.\n\n### O2. Second\n\nThe second.\n";
        let sec = section(md, Some("o1-first")).unwrap();
        // the entry's own paragraph is never taken
        assert!(brief_part(sec, &names(&["What it does"]), "a.md").is_none());
        // a row of the register table, named
        let (from, html) = brief_part(sec, &names(&["What it does", "Rank"]), "a.md").unwrap();
        assert_eq!((from.as_str(), html.trim()), ("Rank", "<p>high</p>"));
        assert!(brief_part(sec, &names(&["Impact"]), "a.md").is_none());
        assert!(design(sec, &names(&["Design choices"]), "a.md").is_none());
        assert!(section(md, Some("nope")).is_none());
    }

    #[test]
    fn a_section_written_as_a_list() {
        let md = "# I1. A bug\n\n## Symptom\n\n~~Struck.~~\n\n- **Every run logged an ERROR** as its client went:\n  65 of them.\n- ~~**Old lead** gone~~ **New lead**, kept.\n- ~~Wholly struck.~~\n- A plain item, see [F2](f2.md).\n  - **Nested** in it.\n\nA coda.\n\n## The fix\n\nIt was fixed.\n\n- **Not this.**\n";
        let sec = section(md, None).unwrap();
        // a struck paragraph is passed over for the list after it, which comes before the coda
        let (from, why) = brief_part(sec, &names(&["Symptom"]), "a/i1.md").unwrap();
        assert_eq!(from, "Symptom");
        assert!(!why.contains("coda"), "{why}");
        // an item with a bold lead gives its lead alone
        assert!(why.starts_with("<ul><li>Every run logged an ERROR</li>"), "{why}");
        assert!(!why.contains("65 of them"), "{why}");
        // a struck lead gives way to the one after it
        assert!(why.contains("<li>New lead</li>") && !why.contains("Old lead"), "{why}");
        // an item struck through whole is dropped
        assert!(!why.contains("Wholly"), "{why}");
        // an item with no lead is taken whole, with what is nested in it and its links rebased
        assert!(why.contains("A plain item, see <a href=\"reef-root:/a/f2.html\">F2</a>"), "{why}");
        assert!(why.contains("<strong>Nested</strong> in it"), "{why}");
        assert_eq!(why.matches("<li>").count(), 4, "three items and the one nested: {why}");
        // a paragraph before a list is the block taken
        let (from, what) = brief_part(sec, &names(&["The fix"]), "a/i1.md").unwrap();
        assert_eq!((from.as_str(), what.trim()), ("The fix", "<p>It was fixed.</p>"));
    }

    #[test]
    fn measured_tables_skip_the_register() {
        let md = "### O1. First\n\n| | |\n| --- | --- |\n| **Rank** | high |\n\n**Probed** (2026-11-08). Five writers.\n\n| Way | Commits |\n| --- | --- |\n| Buffered | 374 |\n\nPlain words.\n\n| Figure | A | B |\n| --- | --- | --- |\n| x | 1 | 2 |\n\n- a list\n\n  | In | list |\n  | --- | --- |\n  | 1 | 2 |\n";
        assert_eq!(measured_tables(md), vec![Some("Probed".to_string()), None]);
    }

    #[test]
    fn items_lose_their_markers() {
        assert_eq!(item_text("- **A.** b\n  c\n"), "**A.** b\nc");
        assert_eq!(item_text("1. x\n   y"), "x\ny");
        assert_eq!(bold_lead("- **Lead.** rest").unwrap(), ("Lead.".into(), "rest".into()));
        assert!(bold_lead("- plain").is_none());
    }
}
