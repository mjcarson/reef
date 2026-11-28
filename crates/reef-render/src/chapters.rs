//! A plan's docs read as a book: a folder or a summary, over any source of docs (C122)
//!
//! - **A folder is a book.** Its `README.md` or `index.md` is its own page and comes first, then
//!   its other markdown files by name, then its folders by name. Each folder is a section headed
//!   by its own page, else titled by its name, and a folder that holds no markdown is left out.
//! - **A summary orders a folder.** A folder that holds a `SUMMARY.md`, at any depth, is read by
//!   that summary in mdbook's format rather than by name: a title, prefix chapters, numbered
//!   chapters nested as lists with drafts as `[x]()`, part titles, separators, and suffix
//!   chapters. Its links resolve from the summary's folder.
//! - **Titles come from the docs.** A folder's chapter is titled by its doc's first heading,
//!   else its file's name; a summary's by the text it links it with.
//! - **Numbered as mdbook numbers them.** A summary's numbered chapters get mdbook's section
//!   numbers, which run on across parts. Prefix, suffix and folder chapters get none.
//! - **Problems hold still.** Each has no line and the same words every time, since a write is
//!   judged by the problems it adds, compared as check prints them. A chapter a summary lists
//!   that isn't there is a problem, and is read as a draft.
//! - **A plan that names no docs** reads its root node, then its pages by path under `Pages`,
//!   as export's fresh summary lists them ([`plan_book`]).

use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};
use reef_model::spec::first_heading;
use reef_model::{Book, BookFrom, BookItem, Chapter, Diagnostic};

use crate::markdown::md_options;
use crate::paths::{self, Target};
use crate::source::{DocSource, Entry, MemSource, doc_body, leaves};

/// Where a problem with the docs a plan names is said to be found
const NAMED: &str = "the plan's docs";

/// The names a folder's own page is read from, the likeliest first
const INDEXES: [&str; 3] = ["README.md", "readme.md", "index.md"];

/// The file that orders a folder in mdbook's format
const SUMMARY: &str = "SUMMARY.md";

/// The title mdbook's own summaries open with, which names no section
const SUMMARY_TITLE: &str = "Summary";

/// The title of the part a plan that names no docs lists its pages under
pub const PAGES_PART: &str = "Pages";

/// How deep a folder book is read, so a symlink to a folder above it never reads forever
const FOLDER_DEPTH: usize = 8;

/// The path a plan's docs are named by, as it is kept: `.`, `..` and empty parts worked out and
/// a trailing `/` dropped, so the root is the empty path, or a path that leaves the repo as
/// written, kept as written so it is refused as one
///
/// # Arguments
///
/// * `named` - The path as it was named
///
/// # Examples
///
/// ```
/// use reef_render::chapters::docs_path;
///
/// assert_eq!(docs_path("docs/"), "docs");
/// assert_eq!(docs_path("./docs/./src/SUMMARY.md"), "docs/src/SUMMARY.md");
/// assert_eq!(docs_path("."), "");
/// assert_eq!(docs_path("/etc"), "/etc");
/// ```
pub fn docs_path(named: &str) -> String {
    // drop any whitespace around the path as it was typed
    let named = named.trim();
    // keep a path that leaves the repo as written so it is refused, rather than working it out
    // to one that doesn't leave
    match leaves(named) {
        true => named.to_string(),
        false => paths::join("", named),
    }
}

/// How the docs a plan names are shown to a person: a folder with the `/` that says it is one,
/// the root as `./`
///
/// # Arguments
///
/// * `docs` - The path the docs are kept by
///
/// # Examples
///
/// ```
/// use reef_render::chapters::shown_docs;
///
/// assert_eq!(shown_docs("docs"), "docs/");
/// assert_eq!(shown_docs(""), "./");
/// assert_eq!(shown_docs("docs/SUMMARY.md"), "docs/SUMMARY.md");
/// ```
pub fn shown_docs(docs: &str) -> String {
    // show the root as `./`, a summary as its file, and any other path as a folder with a `/`
    match docs {
        "" => "./".to_string(),
        summary if summary.ends_with(".md") => summary.to_string(),
        folder => format!("{folder}/"),
    }
}

