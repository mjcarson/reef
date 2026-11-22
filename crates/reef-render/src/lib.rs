//! Rendering for reef planning graphs, over any source of docs: a node's markdown rendered for
//! the sidebar (`markdown`), its doc links resolved with mdbook's path conventions (`paths`), and
//! its why, what and design choices taken from the docs it links to (`extract`, `docs`).
//!
//! Docs are read through a [`DocSource`]: [`FsSource`] reads a book's files, [`RepoSource`] a
//! repo's working tree, which no link may leave, and [`MemSource`] holds docs in memory, as a
//! server holding a plan's pages does. [`Layered`] reads one source over another, as a plan's
//! pages over its repo, and [`Cached`] reads each doc of a source once. [`render_page`] renders
//! a doc as the page mdbook would write for it, for a server with no book to serve.
//!
//! A book's own files are read by [`book`]: its node files, and the plan they make, checked
//! and briefed, as mdbook-reef draws it and `reef import` lifts it into a central plan. The
//! links written in a node are moved with it by [`links`], so they lead where they did.

pub mod book;
pub mod docs;
pub mod extract;
pub mod links;
pub mod markdown;
pub mod paths;
pub mod source;

pub use docs::{Renderer, Sections};
pub use markdown::{HeadingSpan, ROOT, heading_spans, headings_of, inline_html, is_reef, md_options, render, render_body, render_page};
pub use source::{Cached, DocSource, FsSource, Headings, Layered, MemSource, RepoSource, doc_body};
