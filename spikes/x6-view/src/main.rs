//! X6's scratch Dioxus fullstack app, drawing a plan with reef-view linked into its wasm
//!
//! The server reads a book's plan into the viewer's payload and serves the book's rendered pages
//! under `/book/`, so *Read here* has pages to fetch. The client mounts reef-view into an element
//! Dioxus made, through `reef_view::mount`, and swaps each payload after the first in through
//! the handle it returned.

use dioxus::prelude::*;
use reef_model::Payload;
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
mod book;

/// The view the app draws: the example book's own plan view, filling the page
#[cfg(feature = "web")]
const VIEW: &str = r#"{"root": "scans-plan", "layout": "page"}"#;

/// The mdbook theme variables reef.css reads with no fallback, which a page outside mdbook lacks
const THEME: &str = r#"
:root { --fg: #333; --bg: #fff; --links: #2b79a2; }
html, body { margin: 0; height: 100%; font-family: sans-serif; color: var(--fg); background: var(--bg); }
main { display: flex; flex-direction: column; height: 100vh; box-sizing: border-box; padding: 8px; }
header { display: flex; gap: 6px; align-items: center; flex-wrap: wrap; }
"#;

/// Which plan the app asks the server for
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Which {
    /// The example book's plan, as written
    Example,
    /// The example book's plan with its first work item to do marked done: a newer revision
    ExampleOneDone,
    /// This plan's own book, a different plan altogether
    Plan,
}

/// Starts the wasm client, or the server with the books' rendered pages beside the app
fn main() {
    // the wasm client renders the page and draws the plan
    #[cfg(not(feature = "server"))]
    dioxus::launch(App);
    // the server serves the app, its server functions, reef.css and both books' rendered pages
    #[cfg(feature = "server")]
    dioxus::serve(|| async move {
        use tower_http::services::{ServeDir, ServeFile};
        // the pages `mdbook build` rendered, where the payload's doc links point
        let (example, plan) = (book::Book::example(), book::Book::plan());
        // the viewer's stylesheet from mdbook-reef's assets: `asset!` refuses a file outside the
        // crate, and mdbook hashes the copy it renders into a book
        let css = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../crates/mdbook-reef/assets/reef.css"
        );
        Ok(dioxus::server::router(App)
            .route_service("/reef.css", ServeFile::new(css))
            .nest_service(
                example.served_at.trim_end_matches('/'),
                ServeDir::new(example.rendered()),
            )
            .nest_service(
                plan.served_at.trim_end_matches('/'),
                ServeDir::new(plan.rendered()),
            ))
    });
}

/// The page: buttons that pick a plan or hide the view, and the view itself
#[component]
fn App() -> Element {
    // the plan to draw, and whether the view is in the page at all
    let mut which = use_signal(|| Which::Example);
    let mut shown = use_signal(|| true);
    rsx! {
        Head {}
        main {
            header {
                strong { "X6: reef-view inside Dioxus" }
                button { onclick: move |_| which.set(Which::Example), "Example book" }
                button { onclick: move |_| which.set(Which::ExampleOneDone), "Example, one more done" }
                button { onclick: move |_| which.set(Which::Plan), "This plan" }
                button {
                    onclick: move |_| shown.toggle(),
                    if shown() { "Hide the view" } else { "Show the view" }
                }
                span { "Drawing: {which:?}" }
            }
            if shown() {
                PlanView { which }
            }
        }
    }
}

/// The page's stylesheets: reef.css as mdbook-reef installs it, and the theme it expects
///
/// It takes no props, so it never renders again: `document::Style` warns when its props change,
/// and a plain `style` element gets a hydration marker in its text that breaks its first rule.
#[component]
fn Head() -> Element {
    rsx! {
        document::Stylesheet { href: "/reef.css" }
        document::Style { {THEME} }
    }
}

/// The plan drawn by reef-view, mounted once and swapped after that
#[component]
fn PlanView(which: Signal<Which>) -> Element {
    // the chosen plan's payload, fetched again whenever the choice changes
    let payload = use_resource(move || async move { plan_payload(which()).await });
    // the element Dioxus made for the view, once it is in the page
    let mut element = use_signal(|| None::<std::rc::Rc<MountedData>>);
    // the drawn view, once the first payload has come
    let handle = use_signal(|| None::<Drawn>);
    // draw each payload as it comes, once the element is there
    use_effect(move || {
        // nothing can be drawn until Dioxus has put the element in the page
        let Some(mounted) = element() else {
            return;
        };
        // a payload that failed or hasn't come yet leaves the view as it is
        if let Some(Ok(payload)) = &*payload.read() {
            draw(&mounted, payload.clone(), handle);
        }
    });
    rsx! {
        div {
            class: "reef reef-page",
            onmounted: move |event| element.set(Some(event.data())),
        }
        match &*payload.read() {
            Some(Err(error)) => rsx! { p { "Failed: {error}" } },
            None => rsx! { p { "Loading the plan…" } },
            Some(Ok(_)) => rsx! {},
        }
    }
}

/// A view reef-view drew, in the client
#[cfg(feature = "web")]
type Drawn = reef_view::Handle;

/// Nothing is drawn on the server
#[cfg(not(feature = "web"))]
type Drawn = ();

/// Mounts the viewer the first time, and swaps the payload in after that
///
/// # Arguments
///
/// * `mounted` - The element Dioxus made for the view
/// * `payload` - The plan to draw
/// * `handle` - The view drawn so far, if any
#[cfg(feature = "web")]
fn draw(mounted: &MountedData, payload: Payload, mut handle: Signal<Option<Drawn>>) {
    use dioxus::web::WebEventExt;
    use wasm_bindgen::JsCast;
    // a view drawn already takes the new payload in place, with no reload
    if let Some(drawn) = &*handle.peek() {
        drawn.swap(payload);
        return;
    }
    // the first payload mounts the viewer into Dioxus's element
    let Some(element) = mounted.try_as_web_event() else {
        return;
    };
    let view: reef_model::ViewSpec = serde_json::from_str(VIEW).expect("the view parses");
    handle.set(Some(reef_view::mount(
        element.unchecked_into(),
        payload,
        view,
    )));
}

/// Nothing is drawn on the server, where effects never run
#[cfg(not(feature = "web"))]
fn draw(_mounted: &MountedData, _payload: Payload, _handle: Signal<Option<Drawn>>) {}

/// The chosen plan as the viewer's payload, its doc links under `/book/`
///
/// # Arguments
///
/// * `which` - The plan to read
#[server]
async fn plan_payload(which: Which) -> Result<Payload, ServerFnError> {
    // read the book's plan the way mdbook-reef does
    let book = match which {
        Which::Example | Which::ExampleOneDone => book::Book::example(),
        Which::Plan => book::Book::plan(),
    };
    let mut payload = book
        .payload()
        .map_err(|error| ServerFnError::new(error.to_string()))?;
    // a newer revision of the same plan has one more item done
    if which == Which::ExampleOneDone
        && let Some(id) = book::finish_one(&mut payload)
    {
        eprintln!("x6: marked {id} done");
    }
    Ok(payload)
}