/// Reads the docs a plan names as a book: a summary if the path ends in `.md`, else a folder,
/// with the problems found
///
/// A path that leaves the repo, or names nothing there, reads as an empty book, and so does a
/// folder that holds no markdown, each with its problem. A chapter a summary lists that isn't
/// there is a problem too, and reads as a draft.
///
/// # Arguments
///
/// * `source` - Where the plan's docs are read from, its pages over its repo
/// * `named` - The folder or summary, from the root of the docs
pub fn read_book<S: DocSource>(source: &S, named: &str) -> (Book, Vec<Diagnostic>) {
    // collect the problems found while reading the book
    let mut problems = Vec::new();
    // work out the path as it is kept, the root being the empty path
    let path = docs_path(named);
    // read a path ending in `.md` as a summary and any other as a folder
    let from = match path.ends_with(".md") {
        true => BookFrom::Summary { path: path.clone() },
        false => BookFrom::Folder { path: path.clone() },
    };
    // build the empty book that docs which can't be read answer as
    let empty = |from: BookFrom| Book {
        from,
        title: None,
        items: Vec::new(),
    };
    // refuse a path that leaves the repo since it can't name any of the repo's docs
    if leaves(&path) {
        problems.push(named_problem(format!("`{path}` leaves the repo")));
        return (empty(from), problems);
    }
    // show the path in problems as a person would name it
    let shown = shown_docs(&path);
    match from {
        // read a summary as mdbook reads one
        BookFrom::Summary { .. } => {
            // answer an empty book if the summary isn't there
            let Some(text) = source.read(&path) else {
                problems.push(named_problem(format!("`{shown}` isn't there")));
                return (empty(from), problems);
            };
            // read the summary's title and the chapters it lists
            let (title, items) = read_summary(source, &path, &text, &mut problems);
            (Book { from, title, items }, problems)
        }
        // read a folder by name, or by the summary it holds
        _ => {
            // answer an empty book if the folder isn't there
            if !is_folder(source, &path) {
                problems.push(named_problem(format!("`{shown}` isn't there")));
                return (empty(from), problems);
            }
            // read the folder, then put its own page ahead of the rest of its chapters
            let read = read_folder(source, &path, 0, &mut problems);
            let mut items: Vec<BookItem> = read.index.map(BookItem::Chapter).into_iter().collect();
            items.extend(read.items);
            // flag a folder with no markdown in it since its book would be empty
            if items.is_empty() {
                problems.push(named_problem(format!("`{shown}` holds no markdown")));
            }
            // build the book, titled by the folder's summary if it holds one
            let book = Book {
                from,
                title: read.title,
                items,
            };
            (book, problems)
        }
    }
}

/// A plan's book when it names no docs: its root node, then each of its pages by path under
/// [`PAGES_PART`], as export's fresh summary lists them
///
/// # Arguments
///
/// * `root` - The root node's id and title, if the plan has one
/// * `pages` - The plan's pages
///
/// # Examples
///
/// ```
/// use reef_model::{BookFrom, BookItem};
/// use reef_render::MemSource;
/// use reef_render::chapters::plan_book;
///
/// let pages = MemSource::new().with("design/app.md", "# The app\n");
/// let book = plan_book(Some(("M1", "M1 The plan")), &pages);
/// assert_eq!(book.from, BookFrom::Plan);
/// let titles: Vec<&str> = book.chapters().iter().map(|c| c.title.as_str()).collect();
/// assert_eq!(titles, ["M1 The plan", "The app"]);
/// ```
pub fn plan_book(root: Option<(&str, &str)>, pages: &MemSource) -> Book {
    // collect the book's items in the order they are read
    let mut items = Vec::new();
    // add the root node first, naming its node since its body has no page of its own
    if let Some((id, title)) = root {
        items.push(BookItem::Chapter(Chapter {
            title: title.to_string(),
            node: Some(id.to_string()),
            ..Chapter::default()
        }));
    }
    // sort the pages by path since they are kept in no order
    let mut sorted: Vec<(&str, &str)> = pages.iter().collect();
    sorted.sort();
    // open a part for the pages if there are any
    if !sorted.is_empty() {
        items.push(BookItem::Part {
            title: PAGES_PART.to_string(),
        });
    }
    // add each page as a chapter, titled as export titles it
    for (path, text) in sorted {
        items.push(BookItem::Chapter(Chapter {
            title: title_of(path, text),
            path: Some(path.to_string()),
            ..Chapter::default()
        }));
    }
    // build the book, which says it is the plan's own
    Book {
        from: BookFrom::Plan,
        title: None,
        items,
    }
}

/// Parses a summary in mdbook's format: its title, if it opens with one, and its items
///
/// Prefix and suffix chapters are links in paragraphs outside any list, numbered chapters are a
/// list's items, nested lists their sub-chapters, a `[Draft]()` a chapter with no doc, each
/// `#` heading after the title a part's title, and each `---` a separator. Each link resolves
/// from the summary's folder.
///
/// # Arguments
///
/// * `md` - The summary's markdown, without frontmatter
/// * `at` - The summary's path, from the root of the docs
///
/// # Examples
///
/// ```
/// use reef_render::chapters::summary;
/// use reef_model::BookItem;
///
/// let (title, items) = summary("# Summary\n\n- [A](a.md)\n  - [B]()\n", "docs/SUMMARY.md");
/// assert_eq!(title.as_deref(), Some("Summary"));
/// let BookItem::Chapter(a) = &items[0] else { panic!() };
/// assert_eq!((a.path.as_deref(), a.number.as_slice()), (Some("docs/a.md"), &[1][..]));
/// let BookItem::Chapter(b) = &a.items[0] else { panic!() };
/// assert_eq!((b.path.as_deref(), b.number.as_slice()), (None, &[1, 1][..]));
/// ```
pub fn summary(md: &str, at: &str) -> (Option<String>, Vec<BookItem>) {
    // feed each of the summary's markdown events to a reader in turn
    let mut reader = SummaryReader::new(at);
    for event in Parser::new_ext(md, md_options()) {
        reader.event(event);
    }
    // answer the title and the items the reader built
    (reader.title, reader.items)
}

