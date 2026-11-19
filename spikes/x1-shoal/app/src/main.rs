//! X1's scratch Dioxus fullstack server, with shoal in-process or external
//!
//! With `X1_SHOAL_ADDR` unset the server starts shoal in its own process; set, it connects to the
//! node at that address. Either way it answers `GET /x1/roundtrip`, a plain JSON route on the
//! app's own port, and the page round-trips the same note through a server function.

use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

#[cfg(feature = "server")]
mod store;

/// Which build answered; `restart.sh` changes it to tell a restarted server from the old one
pub const BUILD: &str = "0";

/// What one round trip wrote and read back
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RoundTrip {
    /// How the server reaches shoal: `in-process` or `external`
    pub mode: String,
    /// The build that answered
    pub build: String,
    /// The note this round trip wrote and read back
    pub text: String,
    /// The note the last round trip left, read before writing, if there was one
    pub prior: Option<String>,
}

/// Starts the wasm client, or the server with shoal opened before it serves
fn main() {
    // the wasm client only renders the page
    #[cfg(not(feature = "server"))]
    dioxus::launch(App);
    // the server opens shoal, then serves the app and the JSON route on one port
    #[cfg(feature = "server")]
    dioxus::serve(|| async move {
        // start or reach shoal before serving, once: this runs again after every hot-patch
        store::Store::get().await?;
        // the app and its server functions, plus a plain JSON route beside them
        Ok(dioxus::server::router(App).route(
            "/x1/roundtrip",
            dioxus::server::axum::routing::get(store::round_trip_route),
        ))
    });
}

/// The page: a button that round-trips a note through a server function
#[component]
fn App() -> Element {
    // the last round trip, or why it failed
    let mut result = use_signal(|| None::<Result<RoundTrip, String>>);
    rsx! {
        h1 { "X1: shoal behind a Dioxus server" }
        button {
            onclick: move |_| async move {
                // ask the server to round-trip a note, and show what came back
                let answer = round_trip().await.map_err(|error| error.to_string());
                result.set(Some(answer));
            },
            "Round-trip a note"
        }
        match result() {
            None => rsx! { p { "Nothing round-tripped yet." } },
            Some(Ok(trip)) => rsx! {
                p { "Mode: {trip.mode}, build {trip.build}" }
                p { "Wrote and read back: {trip.text}" }
                p { "Before that: {trip.prior:?}" }
            },
            Some(Err(error)) => rsx! { p { "Failed: {error}" } },
        }
    }
}

/// Round-trips a note through shoal, for the page
#[server]
async fn round_trip() -> Result<RoundTrip, ServerFnError> {
    // the store the server opened before it started serving
    let store = store::Store::get().await.map_err(ServerFnError::new)?;
    // read the last note, write ours, and read it back
    store.round_trip().await.map_err(ServerFnError::new)
}
