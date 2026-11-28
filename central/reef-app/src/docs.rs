//! A plan's docs as a book, drawn as mdbook draws one (C123)
//!
//! The chapter bar on the left lists the book's chapters, numbered as mdbook numbers them, each
//! section folding its chapters, the open chapter marked. The chapter is drawn in mdbook's
//! content column, from runs of HTML and views the service answers it as, so a
//! ```` ```reef ```` fence draws as a live view of the plan, and its previous and next chapters
//! follow it. A link to another chapter opens it here, at the heading its anchor names, and a
//! link naming a node opens that node, as the view's owner says. The book and its open chapter
//! are read again after each change to the plan, a page edited or its branch fetched among them.

use dioxus::prelude::*;
use reef_model::{Book, BookItem, Chapter, ChapterText, Layout, Run};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::app::Route;
use crate::calls;
use crate::canvas::{Canvas, Keyed, dom};
use crate::live::use_changes;
use crate::sign_in::{CallFailed, SignedInAs};

/// A chapter the view can open: one of the plan's docs by its path, or a node's body by its id
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    /// One of the plan's docs, by its path from the root of its docs
    Doc(String),
    /// A node's body, as a plan that names no docs opens with its root node's
    Node(String),
}

impl Target {
    /// The chapter a book's entry opens, if it opens one: a draft or a section with no page of
    /// its own opens none
    ///
    /// # Arguments
    ///
    /// * `chapter` - The book's entry
    fn of(chapter: &Chapter) -> Option<Target> {
        // its doc, else its node
        match (&chapter.path, &chapter.node) {
            (Some(path), _) => Some(Target::Doc(path.clone())),
            (None, Some(id)) => Some(Target::Node(id.clone())),
            (None, None) => None,
        }
    }

    /// The route that opens the chapter, at a heading in it
    ///
    /// A node's chapter is only ever the book's first, so it opens at the book's own route.
    ///
    /// # Arguments
    ///
    /// * `project` - The project the plan belongs to
    /// * `plan` - The plan's name within it
    /// * `anchor` - The heading to open it at, or empty for its top
    fn route(&self, project: String, plan: String, anchor: String) -> Route {
        // a doc's path is the route's own, a segment a folder
        let path = match self {
            Target::Doc(path) => path.split('/').map(str::to_string).collect(),
            Target::Node(_) => Vec::new(),
        };
        Route::DocsPage {
            project,
            plan,
            path,
            anchor,
        }
    }
}

/// Every chapter of a book the view can open, in the order the book lists them
///
/// # Arguments
///
/// * `book` - The book
fn openable(book: &Book) -> Vec<(Target, &Chapter)> {
    // each chapter in turn, those with a doc or a node
    book.chapters()
        .into_iter()
        .filter_map(|chapter| Target::of(chapter).map(|target| (target, chapter)))
        .collect()
}

/// A plan's docs, full-window: a bar naming the plan, with a way back to its graph, over the
/// docs view
///
/// A `node:` link in the docs opens the plan's page with that node shown.
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `path` - The open chapter's doc, by its path's segments, or none for the book's first
/// * `anchor` - The heading to open the chapter at, or empty for its top
#[component]
pub fn DocsPage(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    path: ReadSignal<Vec<String>>,
    anchor: ReadSignal<String>,
) -> Element {
    // the plan's changes, which read its book and its open chapter again
    let changes = use_changes(project, plan);
    // the chapter the route opens, by its doc's path, or empty for the book's first
    let open = use_memo(move || path.read().join("/"));
    // a link naming a node opens the plan's page with it shown
    let nav = navigator();
    let on_node = move |id: String| {
        nav.push(Route::PlanPage {
            project: project(),
            plan: plan(),
            node: id,
        });
    };
    let graph = Route::PlanPage {
        project: project(),
        plan: plan(),
        node: String::new(),
    };
    rsx! {
        div { class: "reef-app-plan",
            header { class: "reef-app-bar",
                Link { to: Route::Explorer {}, "← Plans" }
                strong { class: "reef-app-plan-name", "{project}/{plan}" }
                Link { class: "reef-app-pane-link", to: graph, "Graph" }
                SignedInAs {}
            }
            div { class: "reef-app-stage",
                // another plan starts the view afresh, its folded sections with it
                {
                    rsx! {
                        DocsView {
                            key: "{project}/{plan}",
                            project,
                            plan,
                            path: open,
                            anchor,
                            changes,
                            on_node,
                        }
                    }
                }
            }
        }
    }
}