/// A list item of a summary being read, as the chapter it makes
#[derive(Debug, Default)]
struct OpenItem {
    /// The chapter so far: its number, and the chapters nested under it
    chapter: Chapter,
    /// The destination of the item's first link, once it has one
    dest: Option<String>,
    /// Whether that link's text is being read
    in_link: bool,
    /// The link's text so far
    link_text: String,
    /// The item's text before any link, for an item with none
    plain: String,
}

/// A summary read an event at a time
struct SummaryReader<'a> {
    /// The summary's path, which its links resolve from
    at: &'a str,
    /// Its title, if it opened with one
    title: Option<String>,
    /// Whether anything has been read yet, so only an opening heading is its title
    begun: bool,
    /// Its items so far, at the top level
    items: Vec<BookItem>,
    /// The last top-level number given, which runs on across parts
    root: u32,
    /// Each list open, innermost last, with the chapters read into it so far
    lists: Vec<Vec<BookItem>>,
    /// Each list item open, innermost last
    open: Vec<OpenItem>,
    /// The heading being read, its level and its text so far
    heading: Option<(HeadingLevel, String)>,
    /// The link being read outside any list, as a prefix or suffix chapter: its destination and
    /// its text so far
    loose: Option<(String, String)>,
}

impl<'a> SummaryReader<'a> {
    /// A reader of the summary at a path
    ///
    /// # Arguments
    ///
    /// * `at` - The summary's path, which its links resolve from
    fn new(at: &'a str) -> Self {
        SummaryReader {
            at,
            title: None,
            begun: false,
            items: Vec::new(),
            root: 0,
            lists: Vec::new(),
            open: Vec::new(),
            heading: None,
            loose: None,
        }
    }

    /// Reads one event of the summary's markdown
    ///
    /// # Arguments
    ///
    /// * `event` - The event
    fn event(&mut self, event: Event<'_>) {
        match event {
            // open a new list to collect the numbered chapters it holds
            Event::Start(Tag::List(_)) => self.lists.push(Vec::new()),
            // close the list, nesting its chapters under the item it is in or adding them at the
            // top level when it is in none
            Event::End(TagEnd::List(_)) => {
                let list = self.lists.pop().unwrap_or_default();
                match self.open.last_mut() {
                    Some(parent) => parent.chapter.items.extend(list),
                    None => self.push(list),
                }
            }
            // open a chapter for the item, numbered under its parent or on from the last one at
            // the top level
            Event::Start(Tag::Item) => {
                let number = self.next_number();
                self.open.push(OpenItem {
                    chapter: Chapter {
                        number,
                        ..Chapter::default()
                    },
                    ..OpenItem::default()
                });
            }
            // close the item, adding its chapter to the list it is in, or the top level if none
            Event::End(TagEnd::Item) => {
                if let Some(open) = self.open.pop() {
                    let chapter = self.finished(open);
                    match self.lists.last_mut() {
                        Some(list) => list.push(BookItem::Chapter(chapter)),
                        None => self.push(vec![BookItem::Chapter(chapter)]),
                    }
                }
            }
            // start reading a link, which names the doc of the item it is in or is a prefix or
            // suffix chapter when it is outside any list
            Event::Start(Tag::Link { dest_url, .. }) => match self.open.last_mut() {
                // take an item's first link as its doc and read its text as the title
                Some(open) if open.dest.is_none() => {
                    open.dest = Some(dest_url.to_string());
                    open.in_link = true;
                }
                // ignore any later link in an item that already names its doc
                Some(_) => {}
                // start a prefix or suffix chapter for a link outside any list or heading
                None if self.heading.is_none() => {
                    self.loose = Some((dest_url.to_string(), String::new()));
                }
                // ignore a link in a heading
                None => {}
            },
            // stop reading a link's text
            Event::End(TagEnd::Link) => match self.open.last_mut() {
                // stop adding text to the item's title
                Some(open) => open.in_link = false,
                // add a finished prefix or suffix chapter at the top level
                None => {
                    if let Some((dest, text)) = self.loose.take() {
                        let chapter = chapter_at(self.at, text, Some(dest), Vec::new());
                        self.push(vec![BookItem::Chapter(chapter)]);
                    }
                }
            },
            // start reading a heading's text when it is outside any list item
            Event::Start(Tag::Heading { level, .. }) if self.open.is_empty() => {
                self.heading = Some((level, String::new()));
            }
            // finish a top-level heading, which titles the summary if it opens it and titles a
            // part otherwise, while headings of any other level are dropped
            Event::End(TagEnd::Heading(_)) => {
                if let Some((level, text)) = self.heading.take()
                    && level == HeadingLevel::H1
                {
                    let text = text.trim().to_string();
                    match self.begun {
                        // take the opening heading as the summary's title
                        false => {
                            self.title = Some(text);
                            self.begun = true;
                        }
                        // add any later heading as a part's title
                        true => self.push(vec![BookItem::Part { title: text }]),
                    }
                }
            }
            // add a rule between chapters as a separator
            Event::Rule if self.open.is_empty() => self.push(vec![BookItem::Separator]),
            // add text and code to whatever is being read
            Event::Text(text) | Event::Code(text) => self.text(&text),
            // read a line break as a space
            Event::SoftBreak | Event::HardBreak => self.text(" "),
            // ignore every other event since it shapes nothing in the book
            _ => {}
        }
    }

