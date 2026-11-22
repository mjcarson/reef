//! Book-relative paths and links.

/// A link target resolved against the file that wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// Left as written: a URL, a mail link, an absolute path.
    External(String),
    /// A file in the book, relative to `src` with `/` separators, and the anchor after it.
    Internal {
        /// The file, relative to `src`
        path: String,
        /// The heading id after the `#`, if any
        anchor: Option<String>,
    },
}

/// Resolve `link`, as written in `from` (relative to `src`).
///
/// # Arguments
///
/// * `from` - The path the link is written in
/// * `link` - The link as written
pub fn resolve(from: &str, link: &str) -> Target {
    let link = link.trim();
    if link.contains("://") || link.starts_with("mailto:") || link.starts_with('/') || link.starts_with("data:") {
        return Target::External(link.to_string());
    }
    let (path, anchor) = match link.split_once('#') {
        Some((p, a)) => (p, Some(a.to_string()).filter(|a| !a.is_empty())),
        None => (link, None),
    };
    if path.is_empty() {
        return Target::Internal { path: from.to_string(), anchor };
    }
    Target::Internal { path: join(parent(from), path), anchor }
}

/// The directory part of a `src`-relative path.
///
/// # Arguments
///
/// * `path` - The path to take the directory of
pub fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |i| &path[..i])
}