/// A plan's docs as a book: the chapter bar, the open chapter and its previous and next, read
/// again after each change to the plan
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `path` - The open chapter's doc, by its path, or empty for the book's first
/// * `anchor` - The heading to open the chapter at, or empty for its top
/// * `changes` - How many changes to the plan have come, which read the book and chapter again
/// * `on_node` - What a link naming a node does, given the node's id
#[component]
pub fn DocsView(
    project: ReadSignal<String>,
    plan: ReadSignal<String>,
    path: ReadSignal<String>,
    anchor: ReadSignal<String>,
    changes: ReadSignal<u64>,
    on_node: EventHandler<String>,
) -> Element {
    // the plan's book, read in the client after each change, the last one read standing meanwhile
    let book = use_resource(move || async move {
        let _ = changes();
        calls::book(project(), plan()).await
    });
    // the chapter open: the one the route names, else the book's first it can open
    let target = use_memo(move || {
        let path = path();
        if !path.is_empty() {
            return Some(Target::Doc(path));
        }
        match &*book.read() {
            Some(Ok(book)) => openable(book).into_iter().next().map(|(target, _)| target),
            _ => None,
        }
    });
    // its text, read again after each change, named for the chapter it is of
    let text = use_resource(move || async move {
        let _ = changes();
        let target = target()?;
        let read = match &target {
            Target::Doc(path) => calls::chapter(project(), plan(), path.clone()).await,
            Target::Node(id) => calls::node_chapter(project(), plan(), id.clone()).await,
        };
        Some((target, read))
    });
    // the page's column, which scrolls, and what it was last scrolled to
    let mut column = use_signal(|| None::<dom::El>);
    let scrolled = use_hook(|| Rc::new(RefCell::new(None::<(Target, String)>)));
    // a chapter come in, or a heading named, scrolls to the heading or else to the chapter's top,
    // while a chapter read again after a change stays where it was
    let on_shown = scrolled.clone();
    use_effect(move || {
        let shown = match &*text.read() {
            Some(Some((target, Ok(_)))) => target.clone(),
            _ => return,
        };
        let at = (shown, anchor());
        if on_shown.borrow().as_ref() == Some(&at) {
            return;
        }
        if let Some(el) = column.peek().as_ref() {
            scroll(el, &at.1);
        }
        *on_shown.borrow_mut() = Some(at);
    });
    // a click on a link: a node's opens it, a chapter's opens it here, and any other is the
    // browser's, as are the clicks inside a view of the plan, whose sidebar has links of its own
    let on_click = move |event: MouseEvent| {
        let mods = event.modifiers();
        if mods.ctrl() || mods.meta() || mods.shift() || mods.alt() {
            return;
        }
        let Some(clicked) = dom::target(&*event.data()) else {
            return;
        };
        if dom::closest(&clicked, ".reef").is_some() {
            return;
        }
        if let Some(id) =
            dom::closest(&clicked, "[data-goto]").and_then(|a| dom::attr(&a, "data-goto"))
        {
            event.prevent_default();
            on_node.call(id);
            return;
        }
        let Some(href) = dom::closest(&clicked, "a[href]").and_then(|a| dom::attr(&a, "href"))
        else {
            return;
        };
        // the chapter the link's page is served as, if the book lists it
        let (page, anchor) = href.split_once('#').unwrap_or((&href, ""));
        let chapter = match &*book.peek() {
            Some(Ok(book)) => book
                .chapters()
                .into_iter()
                .find(|chapter| chapter.page.as_deref() == Some(page))
                .and_then(Target::of),
            _ => None,
        };
        let Some(chapter) = chapter else { return };
        event.prevent_default();
        // the chapter shown already scrolls to the heading now, as no new chapter comes
        if target.peek().as_ref() == Some(&chapter)
            && let Some(el) = column.peek().as_ref()
        {
            scroll(el, anchor);
            *scrolled.borrow_mut() = Some((chapter.clone(), anchor.to_string()));
        }
        navigator().push(chapter.route(project(), plan(), anchor.to_string()));
    };
    // a view filling the page takes the whole column, as reef.css has it take mdbook's
    let fills = match &*text.read() {
        Some(Some((_, Ok(Some(text))))) => text
            .runs
            .iter()
            .any(|run| matches!(run, Run::View { spec } if spec.layout == Layout::Page)),
        _ => false,
    };
    let open = target();
    rsx! {
        div { class: "reef-app-book",
            nav { class: "reef-app-book-bar", "aria-label": "The plan's chapters",
                match &*book.read() {
                    // keyed by the open chapter, so the sections folded by hand unfold again for
                    // the next one, as mdbook draws each page's bar afresh
                    Some(Ok(book)) => rsx! {
                        ChapterBar {
                            key: "{open:?}",
                            book: book.clone(),
                            open: open.clone(),
                            project: project(),
                            plan: plan(),
                        }
                    },
                    Some(Err(error)) => rsx! {
                        CallFailed { error: error.clone(), doing: "Reading the plan's docs" }
                    },
                    None => rsx! {
                        p { class: "reef-app-muted", "Loading…" }
                    },
                }
            }
            div {
                class: "reef-app-book-page",
                onmounted: move |event| column.set(dom::mounted(&event.data())),
                onclick: on_click,
                main { class: if fills { "reef-app-book-main fills" } else { "reef-app-book-main" },
                    match &*text.read() {
                        Some(Some((_, Ok(Some(text))))) => rsx! {
                            ChapterBody {
                                project: project(),
                                plan: plan(),
                                text: text.clone(),
                                changes,
                            }
                        },
                        Some(Some((Target::Doc(path), Ok(None)))) => rsx! {
                            p { class: "reef-app-muted", "This plan's docs have no doc at " code { "{path}" } "." }
                        },
                        Some(Some((Target::Node(id), Ok(None)))) => rsx! {
                            p { class: "reef-app-muted", "This plan has no node " code { "{id}" } "." }
                        },
                        Some(Some((_, Err(error)))) => rsx! {
                            CallFailed { error: error.clone(), doing: "Reading the chapter" }
                        },
                        Some(None) if matches!(&*book.read(), Some(Ok(_))) => rsx! {
                            p { class: "reef-app-muted", "This plan's book has no chapter to open." }
                        },
                        _ => rsx! {
                            p { class: "reef-app-muted", "Loading…" }
                        },
                    }
                    if !fills {
                        if let Some(Ok(book)) = &*book.read() {
                            PrevNext { book: book.clone(), open, project: project(), plan: plan() }
                        }
                    }
                }
            }
        }
    }
}