    /// Adds items at the top level
    ///
    /// # Arguments
    ///
    /// * `items` - The items
    fn push(&mut self, items: Vec<BookItem>) {
        // mark the summary as begun so any heading after this titles a part
        self.begun = true;
        // add the items at the top level
        self.items.extend(items);
    }

    /// The number the next list item takes: on from the last at the top level, or its parent's
    /// with its place among its siblings added
    fn next_number(&mut self) -> Vec<u32> {
        match self.open.last() {
            // number a top-level chapter on from the last one, across lists and parts
            None => {
                self.root += 1;
                vec![self.root]
            }
            // number a nested chapter after its parent and the chapters before it in its list
            Some(parent) => {
                // count the chapters already read into this list
                let before = self.lists.last().map_or(0, |list| {
                    list.iter()
                        .filter(|item| matches!(item, BookItem::Chapter(_)))
                        .count()
                });
                // add this chapter's place in the list to its parent's number
                let mut number = parent.chapter.number.clone();
                number.push(before as u32 + 1);
                number
            }
        }
    }

    /// Adds text to whatever is being read: a heading, an item's link or its text, or a loose
    /// link
    ///
    /// # Arguments
    ///
    /// * `text` - The text
    fn text(&mut self, text: &str) {
        // add the text to a heading being read
        if let Some((_, heading)) = &mut self.heading {
            heading.push_str(text);
        } else if let Some(open) = self.open.last_mut() {
            // add it to the item's link text, or to its own text while it has no link, since
            // either one may title the item
            if open.in_link {
                open.link_text.push_str(text);
            } else if open.dest.is_none() {
                open.plain.push_str(text);
            }
        } else if let Some((_, loose)) = &mut self.loose {
            // add it to the text of a prefix or suffix chapter's link
            loose.push_str(text);
        }
    }

    /// The chapter an item read makes, its doc resolved from the summary's folder
    ///
    /// # Arguments
    ///
    /// * `open` - The item read
    fn finished(&self, open: OpenItem) -> Chapter {
        // take apart what was read of the item
        let OpenItem {
            chapter,
            dest,
            link_text,
            plain,
            ..
        } = open;
        // title the chapter by its link's text, or by its own text when it has no link
        let title = match dest {
            Some(_) => link_text,
            None => plain,
        };
        // build the chapter with its doc resolved, keeping the chapters nested under it
        let mut finished = chapter_at(self.at, title, dest, chapter.number);
        finished.items = chapter.items;
        finished
    }
}

/// A chapter a summary lists, its doc resolved from the summary's folder, or none for a draft or
/// a link out of the docs
///
/// # Arguments
///
/// * `at` - The summary's path
/// * `title` - The text it is listed by
/// * `dest` - The link's destination, if it has one
/// * `number` - Its section number, if it has one
fn chapter_at(at: &str, title: String, dest: Option<String>, number: Vec<u32>) -> Chapter {
    // resolve the link from the summary's folder, naming no doc for an empty link, which is a
    // draft, or for a link out of the docs
    let path = dest
        .filter(|dest| !dest.trim().is_empty())
        .and_then(|dest| match paths::resolve(at, &dest) {
            Target::Internal { path, .. } if !path.is_empty() => Some(path),
            _ => None,
        });
    // build the chapter, its title trimmed of the space around the text
    Chapter {
        title: title.trim().to_string(),
        path,
        number,
        ..Chapter::default()
    }
}