/// Join and normalise `.` and `..` in `/`-separated paths. A `..` past the root stays.
///
/// # Arguments
///
/// * `base` - The directory to start from
/// * `rel` - The path to join onto it
pub fn join(base: &str, rel: &str) -> String {
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rel.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The page mdbook writes for a markdown file.
///
/// # Arguments
///
/// * `path` - The markdown file's path
pub fn html_of(path: &str) -> String {
    let (dir, file) = match path.rfind('/') {
        Some(i) => (&path[..=i], &path[i + 1..]),
        None => ("", path),
    };
    let file = if file.eq_ignore_ascii_case("README.md") {
        "index.html".to_string()
    } else if let Some(stem) = file.strip_suffix(".md") {
        format!("{stem}.html")
    } else {
        file.to_string()
    };
    format!("{dir}{file}")
}

/// The markdown files mdbook writes a page from, as [`html_of`] names it, the likeliest first:
/// `a/b.html` is written from `a/b.md`, and `a/index.html` from `a/index.md` or a `README.md`.
///
/// # Arguments
///
/// * `page` - The page's path
///
/// # Examples
///
/// ```
/// use reef_render::paths::md_of;
///
/// assert_eq!(md_of("design/app.html"), ["design/app.md"]);
/// assert!(md_of("design/app.png").is_empty());
/// ```
pub fn md_of(page: &str) -> Vec<String> {
    let (dir, file) = match page.rfind('/') {
        Some(i) => (&page[..=i], &page[i + 1..]),
        None => ("", page),
    };
    // only a page is written from markdown
    let Some(stem) = file.strip_suffix(".html") else { return Vec::new() };
    let mut out = vec![format!("{dir}{stem}.md")];
    // and a directory's index from its README too, however its name is cased
    if stem == "index" {
        out.extend(["README.md", "readme.md"].map(|readme| format!("{dir}{readme}")));
    }
    out
}

/// `../` once for each directory a page is nested in.
///
/// # Arguments
///
/// * `page` - The page's path from the book's root
pub fn prefix_for(page: &str) -> String {
    "../".repeat(page.matches('/').count())
}

/// A relative path from directory `from_dir` to `to`, both relative to the same root.
///
/// # Arguments
///
/// * `from_dir` - The directory the path starts from
/// * `to` - The path it leads to
pub fn relative(from_dir: &str, to: &str) -> String {
    let a: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let b: Vec<&str> = to.split('/').filter(|s| !s.is_empty()).collect();
    let common = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let mut out: Vec<&str> = std::iter::repeat_n("..", a.len() - common).collect();
    out.extend_from_slice(&b[common..]);
    out.join("/")
}

/// A link written in `from` as it would be written in a file of the directory `to_dir`, so it
/// leads to the same file and heading from there. Both are relative to the same root.
///
/// A URL, a mail link or an absolute path leads to the same place from anywhere, and so is
/// `None`: left as written. So is a link that names no file, such as `#x` written from the
/// root itself, since there is no file for it to name from elsewhere. A link to a directory
/// keeps the trailing `/` that says it is one.
///
/// # Arguments
///
/// * `from` - The file the link is written in
/// * `to_dir` - The directory of the file it is to be written in
/// * `link` - The link as written
///
/// # Examples
///
/// ```
/// use reef_render::paths::moved;
///
/// // a link in a book's file, as a link from its repo's root
/// let rooted = moved("plan/src/m/a.md", "", "../d/x.md#h");
/// assert_eq!(rooted.as_deref(), Some("plan/src/d/x.md#h"));
/// // and from the root back into an exported book's `src`
/// let back = moved("", "out/src", "plan/src/d/x.md#h");
/// assert_eq!(back.as_deref(), Some("../../plan/src/d/x.md#h"));
/// // a URL is left as written
/// assert_eq!(moved("plan/src/a.md", "", "https://x.org"), None);
/// ```
pub fn moved(from: &str, to_dir: &str, link: &str) -> Option<String> {
    // only a link to a file of the root's leads elsewhere once it is moved
    let Target::Internal { path, anchor } = resolve(from, link) else { return None };
    // a link from the root to the root itself names no file
    if path.is_empty() {
        return None;
    }
    // the path from the new file's directory, which is `.` when it is that directory
    let rel = match relative(to_dir, &path) {
        rel if rel.is_empty() => ".".to_string(),
        rel => rel,
    };
    // a link to a directory keeps the slash that says it is one
    let rel = match link.trim().split('#').next().is_some_and(|path| path.ends_with('/')) {
        true => format!("{rel}/"),
        false => rel,
    };
    // with the heading it named, if it named one
    Some(match anchor {
        Some(anchor) => format!("{rel}#{anchor}"),
        None => rel,
    })
}

/// The id mdbook gives a heading (`id_from_content` in mdbook-html): lowercase, alphanumerics,
/// `_` and `-` kept, each whitespace character a `-`, everything else dropped.
///
/// # Arguments
///
/// * `text` - The heading's text
pub fn heading_id(text: &str) -> String {
    text.trim()
        .to_lowercase()
        .chars()
        .filter_map(|c| {
            if c.is_alphanumeric() || c == '_' || c == '-' {
                Some(c)
            } else if c.is_whitespace() {
                Some('-')
            } else {
                None
            }
        })
        .collect()
}

/// Escape text for HTML content and attribute values.
///
/// # Arguments
///
/// * `s` - The text to escape
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A link moved to the root and back leads where it did, whatever form it was written in
    #[test]
    fn moved_links_lead_where_they_did() {
        // a heading of the file itself, a sibling, a climb out of `src` and a directory
        let file = "book/src/plan/a.md";
        assert_eq!(moved(file, "", "#x").as_deref(), Some("book/src/plan/a.md#x"));
        assert_eq!(moved(file, "", "b.md").as_deref(), Some("book/src/plan/b.md"));
        assert_eq!(moved(file, "", "../../../README.md").as_deref(), Some("README.md"));
        assert_eq!(moved(file, "", "./").as_deref(), Some("book/src/plan/"));
        assert_eq!(moved(file, "", "../../../out/x/#y").as_deref(), Some("out/x/#y"));
        assert_eq!(moved("", "book/src", "book/src/").as_deref(), Some("./"));
        // a link above the root keeps its climb, for the repo's source to refuse
        assert_eq!(moved("a.md", "", "../x.md").as_deref(), Some("../x.md"));
        // what leads the same place from anywhere is left as written
        for link in ["https://x.org/a.md", "mailto:a@b.org", "/abs.md", "data:x"] {
            assert_eq!(moved(file, "", link), None, "{link}");
        }
        // from the root, a heading of no file is left as written, and the directory itself is `.`
        assert_eq!(moved("", "out/src", "#x"), None);
        assert_eq!(moved("", "out/src", "out/src").as_deref(), Some("."));
        // moved into another file and back to the root, each link is as it was rooted
        for link in ["#x", "b.md#y", "../../../README.md", "../d/c.md", "./", "../../../out/x/"] {
            let rooted = moved(file, "", link).unwrap();
            let exported = moved("", "out/src", &rooted).unwrap();
            assert_eq!(moved("out/src/001-a.md", "", &exported).as_deref(), Some(rooted.as_str()), "{link}");
        }
    }

    #[test]
    fn resolves() {
        assert_eq!(
            resolve("future-work/scans/a.md", "../../features/b.md#x"),
            Target::Internal { path: "features/b.md".into(), anchor: Some("x".into()) }
        );
        assert_eq!(resolve("a/b.md", "#top"), Target::Internal { path: "a/b.md".into(), anchor: Some("top".into()) });
        assert!(matches!(resolve("a.md", "https://x.y/z"), Target::External(_)));
    }

    #[test]
    fn pages() {
        assert_eq!(html_of("a/README.md"), "a/index.html");
        assert_eq!(html_of("a/b.md"), "a/b.html");
        // each page leads back to the markdown it is written from
        for md in ["a/b.md", "a/README.md", "index.md"] {
            assert!(md_of(&html_of(md)).contains(&md.to_string()), "{md}");
        }
        assert_eq!(md_of("a/index.html"), ["a/index.md", "a/README.md", "a/readme.md"]);
        assert_eq!(prefix_for("a/b/c.md"), "../../");
        assert_eq!(relative("future-work/scans/plan", "features/f.md"), "../../../features/f.md");
        assert_eq!(heading_id("M23. The walk, rebuilt"), "m23-the-walk-rebuilt");
    }
}