/// Scrolls the page's column to a heading in its chapter, or to its top for none
///
/// # Arguments
///
/// * `column` - The column, which scrolls
/// * `anchor` - The heading's id, or empty for the top
fn scroll(column: &dom::El, anchor: &str) {
    // a heading named, if the chapter has it, else the top
    let heading = (!anchor.is_empty())
        .then(|| dom::query(column, &format!("[id=\"{}\"]", anchor.replace('"', "\\\""))))
        .flatten();
    match heading {
        Some(heading) => dom::scroll_to_start(&heading),
        None => dom::scroll_to_top(column),
    }
}

/// The chapter bar: the book's title, then its chapters, parts and separators, each section
/// folding the chapters under it
///
/// A section starts folded unless it holds the open chapter, and a fold or unfold stands until
/// another chapter opens.
///
/// # Arguments
///
/// * `book` - The book
/// * `open` - The chapter open, if one is
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[component]
fn ChapterBar(book: Book, open: Option<Target>, project: String, plan: String) -> Element {
    // each section folded or unfolded by hand, by its place in the book
    let folds = use_signal(HashMap::<String, bool>::new);
    rsx! {
        if let Some(title) = &book.title {
            p { class: "reef-app-book-title", "{title}" }
        }
        ChapterList { items: book.items, at: String::new(), open, project, plan, folds }
    }
}