/// Reads a summary's items, each chapter it lists checked to be there, a missing one a problem
/// read as a draft
///
/// # Arguments
///
/// * `source` - Where the docs are read from
/// * `at` - The summary's path
/// * `text` - The summary as written
/// * `problems` - Where each problem found is added
fn read_summary<S: DocSource>(
    source: &S,
    at: &str,
    text: &str,
    problems: &mut Vec<Diagnostic>,
) -> (Option<String>, Vec<BookItem>) {
    // parse the summary as mdbook does, leaving out any frontmatter
    let (title, mut items) = summary(doc_body(text), at);
    // check that each chapter it lists is there
    check_listed(source, at, &mut items, problems);
    (title, items)
}

/// Checks that each chapter a summary lists is there, depth first, and reads one that isn't as
/// a draft, a problem at the summary
///
/// # Arguments
///
/// * `source` - Where the docs are read from
/// * `at` - The summary's path
/// * `items` - The items it lists
/// * `problems` - Where each problem found is added
fn check_listed<S: DocSource>(
    source: &S,
    at: &str,
    items: &mut [BookItem],
    problems: &mut Vec<Diagnostic>,
) {
    for item in items {
        // skip parts and separators since only a chapter names a doc
        let BookItem::Chapter(chapter) = item else {
            continue;
        };
        // flag a chapter whose doc isn't there and read it as a draft
        if let Some(path) = &chapter.path
            && source.read(path).is_none()
        {
            problems.push(Diagnostic {
                source: at.to_string(),
                line: None,
                message: format!("lists `{path}`, which isn't there"),
            });
            chapter.path = None;
        }
        // check the chapters nested under it too
        check_listed(source, at, &mut chapter.items, problems);
    }
}

/// A folder read as chapters: its own page, if it has one, its other chapters, and the title a
/// summary it holds gives it
#[derive(Debug, Default)]
struct Folder {
    /// The title its summary gives it, if it is read by one
    title: Option<String>,
    /// Whether it is read by a summary it holds
    summarized: bool,
    /// Its own page, its `README.md` or `index.md`
    index: Option<Chapter>,
    /// Its other chapters, in order
    items: Vec<BookItem>,
}

/// Reads a folder as chapters: by the summary it holds, else its own page, its other markdown
/// files by name and its folders by name
///
/// # Arguments
///
/// * `source` - Where the docs are read from
/// * `dir` - The folder's path, empty for the root
/// * `depth` - How many folders deep it is from the one the plan names
/// * `problems` - Where each problem found is added
fn read_folder<S: DocSource>(
    source: &S,
    dir: &str,
    depth: usize,
    problems: &mut Vec<Diagnostic>,
) -> Folder {
    // list what the folder holds
    let entries = source.list(dir);
    // read the folder by its summary if it holds one, instead of by name
    if entries.contains(&Entry::file(SUMMARY)) {
        let at = paths::join(dir, SUMMARY);
        if let Some(text) = source.read(&at) {
            // read the summary's title and the chapters it lists
            let (title, items) = read_summary(source, &at, &text, problems);
            return Folder {
                title,
                summarized: true,
                index: None,
                items,
            };
        }
    }
    // find the folder's own page, trying the likeliest name first
    let index = INDEXES
        .into_iter()
        .find(|name| entries.contains(&Entry::file(*name)));
    // start the folder with its own page as its index
    let mut folder = Folder {
        index: index.map(|name| chapter_of(source, &paths::join(dir, name))),
        ..Folder::default()
    };
    // add each of its other markdown files by name
    for entry in &entries {
        if !entry.folder && entry.name.ends_with(".md") && Some(entry.name.as_str()) != index {
            let chapter = chapter_of(source, &paths::join(dir, &entry.name));
            folder.items.push(BookItem::Chapter(chapter));
        }
    }
    // add each of its folders by name as a section, unless it is too deep or holds no markdown
    if depth < FOLDER_DEPTH {
        for entry in entries.iter().filter(|entry| entry.folder) {
            let sub = paths::join(dir, &entry.name);
            if let Some(section) = section(source, &sub, &entry.name, depth + 1, problems) {
                folder.items.push(BookItem::Chapter(section));
            }
        }
    }
    folder
}

/// A folder read as a section: headed by its own page, else titled by its summary's title or
/// its name, or `None` for a folder that holds no markdown
///
/// # Arguments
///
/// * `source` - Where the docs are read from
/// * `dir` - The folder's path
/// * `name` - The folder's name
/// * `depth` - How many folders deep it is from the one the plan names
/// * `problems` - Where each problem found is added
fn section<S: DocSource>(
    source: &S,
    dir: &str,
    name: &str,
    depth: usize,
    problems: &mut Vec<Diagnostic>,
) -> Option<Chapter> {
    // read the folder's own chapters
    let folder = read_folder(source, dir, depth, problems);
    // leave out a folder with no markdown in it
    if folder.index.is_none() && folder.items.is_empty() {
        return None;
    }
    // head the section by the folder's own page, else title it by its summary's title or by
    // its name when that title is mdbook's default
    let mut chapter = folder.index.unwrap_or_else(|| Chapter {
        title: folder
            .title
            .filter(|title| folder.summarized && title != SUMMARY_TITLE)
            .unwrap_or_else(|| name.to_string()),
        ..Chapter::default()
    });
    // nest the folder's other chapters under it
    chapter.items = folder.items;
    Some(chapter)
}

