//! Rewriting the links written in markdown, so text moved to another file still leads where it
//! did.
//!
//! `reef import` moves a book's nodes into a central plan, whose links are read from the repo's
//! root rather than from the file each node was written in, and `reef export` moves them back
//! into a book's files. [`rewrite_links`] changes only each link's destination in the markdown
//! as written, so the rest of the text is kept byte for byte, and it checks its work by parsing
//! the result again. [`rewrite_spec`] does so for every part of a node that is rendered as
//! markdown, and for its doc links.

use crate::markdown::md_options;
use pulldown_cmark::{Event, LinkType, Parser, Tag, TagEnd};
use reef_model::NodeSpec;
use std::ops::Range;

/// A link whose destination couldn't be rewritten in the markdown it was written in
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkError {
    /// Where the link was written, such as `` `C26` description.what ``, once that is known
    pub at: String,
    /// The link's destination as written
    pub link: String,
}

impl std::fmt::Display for LinkError {
    /// Says which link couldn't be rewritten, and where it was written
    ///
    /// # Arguments
    ///
    /// * `f` - The formatter to write to
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // the place first, when it is known
        if !self.at.is_empty() {
            write!(f, "{}: ", self.at)?;
        }
        write!(f, "the link `{}` couldn't be rewritten", self.link)
    }
}

impl std::error::Error for LinkError {}

/// A link or image still open while its text is parsed
struct Open {
    /// Whether its destination is written in it, in parentheses after its text
    inline: bool,
    /// Its destination as parsed
    dest: String,
    /// The furthest byte any part of its text reaches, so its closing `]` is the next one
    text_end: usize,
}

/// Rewrites the destination of each link and image written in `md`
///
/// `map` takes a destination as parsed and gives the one to write instead, or `None` to leave
/// it as written. Inline links and images are rewritten where they are written, and reference
/// links where their definitions are. Autolinks are a URL or a mail address, and are left
/// alone. The rest of the markdown is kept as written.
///
/// # Arguments
///
/// * `md` - The markdown to rewrite
/// * `map` - The destination to write for each one parsed, if it changes
///
/// # Errors
///
/// A link whose destination can't be found where it was written, or whose rewritten text
/// doesn't parse to the same markdown with only its destinations changed, is an error naming
/// it.
///
/// # Examples
///
/// ```
/// use reef_render::links::rewrite_links;
///
/// let map = |dest: &str| Some(format!("docs/{dest}"));
/// let md = "See [the design](design.md#rows \"Rows\") and [ref].\n\n[ref]: <other one.md>\n";
/// let out = rewrite_links(md, &map).unwrap();
/// assert_eq!(out, "See [the design](docs/design.md#rows \"Rows\") and [ref].\n\n[ref]: <docs/other one.md>\n");
/// ```
pub fn rewrite_links(md: &str, map: &dyn Fn(&str) -> Option<String>) -> Result<String, LinkError> {
    // markdown without a bracket holds no link to rewrite
    if !md.contains(']') {
        return Ok(md.to_string());
    }
    let mut edits: Vec<(Range<usize>, String)> = Vec::new();
    let parser = Parser::new_ext(md, md_options()).into_offset_iter();
    // each reference definition is rewritten where it is written, which its links all use
    for (_, def) in parser.reference_definitions().iter() {
        if let Some(new) = changed(map, &def.dest) {
            let (span, angled) =
                definition_dest(md, def.span.clone()).ok_or_else(|| unfound(&def.dest))?;
            edits.push((span, written(&new, angled)));
        }
    }
    // each inline link and image is rewritten where its destination is written
    let mut open: Vec<Open> = Vec::new();
    for (event, range) in parser {
        match &event {
            Event::Start(Tag::Link {
                link_type,
                dest_url,
                ..
            })
            | Event::Start(Tag::Image {
                link_type,
                dest_url,
                ..
            }) => {
                // an inner link or image is part of the text of each one around it
                reach(&mut open, range.end);
                // its text starts after its `[`, or an image's `![`
                let opener = if matches!(event, Event::Start(Tag::Image { .. })) {
                    2
                } else {
                    1
                };
                open.push(Open {
                    inline: *link_type == LinkType::Inline,
                    dest: dest_url.to_string(),
                    text_end: range.start + opener,
                });
            }
            Event::End(TagEnd::Link) | Event::End(TagEnd::Image) => {
                // a link's text is all parsed once it ends, so its destination follows
                let done = open.pop().expect("every link that ends was started");
                if done.inline
                    && let Some(new) = changed(map, &done.dest)
                {
                    let (span, angled) =
                        inline_dest(md, done.text_end).ok_or_else(|| unfound(&done.dest))?;
                    edits.push((span, written(&new, angled)));
                }
                // and it is part of the text of each one around it
                reach(&mut open, range.end);
            }
            _ => reach(&mut open, range.end),
        }
    }
    // nothing to rewrite leaves the markdown as it was
    if edits.is_empty() {
        return Ok(md.to_string());
    }
    // splice the edits from the back, so each earlier span still lies where it was found
    edits.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
    let mut out = md.to_string();
    let mut limit = usize::MAX;
    for (span, new) in edits {
        // two edits of one destination would undo each other
        if span.end > limit {
            return Err(unfound(&md[span.clone()]));
        }
        limit = span.start;
        out.replace_range(span, &new);
    }
    // the rewritten markdown must parse as the old did, with only its destinations changed
    check(md, &out, map)?;
    Ok(out)
}

