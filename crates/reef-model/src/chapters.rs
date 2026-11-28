//! A plan's docs read as a book: a tree of chapters, as the service answers them and the app
//! draws them (C122)
//!
//! A plan names its docs as a folder or as a summary in mdbook's format, and reads them over
//! its pages and its repo's files. A plan that names none reads its root node, then its pages by
//! path. Either way the book is a list of [`BookItem`]s: chapters, each of which may hold more,
//! with part titles and separators between them where a summary writes them.

use serde::{Deserialize, Serialize};

/// A plan's docs as a book: where they were read from, the title a summary gives them, and
/// their chapters in order
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Book {
    /// What the book was read from
    pub from: BookFrom,
    /// The title a summary's `# Title` gives it, if it gives one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Its chapters, parts and separators, in order
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<BookItem>,
}

/// What a plan's book was read from
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum BookFrom {
    /// A folder, each of its markdown files and folders a chapter
    Folder {
        /// The folder's path from the root of the plan's docs, empty for the root itself
        path: String,
    },
    /// A summary in mdbook's format, which lists the chapters
    Summary {
        /// The summary's path from the root of the plan's docs
        path: String,
    },
    /// The plan names no docs, so its book is its root node, then its pages
    #[default]
    Plan,
}

/// One item of a book's list: a chapter, a part's title, or a separator
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BookItem {
    /// A chapter, and the chapters under it
    Chapter(Chapter),
    /// A part's title, which heads the numbered chapters after it
    Part {
        /// The title
        title: String,
    },
    /// A line between two runs of chapters
    Separator,
}

/// One chapter of a book: its title, the doc it shows, and the chapters under it
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Chapter {
    /// The title it is listed by
    pub title: String,
    /// The doc it shows, from the root of the plan's docs, or `None` for a draft or a section
    /// with no page of its own
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// The node it shows, by id, for the root node's chapter of a plan that names no docs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Where its doc is served, as the service fills it in
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// Its section number, as mdbook numbers it, or empty for a chapter mdbook doesn't number
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub number: Vec<u32>,
    /// The chapters under it
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<BookItem>,
}

impl Book {
    /// Every chapter of the book, depth first, in the order it lists them
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_model::{Book, BookItem, Chapter};
    ///
    /// let inner = Chapter { title: "B".to_string(), ..Chapter::default() };
    /// let outer = Chapter {
    ///     title: "A".to_string(),
    ///     items: vec![BookItem::Chapter(inner)],
    ///     ..Chapter::default()
    /// };
    /// let book = Book { items: vec![BookItem::Chapter(outer)], ..Book::default() };
    /// let titles: Vec<&str> = book.chapters().iter().map(|c| c.title.as_str()).collect();
    /// assert_eq!(titles, ["A", "B"]);
    /// ```
    pub fn chapters(&self) -> Vec<&Chapter> {
        // collect every chapter depth first, each before the chapters nested under it
        let mut out = Vec::new();
        chapters_of(&self.items, &mut out);
        out
    }

    /// Changes every chapter of the book in place, depth first, in the order it lists them
    ///
    /// # Arguments
    ///
    /// * `change` - What to do to each chapter, before the chapters under it
    pub fn each_chapter_mut<F: FnMut(&mut Chapter)>(&mut self, mut change: F) {
        // change every chapter depth first, each before the chapters nested under it
        each_of_mut(&mut self.items, &mut change);
    }
}

impl Chapter {
    /// The section number as mdbook prints it, such as `1.2.`, or empty for none
    ///
    /// # Examples
    ///
    /// ```
    /// use reef_model::Chapter;
    ///
    /// let chapter = Chapter { number: vec![1, 2], ..Chapter::default() };
    /// assert_eq!(chapter.shown_number(), "1.2.");
    /// assert_eq!(Chapter::default().shown_number(), "");
    /// ```
    pub fn shown_number(&self) -> String {
        // write each part of the number followed by the dot mdbook puts after it
        self.number.iter().map(|part| format!("{part}.")).collect()
    }
}