/// A doc's chapter in a folder book, titled by its first heading, else its file's name
///
/// # Arguments
///
/// * `source` - Where the docs are read from
/// * `path` - The doc's path
fn chapter_of<S: DocSource>(source: &S, path: &str) -> Chapter {
    // read the doc, leaving it empty if it can't be read so it is titled by its name alone
    let text = source.read(path).unwrap_or_default();
    // build the chapter titled by the doc's first heading
    Chapter {
        title: title_of(path, &text),
        path: Some(path.to_string()),
        ..Chapter::default()
    }
}

/// The title a doc is listed by: its first heading, its frontmatter left out, else its file's
/// name without `.md`
///
/// # Arguments
///
/// * `path` - The doc's path
/// * `text` - The doc as written
fn title_of(path: &str, text: &str) -> String {
    // get the doc's file name from its path
    let stem = path.rsplit('/').next().unwrap_or(path);
    // title it by its first heading, else by its file name without `.md`
    first_heading(doc_body(text)).unwrap_or_else(|| stem.trim_end_matches(".md").to_string())
}

/// Whether a path names a folder of a source: the root always, else a folder its parent lists
///
/// # Arguments
///
/// * `source` - Where the docs are read from
/// * `dir` - The folder's path
fn is_folder<S: DocSource>(source: &S, dir: &str) -> bool {
    // treat the root as always there
    if dir.is_empty() {
        return true;
    }
    // check that the folder's parent lists it as a folder
    let (parent, name) = dir.rsplit_once('/').unwrap_or(("", dir));
    source.list(parent).contains(&Entry::folder(name))
}