/// Rewrites the doc links of a node and of each of its inline members, and the links in every
/// part of them rendered as markdown: their descriptions, design choices, the note under
/// their measurements, their acceptance criteria and an inline member's body
///
/// A criterion's `test` is a path printed as code, not a link, and is left as written.
///
/// # Arguments
///
/// * `spec` - The node to rewrite, members and all
/// * `map` - The destination to write for each one parsed, if it changes
///
/// # Errors
///
/// The first link that can't be rewritten, named with the node and the part it was written in.
///
/// # Examples
///
/// ```
/// use reef_model::NodeSpec;
/// use reef_render::links::rewrite_spec;
///
/// let mut spec = NodeSpec::from_yaml("id: C1\ndocs: [a.md]\ndecisions: [\"See [b](b.md).\"]\n").unwrap();
/// rewrite_spec(&mut spec, &|dest: &str| Some(format!("plan/{dest}"))).unwrap();
/// assert_eq!(spec.docs, ["plan/a.md"]);
/// assert_eq!(spec.decisions, ["See [b](plan/b.md)."]);
/// ```
pub fn rewrite_spec(
    spec: &mut NodeSpec,
    map: &dyn Fn(&str) -> Option<String>,
) -> Result<(), LinkError> {
    // a doc link is a destination alone, not markdown
    for link in &mut spec.docs {
        if let Some(new) = map(link) {
            *link = new;
        }
    }
    // every part rendered as markdown, each named for an error in it
    let id = spec.id.clone();
    let mut parts: Vec<(String, &mut String)> = Vec::new();
    if let Some(description) = &mut spec.description {
        let named = [
            ("why", &mut description.why),
            ("what", &mut description.what),
            ("details", &mut description.details),
        ];
        for (name, text) in named {
            if let Some(text) = text {
                parts.push((format!("description.{name}"), text));
            }
        }
    }
    for (i, decision) in spec.decisions.iter_mut().enumerate() {
        parts.push((format!("design choice {}", i + 1), decision));
    }
    if let Some(note) = spec
        .performance
        .as_mut()
        .and_then(|performance| performance.note.as_mut())
    {
        parts.push(("performance.note".to_string(), note));
    }
    for (i, criterion) in spec.acceptance.iter_mut().enumerate() {
        parts.push((
            format!("acceptance criterion {}", i + 1),
            &mut criterion.text,
        ));
    }
    if let Some(body) = &mut spec.body {
        parts.push(("body".to_string(), body));
    }
    // rewrite each part, saying where a link that can't be rewritten was written
    for (name, text) in parts {
        *text = rewrite_links(text, map).map_err(|error| LinkError {
            at: format!("`{id}` {name}"),
            ..error
        })?;
    }
    // and each inline member in turn
    for member in &mut spec.nodes {
        rewrite_spec(member, map)?;
    }
    Ok(())
}

/// The destination `map` gives for one parsed, if it differs from it
///
/// # Arguments
///
/// * `map` - The destination to write for each one parsed, if it changes
/// * `dest` - The destination as parsed
fn changed(map: &dyn Fn(&str) -> Option<String>, dest: &str) -> Option<String> {
    map(dest).filter(|new| new != dest)
}

/// Stretches the text of each link still open to reach `end`
///
/// # Arguments
///
/// * `open` - The links and images still open
/// * `end` - The end of something parsed inside them
fn reach(open: &mut [Open], end: usize) {
    for link in open {
        link.text_end = link.text_end.max(end);
    }
}

