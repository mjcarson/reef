//! Rendering for reef planning graphs, over any source of docs: a node's markdown rendered for
//! the sidebar (`markdown`), its doc links resolved with mdbook's path conventions (`paths`), and
//! its why, what and design choices taken from the docs it links to (`extract`, `docs`).
//!
//! Docs are read through a [`DocSource`]: [`FsSource`] reads a book's files, [`RepoSource`] a
//! repo's working tree, which no link may leave, and [`MemSource`] holds docs in memory, as a
//! server holding a plan's pages would. [`render_page`] renders a doc as the page mdbook would
//! write for it, for a server with no book to serve.

pub mod docs;
pub mod extract;
pub mod markdown;
pub mod paths;
pub mod source;

pub use docs::{Renderer, Sections};
pub use markdown::{HeadingSpan, ROOT, heading_spans, headings_of, inline_html, is_reef, md_options, render, render_body, render_page};
pub use source::{DocSource, FsSource, Headings, MemSource, RepoSource, doc_body};