/// Collects every chapter of a list, depth first
///
/// # Arguments
///
/// * `items` - The list
/// * `out` - Where each chapter is pushed
fn chapters_of<'a>(items: &'a [BookItem], out: &mut Vec<&'a Chapter>) {
    for item in items {
        // skip parts and separators since only a chapter holds chapters
        if let BookItem::Chapter(chapter) = item {
            // add this chapter, then the chapters nested under it
            out.push(chapter);
            chapters_of(&chapter.items, out);
        }
    }
}

/// Changes every chapter of a list in place, depth first
///
/// # Arguments
///
/// * `items` - The list
/// * `change` - What to do to each chapter, before the chapters under it
fn each_of_mut<F: FnMut(&mut Chapter)>(items: &mut [BookItem], change: &mut F) {
    for item in items {
        // skip parts and separators since only a chapter holds chapters
        if let BookItem::Chapter(chapter) = item {
            // change this chapter, then the chapters nested under it
            change(chapter);
            each_of_mut(&mut chapter.items, change);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Book, BookFrom, BookItem, Chapter};
    use serde_json::json;

    /// A book's JSON is pinned, since the app and the CLI read it
    #[test]
    fn a_book_is_kebab_case_and_leaves_out_what_is_unset() {
        // build a summary's book with a part, a separator, and a draft nested under a chapter
        let book = Book {
            from: BookFrom::Summary {
                path: "docs/SUMMARY.md".to_string(),
            },
            title: Some("Docs".to_string()),
            items: vec![
                BookItem::Part {
                    title: "Guide".to_string(),
                },
                BookItem::Chapter(Chapter {
                    title: "Intro".to_string(),
                    path: Some("docs/intro.md".to_string()),
                    page: Some("/x/docs/intro.html".to_string()),
                    number: vec![1],
                    items: vec![BookItem::Chapter(Chapter {
                        title: "Later".to_string(),
                        number: vec![1, 1],
                        ..Chapter::default()
                    })],
                    ..Chapter::default()
                }),
                BookItem::Separator,
            ],
        };
        // check its JSON is kebab case and leaves out every field that is unset
        let value = serde_json::to_value(&book).unwrap();
        assert_eq!(
            value,
            json!({
                "from": {"kind": "summary", "path": "docs/SUMMARY.md"},
                "title": "Docs",
                "items": [
                    {"part": {"title": "Guide"}},
                    {"chapter": {
                        "title": "Intro",
                        "path": "docs/intro.md",
                        "page": "/x/docs/intro.html",
                        "number": [1],
                        "items": [{"chapter": {"title": "Later", "number": [1, 1]}}],
                    }},
                    "separator",
                ],
            })
        );
        // check it reads back as itself
        assert_eq!(serde_json::from_value::<Book>(value).unwrap(), book);
        // check the book of a plan that names no docs says it is read from the plan
        let plan = serde_json::to_value(Book::default()).unwrap();
        assert_eq!(plan, json!({"from": {"kind": "plan"}}));
    }

    /// Every chapter is reached depth first, and can be changed in place
    #[test]
    fn chapters_are_walked_depth_first() {
        // build a book of two chapters with a separator between them, the first holding a third
        let mut book = Book {
            items: vec![
                BookItem::Chapter(Chapter {
                    title: "A".to_string(),
                    items: vec![BookItem::Chapter(Chapter {
                        title: "A1".to_string(),
                        ..Chapter::default()
                    })],
                    ..Chapter::default()
                }),
                BookItem::Separator,
                BookItem::Chapter(Chapter {
                    title: "B".to_string(),
                    ..Chapter::default()
                }),
            ],
            ..Book::default()
        };
        // number each chapter's page in the order it is changed
        let mut n = 0;
        book.each_chapter_mut(|chapter| {
            chapter.page = Some(n.to_string());
            n += 1;
        });
        // check the chapters are walked depth first in that same order
        let walked: Vec<(&str, Option<&str>)> = book
            .chapters()
            .into_iter()
            .map(|chapter| (chapter.title.as_str(), chapter.page.as_deref()))
            .collect();
        assert_eq!(
            walked,
            [("A", Some("0")), ("A1", Some("1")), ("B", Some("2"))]
        );
    }
}