/// The error for a link whose destination can't be rewritten
///
/// # Arguments
///
/// * `link` - The destination as parsed or written
fn unfound(link: &str) -> LinkError {
    LinkError {
        at: String::new(),
        link: link.to_string(),
    }
}

/// Where an inline link's destination is written, and whether in angle brackets
///
/// # Arguments
///
/// * `md` - The markdown the link is written in
/// * `text_end` - The furthest byte the link's text reaches, before its closing `]`
fn inline_dest(md: &str, text_end: usize) -> Option<(Range<usize>, bool)> {
    // the link's text closes at the next `]`, and its destination opens with `(`
    let close = text_end + md.get(text_end..)?.find(']')?;
    if md.as_bytes().get(close + 1) != Some(&b'(') {
        return None;
    }
    destination(md, blank_end(md, close + 2))
}

/// Where a reference definition's destination is written, and whether in angle brackets
///
/// # Arguments
///
/// * `md` - The markdown the definition is written in
/// * `span` - Where the definition is written
fn definition_dest(md: &str, span: Range<usize>) -> Option<(Range<usize>, bool)> {
    let bytes = md.as_bytes();
    // the label opens with `[` and closes with the first `]` not escaped
    let mut i = span.start + md.get(span.clone())?.find('[')? + 1;
    while i < span.end && bytes[i] != b']' {
        i += if bytes[i] == b'\\' { 2 } else { 1 };
    }
    // then a `:`, and the destination after any blanks
    if bytes.get(i + 1) != Some(&b':') {
        return None;
    }
    destination(md, blank_end(md, i + 2))
}

/// The first byte from `start` that isn't a space, a tab or a line ending
///
/// # Arguments
///
/// * `md` - The markdown to look in
/// * `start` - Where to start looking
fn blank_end(md: &str, start: usize) -> usize {
    let bytes = md.as_bytes();
    let mut i = start;
    while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
        i += 1;
    }
    i
}

/// Where a destination that starts at `start` is written: inside its angle brackets, or the
/// run of text up to a blank or an unbalanced `)`, and whether it is in angle brackets
///
/// # Arguments
///
/// * `md` - The markdown the destination is written in
/// * `start` - Where the destination starts
fn destination(md: &str, start: usize) -> Option<(Range<usize>, bool)> {
    let bytes = md.as_bytes();
    // in angle brackets, up to the first `>` not escaped
    if bytes.get(start) == Some(&b'<') {
        let mut i = start + 1;
        while i < bytes.len() && bytes[i] != b'>' {
            // a line ending can't be in one
            if bytes[i] == b'\n' {
                return None;
            }
            i += if bytes[i] == b'\\' { 2 } else { 1 };
        }
        return (i < bytes.len()).then_some((start + 1..i, true));
    }
    // else up to a blank or a control character, or a `)` that closes no `(` of its own
    let mut i = start;
    let mut depth = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 1,
            b'(' => depth += 1,
            b')' if depth == 0 => break,
            b')' => depth -= 1,
            byte if byte.is_ascii_whitespace() || byte.is_ascii_control() => break,
            _ => {}
        }
        i += 1;
    }
    Some((start..i.min(bytes.len()), false))
}

/// A destination as it is written in place of one that was `angled` or not
///
/// A destination written bare that holds a blank or an angle bracket is written in angle
/// brackets instead. Backslashes, and whatever would end a destination early or be read as an
/// entity or a table's cell break, are escaped.
///
/// # Arguments
///
/// * `dest` - The destination to write
/// * `angled` - Whether it is written inside angle brackets already
fn written(dest: &str, angled: bool) -> String {
    // a bare destination ends at a blank, and can't hold an angle bracket
    let needs_angles = dest
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || c == '<' || c == '>');
    let (inside, open, close) = match (angled, needs_angles) {
        (true, _) => (true, "", ""),
        (false, true) => (true, "<", ">"),
        (false, false) => (false, "", ""),
    };
    // the characters that would end it early, or be read as something else
    let mut out = String::with_capacity(dest.len() + 2);
    out.push_str(open);
    for c in dest.chars() {
        let escape = match c {
            '\\' | '&' | '|' => true,
            '<' | '>' => inside,
            '(' | ')' => !inside,
            _ => false,
        };
        if escape {
            out.push('\\');
        }
        out.push(c);
    }
    out.push_str(close);
    out
}