/// A problem with the docs a plan names, at no line, so it reads the same every time
///
/// # Arguments
///
/// * `message` - What the problem is
fn named_problem(message: String) -> Diagnostic {
    // place the problem at the plan's docs with no line
    Diagnostic {
        source: NAMED.to_string(),
        line: None,
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::{plan_book, read_book, summary};
    use crate::source::MemSource;
    use reef_model::{Book, BookFrom, BookItem, Chapter};

    /// Each item of a list as a line: a chapter by its number, title and doc, indented by its
    /// depth, a part by `# title`, and a separator as `---`
    ///
    /// # Arguments
    ///
    /// * `items` - The items
    fn outline(items: &[BookItem]) -> Vec<String> {
        /// Adds each item's lines at a depth
        ///
        /// # Arguments
        ///
        /// * `items` - The items
        /// * `depth` - How deep they are
        /// * `out` - Where each line is added
        fn walk(items: &[BookItem], depth: usize, out: &mut Vec<String>) {
            for item in items {
                match item {
                    // add a chapter's line, then its own chapters one level deeper
                    BookItem::Chapter(chapter) => {
                        out.push(line(chapter, depth));
                        walk(&chapter.items, depth + 1, out);
                    }
                    // add a part's title as a heading
                    BookItem::Part { title } => out.push(format!("# {title}")),
                    // add a separator as a rule
                    BookItem::Separator => out.push("---".to_string()),
                }
            }
        }
        /// A chapter's line: its number, title and doc, indented by its depth
        ///
        /// # Arguments
        ///
        /// * `chapter` - The chapter
        /// * `depth` - How deep it is
        fn line(chapter: &Chapter, depth: usize) -> String {
            // show the chapter's number followed by a space, or nothing if it has none
            let number = match chapter.number.is_empty() {
                true => String::new(),
                false => format!("{} ", chapter.shown_number()),
            };
            // show its doc's path, or `-` when it names none
            let path = chapter.path.as_deref().unwrap_or("-");
            // indent the line two spaces for each level deep
            format!("{}{number}{} ({path})", "  ".repeat(depth), chapter.title)
        }
        // walk the items from the top level down
        let mut out = Vec::new();
        walk(items, 0, &mut out);
        out
    }

    /// The problems of a book read, as check prints them
    ///
    /// # Arguments
    ///
    /// * `problems` - The problems
    fn printed(problems: &[reef_model::Diagnostic]) -> Vec<String> {
        // print each problem as check would
        problems.iter().map(ToString::to_string).collect()
    }

    /// A folder reads as chapters: its README first, then its files and folders by name, each
    /// folder a section headed by its README or index, else its name, each chapter titled by its
    /// first heading, else its file's name, and a folder with no markdown left out
    #[test]
    fn a_folder_reads_as_chapters() {
        // keep a folder of docs with a README, a dotfile, a guide with an index, a folder with no
        // markdown, a folder ordered by its summary, and a doc outside it
        let docs = MemSource::new()
            .with("docs/README.md", "# The docs\n\nWhat they are for.\n")
            .with("docs/b.md", "# Bee\n")
            .with("docs/a.md", "No heading here.\n")
            .with("docs/.hidden.md", "# Hidden\n")
            .with("docs/guide/setup.md", "---\nid: S1\n---\n# Setting up\n")
            .with("docs/guide/index.md", "# The guide\n")
            .with("docs/notes/z.md", "```\n# not a heading\n```\n\n## Zed\n")
            .with("docs/empty/picture.png", "")
            .with(
                "docs/ordered/SUMMARY.md",
                "# Summary\n\n- [Two](two.md)\n- [One](one.md)\n",
            )
            .with("docs/ordered/one.md", "# One\n")
            .with("docs/ordered/two.md", "# Two\n")
            .with("other.md", "# Not in the docs\n");
        // read the folder, its trailing `/` dropped, and check it has no problems
        let (book, problems) = read_book(&docs, "docs/");
        assert!(problems.is_empty(), "{problems:?}");
        // check the book says it was read from the folder and has no title
        assert_eq!(
            book.from,
            BookFrom::Folder {
                path: "docs".to_string()
            }
        );
        assert_eq!(book.title, None);
        // check the book lists its README first, then its files, then its folders, with the
        // ordered folder read by its summary
        assert_eq!(
            outline(&book.items),
            [
                "The docs (docs/README.md)",
                "a (docs/a.md)",
                "Bee (docs/b.md)",
                "The guide (docs/guide/index.md)",
                "  Setting up (docs/guide/setup.md)",
                "notes (-)",
                "  Zed (docs/notes/z.md)",
                "ordered (-)",
                "  1. Two (docs/ordered/two.md)",
                "  2. One (docs/ordered/one.md)",
            ]
        );
        // read the root as `.` and check it has no problems
        let (root, problems) = read_book(&docs, ".");
        assert!(problems.is_empty(), "{problems:?}");
        // check the root is kept as the empty path
        assert_eq!(
            root.from,
            BookFrom::Folder {
                path: String::new()
            }
        );
        // check the root lists its own file, then the docs folder as a section
        let titles: Vec<String> = outline(&root.items);
        assert_eq!(titles[0], "Not in the docs (other.md)");
        assert_eq!(titles[1], "The docs (docs/README.md)");
        // check the empty path reads the same book as `.`
        assert_eq!(read_book(&docs, "").0, root);
    }

    /// A folder that isn't there, holds no markdown, or leaves the repo is a problem with the
    /// same words every time, and reads as an empty book
    #[test]
    fn a_folder_not_there_is_a_problem() {
        // keep one doc and a folder with no markdown in it
        let docs = MemSource::new()
            .with("docs/a.md", "# A\n")
            .with("docs/empty/picture.png", "");
        // check each path that can't be read gives its one problem and an empty book
        for (named, said) in [
            ("gone", "the plan's docs: `gone/` isn't there"),
            ("docs/b.md", "the plan's docs: `docs/b.md` isn't there"),
            (
                "docs/empty",
                "the plan's docs: `docs/empty/` holds no markdown",
            ),
            ("../docs", "the plan's docs: `../docs` leaves the repo"),
            ("/etc", "the plan's docs: `/etc` leaves the repo"),
        ] {
            let (book, problems) = read_book(&docs, named);
            assert_eq!(printed(&problems), [said], "{named}");
            assert!(book.items.is_empty(), "{named}");
        }
    }

    /// A summary reads as mdbook's format: a title, prefix chapters, numbered chapters nested
    /// and drafted, part titles that leave the numbers running on, separators, and suffix
    /// chapters, its links from its folder, a missing chapter a problem read as a draft
    #[test]
    fn a_summary_reads_as_mdbooks() {
        // write a summary shaped like this repo's own `docs/src/SUMMARY.md`
        let text = "# Summary\n\n\
            [Introduction](introduction.md)\n\n\
            # Plans in a book\n\n\
            - [Setting up a book](book/setup.md)\n\
            - [Writing nodes](book/nodes.md)\n  \
              - [Why and what](book/why-and-what.md)\n  \
              - [A `draft`]()\n\
            - [Drawing a view](book/views.md)\n\n\
            # Central reef\n\n\
            - [What central reef is](central/overview.md)\n\
            - [Deploying](./central/deploy/index.md#top)\n  \
              - [With reefadm](central/deploy/reefadm.md)\n    \
                - [Deeper](central/deploy/deeper.md)\n\n\
            ---\n\n\
            [Contributors](../misc/contributors.md)\n\
            [Away](https://example.com)\n";
        // keep the summary beside every doc it lists except the two meant to be missing
        let docs: MemSource = [
            "introduction.md",
            "book/setup.md",
            "book/nodes.md",
            "book/views.md",
            "central/overview.md",
            "central/deploy/index.md",
            "central/deploy/reefadm.md",
            "central/deploy/deeper.md",
        ]
        .into_iter()
        .map(|path| (format!("docs/src/{path}"), format!("# {path}\n")))
        .chain([("docs/src/SUMMARY.md".to_string(), text.to_string())])
        .collect();
        // read the summary and check the book says where it came from and its title
        let (book, problems) = read_book(&docs, "docs/src/SUMMARY.md");
        assert_eq!(
            book.from,
            BookFrom::Summary {
                path: "docs/src/SUMMARY.md".to_string()
            }
        );
        assert_eq!(book.title.as_deref(), Some("Summary"));
        // check each chapter is numbered as mdbook numbers it and titled by its link, with its
        // doc resolved from the summary's folder and a missing or outside doc read as a draft
        assert_eq!(
            outline(&book.items),
            [
                "Introduction (docs/src/introduction.md)",
                "# Plans in a book",
                "1. Setting up a book (docs/src/book/setup.md)",
                "2. Writing nodes (docs/src/book/nodes.md)",
                "  2.1. Why and what (-)",
                "  2.2. A draft (-)",
                "3. Drawing a view (docs/src/book/views.md)",
                "# Central reef",
                "4. What central reef is (docs/src/central/overview.md)",
                "5. Deploying (docs/src/central/deploy/index.md)",
                "  5.1. With reefadm (docs/src/central/deploy/reefadm.md)",
                "    5.1.1. Deeper (docs/src/central/deploy/deeper.md)",
                "---",
                "Contributors (-)",
                "Away (-)",
            ]
        );
        // check each listed chapter that isn't there is a problem placed at the summary
        assert_eq!(
            printed(&problems),
            [
                "docs/src/SUMMARY.md: lists `docs/src/book/why-and-what.md`, which isn't there",
                "docs/src/SUMMARY.md: lists `docs/misc/contributors.md`, which isn't there",
            ]
        );
        // check that naming the summary's folder reads the folder by the summary rather than
        // by its names, with the same problems
        let (folder, problems) = read_book(&docs, "docs/src");
        assert_eq!(folder.title.as_deref(), Some("Summary"));
        assert_eq!(folder.items, book.items);
        assert_eq!(problems.len(), 2);
        // check that a folder above it reads it as a section titled by the folder's name
        let (above, _) = read_book(&docs, "docs");
        let BookItem::Chapter(section) = &above.items[0] else {
            panic!("{:?}", above.items)
        };
        assert_eq!(
            (section.title.as_str(), section.path.as_deref()),
            ("src", None)
        );
        assert_eq!(section.items, book.items);
        // check a summary with no title opens with what it lists
        let (title, items) = summary("- [A](a.md)\n\n# Part\n\n- [B](b.md)\n", "SUMMARY.md");
        assert_eq!(title, None);
        assert_eq!(outline(&items), ["1. A (a.md)", "# Part", "2. B (b.md)"]);
        // check a summary that isn't there is a problem
        let (_, problems) = read_book(&docs, "docs/GONE.md");
        assert_eq!(
            printed(&problems),
            ["the plan's docs: `docs/GONE.md` isn't there"]
        );
    }

    /// A plan that names no docs reads its root node, then its pages by path under `Pages`, each
    /// titled by its first heading, else its file's name
    #[test]
    fn plan_book_lists_root_then_pages() {
        // keep pages with no heading, a heading, and frontmatter before a heading
        let pages = MemSource::new()
            .with("plan.md", "Words only.\n")
            .with("design/b.md", "# Bee\n")
            .with("design/a.md", "---\nid: x\n---\n# Ay\n");
        // read the plan's book and check it says it is the plan's own
        let book = plan_book(Some(("M1", "M1 The milestone")), &pages);
        assert_eq!(book.from, BookFrom::Plan);
        // check the root node comes first, then each page by path under the pages part
        assert_eq!(
            outline(&book.items),
            [
                "M1 The milestone (-)",
                "# Pages",
                "Ay (design/a.md)",
                "Bee (design/b.md)",
                "plan (plan.md)",
            ]
        );
        // check the root's chapter names its node since it has no page of its own
        let BookItem::Chapter(root) = &book.items[0] else {
            panic!("{:?}", book.items)
        };
        assert_eq!(root.node.as_deref(), Some("M1"));
        // check a plan with no root and no pages has an empty book
        assert_eq!(
            plan_book(None, &MemSource::new()),
            Book {
                from: BookFrom::Plan,
                title: None,
                items: Vec::new()
            }
        );
    }
}