/// One list of a book's chapters, each section's own list under it while it is unfolded
///
/// # Arguments
///
/// * `items` - The list's chapters, parts and separators
/// * `at` - The list's place in the book, which each of its items' places start with
/// * `open` - The chapter open, if one is
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `folds` - Each section folded or unfolded by hand, by its place
#[component]
fn ChapterList(
    items: Vec<BookItem>,
    at: String,
    open: Option<Target>,
    project: String,
    plan: String,
    folds: Signal<HashMap<String, bool>>,
) -> Element {
    // the list at the book's root, or a section's under it
    let class = if at.is_empty() {
        "reef-app-chapters"
    } else {
        "reef-app-chapters reef-app-section"
    };
    rsx! {
        ol { class,
            for (i, item) in items.into_iter().enumerate() {
                match item {
                    BookItem::Chapter(chapter) => rsx! {
                        ChapterEntry {
                            key: "{i}",
                            chapter,
                            at: format!("{at}{i}."),
                            open: open.clone(),
                            project: project.clone(),
                            plan: plan.clone(),
                            folds,
                        }
                    },
                    BookItem::Part { title } => rsx! {
                        li { key: "{i}", class: "reef-app-part", "{title}" }
                    },
                    BookItem::Separator => rsx! {
                        li { key: "{i}", class: "reef-app-spacer" }
                    },
                }
            }
        }
    }
}

/// One chapter in the bar: its number and title, a link unless it is a draft or a section with
/// no page, marked if it is open, with a toggle folding the chapters under it
///
/// # Arguments
///
/// * `chapter` - The chapter
/// * `at` - Its place in the book
/// * `open` - The chapter open, if one is
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `folds` - Each section folded or unfolded by hand, by its place
#[component]
fn ChapterEntry(
    chapter: Chapter,
    at: String,
    open: Option<Target>,
    project: String,
    plan: String,
    folds: Signal<HashMap<String, bool>>,
) -> Element {
    let mut folds = folds;
    // whether it is open, and whether its section holds the open chapter
    let target = Target::of(&chapter);
    let is_open = open.is_some() && target == open;
    let holds_open = is_open
        || open
            .as_ref()
            .is_some_and(|open| holds(&chapter.items, open));
    // a section is unfolded if it was by hand, else if it holds the open chapter
    let section = !chapter.items.is_empty();
    let unfolded = section && folds.read().get(&at).copied().unwrap_or(holds_open);
    let number = chapter.shown_number();
    let title = chapter.title.clone();
    let class = match (is_open, unfolded) {
        (true, true) => "reef-app-chapter active expanded",
        (true, false) => "reef-app-chapter active",
        (false, true) => "reef-app-chapter expanded",
        (false, false) => "reef-app-chapter",
    };
    let place = at.clone();
    rsx! {
        li { class,
            div { class: "reef-app-chapter-line",
                match target {
                    Some(target) => rsx! {
                        Link {
                            class: if is_open { "active" } else { "" },
                            to: target.route(project.clone(), plan.clone(), String::new()),
                            if !number.is_empty() {
                                strong { "{number} " }
                            }
                            "{title}"
                        }
                    },
                    None => rsx! {
                        span { class: "reef-app-chapter-draft",
                            if !number.is_empty() {
                                strong { "{number} " }
                            }
                            "{title}"
                        }
                    },
                }
                if section {
                    button {
                        class: "reef-app-fold",
                        "aria-expanded": if unfolded { "true" } else { "false" },
                        title: if unfolded { "Fold {title}" } else { "Unfold {title}" },
                        onclick: move |_| {
                            folds.write().insert(place.clone(), !unfolded);
                        },
                        "❱"
                    }
                }
            }
            if unfolded {
                ChapterList {
                    items: chapter.items,
                    at,
                    open,
                    project,
                    plan,
                    folds,
                }
            }
        }
    }
}

/// Whether a list of chapters holds one, at any depth
///
/// # Arguments
///
/// * `items` - The list
/// * `target` - The chapter looked for
fn holds(items: &[BookItem], target: &Target) -> bool {
    // a chapter of the list is it, or holds it
    items.iter().any(|item| match item {
        BookItem::Chapter(chapter) => {
            Target::of(chapter).as_ref() == Some(target) || holds(&chapter.items, target)
        }
        BookItem::Part { .. } | BookItem::Separator => false,
    })
}