/// Checks that rewritten markdown parses as the old did, with only its destinations changed
///
/// # Arguments
///
/// * `old` - The markdown as written
/// * `new` - The markdown rewritten
/// * `map` - The destination to write for each one parsed, if it changes
fn check(old: &str, new: &str, map: &dyn Fn(&str) -> Option<String>) -> Result<(), LinkError> {
    // the old markdown's events, and each with its destination as it was to be written
    let original: Vec<Event<'static>> = Parser::new_ext(old, md_options())
        .map(Event::into_static)
        .collect();
    let expected: Vec<Event<'static>> = original
        .iter()
        .cloned()
        .map(|event| mapped(event, map))
        .collect();
    let got: Vec<Event<'static>> = Parser::new_ext(new, md_options())
        .map(Event::into_static)
        .collect();
    if expected == got {
        return Ok(());
    }
    // name the last link as written before the first event that differs, else the first there is
    let differs = expected
        .iter()
        .zip(&got)
        .position(|(a, b)| a != b)
        .unwrap_or(expected.len().min(got.len()));
    let dest = |event: &Event<'_>| match event {
        Event::Start(Tag::Link { dest_url, .. }) | Event::Start(Tag::Image { dest_url, .. }) => {
            Some(dest_url.to_string())
        }
        _ => None,
    };
    let before = original[..differs].iter().rev().find_map(dest);
    let link = before
        .or_else(|| original.iter().find_map(dest))
        .unwrap_or_default();
    Err(unfound(&link))
}

