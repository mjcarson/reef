//! Rendering a node's markdown for the sidebar, and finding the headings of a doc with the ids
//! mdbook gives them.

use crate::paths::{self, Target};
use pulldown_cmark::{CodeBlockKind, CowStr, Event, LinkType, Options, Parser, Tag, TagEnd};
use std::collections::HashSet;

/// Where links in rendered node bodies start. Each page replaces it with its way back to the
/// book's root, so a body is rendered once however many pages show it.
pub const ROOT: &str = "reef-root:/";

/// The markdown extensions mdbook turns on, so reef parses a doc the way mdbook renders it
pub fn md_options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_FOOTNOTES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_HEADING_ATTRIBUTES
}

/// Rendered markdown that is one paragraph, without its `<p>`, so it sits in a line.
///
/// # Arguments
///
/// * `html` - The rendered markdown
pub fn inline_html(html: &str) -> String {
    let t = html.trim();
    match t.strip_prefix("<p>").and_then(|r| r.strip_suffix("</p>")) {
        Some(inner) if !inner.contains("<p>") => inner.to_string(),
        _ => t.to_string(),
    }
}

/// `(id, text, level)` of each heading, with the ids mdbook gives them.
///
/// # Arguments
///
/// * `md` - The markdown to find headings in
pub fn headings_of(md: &str) -> Vec<(String, String, u8)> {
    heading_spans(md).into_iter().map(|h| (h.id, h.text, h.level)).collect()
}

/// A heading, with where it lies in its markdown.
#[derive(Debug, Clone)]
pub struct HeadingSpan {
    /// The id mdbook gives the heading
    pub id: String,
    /// The heading's text
    pub text: String,
    /// The heading's level, 1 for `#`
    pub level: u8,
    /// Where the heading starts.
    pub start: usize,
    /// Where the heading ends and its section's text begins.
    pub end: usize,
}

/// Each heading, with the id mdbook gives it and its byte range.
///
/// # Arguments
///
/// * `md` - The markdown to find headings in
pub fn heading_spans(md: &str) -> Vec<HeadingSpan> {
    let mut out = Vec::new();
    let mut used: HashSet<String> = HashSet::new();
    let mut cur: Option<(Option<String>, String, u8, usize)> = None;
    for (ev, range) in Parser::new_ext(md, md_options()).into_offset_iter() {
        match ev {
            Event::Start(Tag::Heading { level, id, .. }) => cur = Some((id.map(|s| s.to_string()), String::new(), level as u8, range.start)),
            Event::Text(t) | Event::Code(t) => {
                if let Some(c) = &mut cur {
                    c.1.push_str(&t);
                }
            }
            Event::End(TagEnd::Heading(_)) => {
                if let Some((id, text, level, start)) = cur.take() {
                    let id = match id {
                        Some(id) => id,
                        None => {
                            let base = paths::heading_id(&text);
                            let mut id = base.clone();
                            let mut k = 1;
                            while used.contains(&id) {
                                id = format!("{base}-{k}");
                                k += 1;
                            }
                            used.insert(id.clone());
                            id
                        }
                    };
                    out.push(HeadingSpan { id, text, level, start, end: range.end });
                }
            }
            _ => {}
        }
    }
    out
}

/// Render a node's body for the sidebar: links made book-relative, its `reef` fences and a
/// first heading that repeats its title left out.
///
/// # Arguments
///
/// * `md` - The body as markdown
/// * `source` - The path the body's links are written relative to
/// * `title` - The node's title
pub fn render_body(md: &str, source: &str, title: &str) -> String {
    render(md, source, Some(title), false)
}