/// The open chapter's previous and next in the book, as mdbook's arrows lead to them
///
/// # Arguments
///
/// * `book` - The book
/// * `open` - The chapter open, if one is
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
#[component]
fn PrevNext(book: Book, open: Option<Target>, project: String, plan: String) -> Element {
    // the chapters it can open, in order, and where the open one is among them
    let chapters = openable(&book);
    let at = open
        .as_ref()
        .and_then(|open| chapters.iter().position(|(target, _)| target == open));
    let Some(at) = at else {
        return rsx! {};
    };
    let route = |(target, chapter): &(Target, &Chapter)| {
        (
            target.route(project.clone(), plan.clone(), String::new()),
            chapter.title.clone(),
        )
    };
    let prev = at.checked_sub(1).and_then(|i| chapters.get(i)).map(route);
    let next = chapters.get(at + 1).map(route);
    rsx! {
        nav { class: "reef-app-book-nav", "aria-label": "The chapters before and after",
            if let Some((to, title)) = prev {
                Link { class: "reef-app-book-prev", to, title: "Previous chapter: {title}",
                    span { class: "reef-app-book-arrow", "‹" }
                    span { class: "reef-app-book-near", "{title}" }
                }
            }
            if let Some((to, title)) = next {
                Link { class: "reef-app-book-next", to, title: "Next chapter: {title}",
                    span { class: "reef-app-book-near", "{title}" }
                    span { class: "reef-app-book-arrow", "›" }
                }
            }
        }
    }
}

/// A chapter's text: each run of HTML as it was rendered, and each fence's view of the plan,
/// which reads the plan's payload once for every view it holds
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `text` - The chapter's text
/// * `changes` - How many changes to the plan have come
#[component]
fn ChapterBody(
    project: String,
    plan: String,
    text: ChapterText,
    changes: ReadSignal<u64>,
) -> Element {
    // a chapter with a view of the plan reads its payload, and one with none never does
    if text.has_views() {
        return rsx! {
            Fenced { project, plan, runs: text.runs, changes }
        };
    }
    rsx! {
        Runs { project, plan, runs: text.runs, changes, payload: None }
    }
}

/// A chapter's text with views of the plan in it, over one read of the plan's payload
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `runs` - The chapter's runs
/// * `changes` - How many changes to the plan have come, which read the payload again
#[component]
fn Fenced(project: String, plan: String, runs: Vec<Run>, changes: ReadSignal<u64>) -> Element {
    // the plan's payload, named for its plan, read in the client and again after each change
    let (read_project, read_plan) = (project.clone(), plan.clone());
    let payload = use_resource(move || {
        let (project, plan) = (read_project.clone(), read_plan.clone());
        async move {
            let _ = changes();
            let name = format!("{project}/{plan}");
            calls::payload(project, plan)
                .await
                .map(|payload| -> Keyed { (name, payload) })
        }
    });
    rsx! {
        Runs { project, plan, runs, changes, payload: Some(payload) }
    }
}

/// A chapter's runs in order: HTML as it was rendered, each view as a canvas of the plan at
/// what its fence asks for, and a fence that can't be read saying why
///
/// # Arguments
///
/// * `project` - The project the plan belongs to
/// * `plan` - The plan's name within it
/// * `runs` - The chapter's runs
/// * `changes` - How many changes to the plan have come
/// * `payload` - The plan's payload, read for a chapter with views
#[component]
fn Runs(
    project: String,
    plan: String,
    runs: Vec<Run>,
    changes: ReadSignal<u64>,
    payload: Option<Resource<Result<Keyed, ServerFnError>>>,
) -> Element {
    rsx! {
        for (i, run) in runs.into_iter().enumerate() {
            match run {
                Run::Html { html } => rsx! {
                    div { key: "{i}", class: "reef-app-run", dangerous_inner_html: "{html}" }
                },
                // a view is keyed by what it shows, so an edited fence draws afresh and one
                // read again as it was stays as the reader left it
                Run::View { spec } => {
                    let shows = serde_json::to_string(&spec).unwrap_or_default();
                    let fills = spec.layout == Layout::Page;
                    match payload {
                        Some(payload) => rsx! {
                            div {
                                key: "{i}:{shows}",
                                class: if fills { "reef-app-fence fills" } else { "reef-app-fence" },
                                Canvas {
                                    project: project.clone(),
                                    plan: plan.clone(),
                                    payload,
                                    changes,
                                    spec: Some(spec),
                                }
                            }
                        },
                        None => rsx! {},
                    }
                }
                Run::Unread { error } => rsx! {
                    p { key: "{i}", class: "reef-app-error reef-app-fence-unread",
                        "This view of the plan can't be drawn: {error}"
                    }
                },
            }
        }
    }
}