/// An event as it is to read once its destination, if it has one, is rewritten
///
/// # Arguments
///
/// * `event` - The event as parsed
/// * `map` - The destination to write for each one parsed, if it changes
fn mapped(event: Event<'_>, map: &dyn Fn(&str) -> Option<String>) -> Event<'static> {
    // an autolink is left as written, and so is a destination the map leaves
    let to = |link_type: LinkType, dest: &str| match link_type {
        LinkType::Autolink | LinkType::Email => dest.to_string(),
        _ => map(dest).unwrap_or_else(|| dest.to_string()),
    };
    match event {
        Event::Start(Tag::Link {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Link {
            link_type,
            dest_url: to(link_type, &dest_url).into(),
            title: title.into_static(),
            id: id.into_static(),
        }),
        Event::Start(Tag::Image {
            link_type,
            dest_url,
            title,
            id,
        }) => Event::Start(Tag::Image {
            link_type,
            dest_url: to(link_type, &dest_url).into(),
            title: title.into_static(),
            id: id.into_static(),
        }),
        event => event.into_static(),
    }
}

#[cfg(test)]
mod tests {
    use super::{rewrite_links, rewrite_spec};
    use reef_model::NodeSpec;

    /// Every relative destination under `docs/`, as import moves a book's links to its repo's root
    fn docs(dest: &str) -> Option<String> {
        // a URL leads the same place from anywhere
        if dest.contains("://") || dest.starts_with("mailto:") {
            return None;
        }
        Some(format!("docs/{dest}"))
    }

    /// Rewrites markdown under `docs/`, panicking if it can't
    ///
    /// # Arguments
    ///
    /// * `md` - The markdown to rewrite
    fn under_docs(md: &str) -> String {
        rewrite_links(md, &docs).unwrap()
    }

    /// Bare and angled destinations, titles, nested parentheses and escapes each keep their form
    #[test]
    fn inline_destinations_keep_their_form() {
        assert_eq!(under_docs("[a](a.md)"), "[a](docs/a.md)");
        assert_eq!(under_docs("[a](<a b.md> 'T')"), "[a](<docs/a b.md> 'T')");
        assert_eq!(
            under_docs("[a]( a.md \"a.md\" )"),
            "[a]( docs/a.md \"a.md\" )"
        );
        assert_eq!(under_docs("[a](f(1).md)"), "[a](docs/f\\(1\\).md)");
        assert_eq!(under_docs("[a](a\\_b.md)"), "[a](docs/a_b.md)");
        // a bare destination that gains a blank is put in angle brackets
        let spaced = rewrite_links("[a](a.md)", &|_: &str| Some("a b.md".to_string())).unwrap();
        assert_eq!(spaced, "[a](<a b.md>)");
        // text with brackets and code in it, and an empty destination
        assert_eq!(
            under_docs("[a `]` [b] c](a.md)"),
            "[a `]` [b] c](docs/a.md)"
        );
        assert_eq!(under_docs("[a]()"), "[a](docs/)");
    }

    /// An image inside a link is rewritten, and so is the link around it
    #[test]
    fn images_and_the_links_around_them() {
        assert_eq!(under_docs("![i](i.png)"), "![i](docs/i.png)");
        assert_eq!(
            under_docs("[![i](i.png)](a.md#x)"),
            "[![i](docs/i.png)](docs/a.md#x)"
        );
    }

    /// Reference, collapsed and shortcut links are rewritten at their definitions, even unused ones
    #[test]
    fn reference_definitions() {
        let md = "[a][r], [r][] and [r].\n\n[r]: r.md \"T\"\n[unused]:\n  <u v.md>\n";
        let out = under_docs(md);
        assert_eq!(
            out,
            "[a][r], [r][] and [r].\n\n[r]: docs/r.md \"T\"\n[unused]:\n  <docs/u v.md>\n"
        );
    }

    /// Code, fences and autolinks hold no link to rewrite, and a URL is left as written
    #[test]
    fn code_autolinks_and_urls_left_alone() {
        let md = "`[a](a.md)`\n\n```reef\npage = \"plan.md\"\n```\n\n<https://w.org> <a@b.org> [w](https://w.org)\n";
        assert_eq!(under_docs(md), md);
    }

    /// A link in a table's cell keeps the cell whole
    #[test]
    fn links_in_table_cells() {
        let md = "| a | b |\n| --- | --- |\n| [x](x.md) | y |\n";
        assert_eq!(
            under_docs(md),
            "| a | b |\n| --- | --- |\n| [x](docs/x.md) | y |\n"
        );
        // a destination that gains a `|` escapes it, so the cell isn't split
        let piped = rewrite_links(md, &|_: &str| Some("a|b.md".to_string())).unwrap();
        assert_eq!(piped, "| a | b |\n| --- | --- |\n| [x](a\\|b.md) | y |\n");
    }

    /// An anchor alone is a destination like any other
    #[test]
    fn anchors_alone() {
        assert_eq!(under_docs("See [x](#x)."), "See [x](docs/#x).");
    }

    /// Every part of a node rendered as markdown is rewritten, members too, but not a test's path
    #[test]
    fn every_part_of_a_spec() {
        let yaml = "id: g\ndocs: [a.md#h]\ndescription: { why: \"[w](w.md)\", what: \"[x](x.md)\", details: \"[d](d.md)\" }\n\
                    decisions: [\"[c](c.md)\"]\nperformance: { note: \"[n](n.md)\", metrics: [] }\n\
                    acceptance:\n  - unit: { text: \"[t](t.md)\", test: tests/t.rs }\n\
                    nodes:\n  - { id: m, docs: m.md, body: \"# M\\n\\nSee [b](b.md).\\n\" }\n";
        let mut spec = NodeSpec::from_yaml(yaml).unwrap();
        rewrite_spec(&mut spec, &docs).unwrap();
        assert_eq!(spec.docs, ["docs/a.md#h"]);
        let description = spec.description.as_ref().unwrap();
        assert_eq!(description.why.as_deref(), Some("[w](docs/w.md)"));
        assert_eq!(description.what.as_deref(), Some("[x](docs/x.md)"));
        assert_eq!(description.details.as_deref(), Some("[d](docs/d.md)"));
        assert_eq!(spec.decisions, ["[c](docs/c.md)"]);
        assert_eq!(
            spec.performance.as_ref().unwrap().note.as_deref(),
            Some("[n](docs/n.md)")
        );
        assert_eq!(spec.acceptance[0].text, "[t](docs/t.md)");
        assert_eq!(spec.acceptance[0].test.as_deref(), Some("tests/t.rs"));
        // a member's links and body
        assert_eq!(spec.nodes[0].docs, ["docs/m.md"]);
        assert_eq!(
            spec.nodes[0].body.as_deref(),
            Some("# M\n\nSee [b](docs/b.md).\n")
        );
    }

    /// A link that can't be rewritten is named with the node and the part it was written in
    #[test]
    fn a_link_that_cant_be_rewritten_is_named() {
        // inside a block quote, a definition's destination on its next line sits after `> `
        let mut spec =
            NodeSpec::from_yaml("id: C1\ndecisions: [\"> [r]:\\n> r.md\\n\\n[r]\"]\n").unwrap();
        let error = rewrite_spec(&mut spec, &docs).unwrap_err();
        assert_eq!(
            error.to_string(),
            "`C1` design choice 1: the link `r.md` couldn't be rewritten"
        );
    }
}