/// Render markdown written in `source` for the sidebar. With `title`, a first heading that
/// repeats it is left out. With `drop_struck`, struck-through text is left out.
///
/// # Arguments
///
/// * `md` - The markdown to render
/// * `source` - The path its links are written relative to
/// * `title` - A title a first heading may repeat
/// * `drop_struck` - Whether to leave struck-through text out
pub fn render(md: &str, source: &str, title: Option<&str>, drop_struck: bool) -> String {
    let title = title.unwrap_or("\u{0}");
    let mut struck = 0usize;
    let mut events = Vec::new();
    let mut skip_fence = false;
    let mut first_heading: Option<Vec<Event<'_>>> = None;
    let mut seen_block = false;
    for ev in Parser::new_ext(md, md_options()) {
        match &ev {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) if is_reef(info) => {
                skip_fence = true;
                continue;
            }
            Event::End(TagEnd::CodeBlock) if skip_fence => {
                skip_fence = false;
                continue;
            }
            _ if skip_fence => continue,
            Event::Start(Tag::Strikethrough) if drop_struck => {
                struck += 1;
                continue;
            }
            Event::End(TagEnd::Strikethrough) if drop_struck => {
                struck = struck.saturating_sub(1);
                continue;
            }
            _ if struck > 0 => continue,
            _ => {}
        }
        if !seen_block {
            if let Event::Start(Tag::Heading { .. }) = ev {
                first_heading = Some(vec![]);
            }
            if let Some(h) = &mut first_heading {
                let end = matches!(ev, Event::End(TagEnd::Heading(_)));
                h.push(ev.clone().into_static());
                if end {
                    seen_block = true;
                    let text: String = h.iter().filter_map(|e| if let Event::Text(t) | Event::Code(t) = e { Some(t.to_string()) } else { None }).collect();
                    if text.trim() != title.trim() {
                        events.extend(first_heading.take().unwrap());
                    }
                    first_heading = None;
                }
                continue;
            }
            seen_block = true;
        }
        let ev = match ev {
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
                Event::Start(Tag::Link { link_type, dest_url: rebase(link_type, source, &dest_url), title, id })
            }
            Event::Start(Tag::Image { link_type, dest_url, title, id }) => {
                Event::Start(Tag::Image { link_type, dest_url: rebase(link_type, source, &dest_url), title, id })
            }
            e => e.into_static(),
        };
        events.push(ev.into_static());
    }
    let mut html = String::new();
    pulldown_cmark::html::push_html(&mut html, events.into_iter());
    html
}

/// A link written in `source`, made to start from [`ROOT`] if it names a file of the book, with a
/// markdown file's name changed to the page mdbook writes for it
///
/// An autolink is left as written: `<a@b.org>` is a mail address, whose `mailto:` the HTML
/// writer adds, not a file beside `source`.
///
/// # Arguments
///
/// * `link_type` - How the link was written
/// * `source` - The path the link is written relative to
/// * `dest` - The link as written
fn rebase(link_type: LinkType, source: &str, dest: &str) -> CowStr<'static> {
    // an autolink names a URL or a mail address, never a file of the book
    if matches!(link_type, LinkType::Autolink | LinkType::Email) {
        return CowStr::from(dest.to_string());
    }
    match paths::resolve(source, dest) {
        Target::External(u) => CowStr::from(u),
        Target::Internal { path, anchor } => {
            let base = format!("{ROOT}{}", paths::html_of(&path));
            CowStr::from(match anchor {
                Some(a) => format!("{base}#{a}"),
                None => base,
            })
        }
    }
}

/// Render a doc as the page mdbook writes for it: a whole HTML document whose `<main>` holds
/// the doc, each heading with the id mdbook gives it, so a section is cut out of it as out of a
/// book's page. Its links start with [`ROOT`], and its `reef` fences are left out.
///
/// # Arguments
///
/// * `md` - The doc's markdown, without its frontmatter
/// * `source` - The doc's path, which its links are written relative to
///
/// # Examples
///
/// ```
/// use reef_render::render_page;
///
/// let page = render_page("# Design\n\n## Rows\n\nSee [f](f.md#x).\n", "docs/design.md");
/// assert!(page.contains("<h2 id=\"rows\">Rows</h2>"));
/// assert!(page.contains("href=\"reef-root:/docs/f.html#x\""));
/// ```
pub fn render_page(md: &str, source: &str) -> String {
    // the ids mdbook gives the headings, in the order they come
    let spans = heading_spans(md);
    let mut ids = spans.iter().map(|h| CowStr::from(h.id.clone()));
    // the page is titled by its first heading, or else by its path
    let title = spans.first().map_or(source, |h| h.text.as_str());
    let mut events = Vec::new();
    let mut skip_fence = false;
    for ev in Parser::new_ext(md, md_options()) {
        let ev = match ev {
            // a plan drawn on the doc's page isn't drawn here
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) if is_reef(&info) => {
                skip_fence = true;
                continue;
            }
            Event::End(TagEnd::CodeBlock) if skip_fence => {
                skip_fence = false;
                continue;
            }
            _ if skip_fence => continue,
            // each heading gets its id, so a link's anchor finds it
            Event::Start(Tag::Heading { level, classes, attrs, .. }) => Event::Start(Tag::Heading { level, id: ids.next(), classes, attrs }),
            // and each link leads where it did from the doc
            Event::Start(Tag::Link { link_type, dest_url, title, id }) => {
                Event::Start(Tag::Link { link_type, dest_url: rebase(link_type, source, &dest_url), title, id })
            }
            Event::Start(Tag::Image { link_type, dest_url, title, id }) => {
                Event::Start(Tag::Image { link_type, dest_url: rebase(link_type, source, &dest_url), title, id })
            }
            e => e,
        };
        events.push(ev);
    }
    // the doc inside a page's `<main>`, as mdbook lays its pages out
    let mut main = String::new();
    pulldown_cmark::html::push_html(&mut main, events.into_iter());
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n</head>\n<body>\n<main>\n{main}</main>\n</body>\n</html>\n",
        paths::escape(title)
    )
}

