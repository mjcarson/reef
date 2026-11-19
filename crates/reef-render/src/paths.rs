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
        assert_eq!(prefix_for("a/b/c.md"), "../../");
        assert_eq!(relative("future-work/scans/plan", "features/f.md"), "../../../features/f.md");
        assert_eq!(heading_id("M23. The walk, rebuilt"), "m23-the-walk-rebuilt");
    }
}