/// Whether a fenced code block's info string makes it a ```` ```reef ```` fence
///
/// # Arguments
///
/// * `info` - The fence's info string
pub fn is_reef(info: &str) -> bool {
    info.split(|c: char| c.is_whitespace() || c == ',' || c == '{').next() == Some("reef")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_links_book_relative() {
        let html = render_body("# T\n\nSee [b](../b.md#x) and [w](https://w.org).\n", "a/n.md", "T");
        assert!(html.contains("href=\"reef-root:/b.html#x\""), "{html}");
        assert!(html.contains("https://w.org"));
        assert!(!html.contains("<h1>"), "repeated title dropped: {html}");
    }

    /// A mail autolink keeps its address, and a URL autolink its URL
    #[test]
    fn autolinks_left_as_written() {
        // in a node's body
        let html = render_body("Mail <a@b.org> or see <https://w.org>.\n", "plan/n.md", "T");
        assert!(html.contains("href=\"mailto:a@b.org\""), "{html}");
        assert!(html.contains("href=\"https://w.org\""), "{html}");
        // and on a doc's page
        let page = render_page("Mail <a@b.org>.\n", "docs/a.md");
        assert!(page.contains("href=\"mailto:a@b.org\""), "{page}");
    }

    #[test]
    fn heading_ids_match_mdbook() {
        let h = headings_of("# A b\n## A b\n### X {#custom}\n");
        assert_eq!(h[0].0, "a-b");
        assert_eq!(h[1].0, "a-b-1");
        assert_eq!(h[2].0, "custom");
    }

    /// A doc's page holds it in a `<main>`, each heading with the id check finds it by
    #[test]
    fn page_has_main_and_mdbooks_heading_ids() {
        let md = "# A & b\n\n## Rows\n\nSee [o](other.md#x).\n\n```reef\nfocus = \"M3\"\n```\n\n## Rows\n\n### Mine {#custom}\n\n~~Was.~~\n";
        let page = render_page(md, "docs/a.md");
        // titled by its first heading, the doc inside its `<main>`
        assert!(page.starts_with("<!doctype html>"), "{page}");
        assert!(page.contains("<title>A &amp; b</title>"), "{page}");
        assert!(page.contains("<main>\n<h1 id=\"a--b\">"), "{page}");
        // every heading has the id the doc's headings are found by, a repeat's and a written one too
        for (id, _, _) in headings_of(md) {
            assert!(page.contains(&format!(" id=\"{id}\"")), "no heading `{id}`: {page}");
        }
        assert!(page.contains("<h2 id=\"rows-1\">") && page.contains("<h3 id=\"custom\">"), "{page}");
        // its links start from the root, its plan isn't drawn, and struck text stays for the viewer to drop
        assert!(page.contains("href=\"reef-root:/docs/other.html#x\""), "{page}");
        assert!(!page.contains("focus"), "{page}");
        assert!(page.contains("<del>Was.</del>"), "{page}");
    }

    #[test]
    fn reef_fences() {
        assert!(is_reef("reef"));
        assert!(is_reef("reef {x}"));
        assert!(!is_reef("reefs"));
        assert!(!is_reef("rust"));
    }

    #[test]
    fn one_paragraph_inline() {
        assert_eq!(inline_html("<p>a <b>b</b></p>\n"), "a <b>b</b>");
        assert_eq!(inline_html("<p>a</p><p>b</p>"), "<p>a</p><p>b</p>");
    }
}
