//! reef's UI: a Dioxus 0.7 fullstack app on axum
//!
//! Its front page explores every project's plans, each summed up and kept live (`explorer`), and
//! each plan opens full-window, beside its queue of work to take first (`queue`), or as its docs,
//! read as a book with its fences drawn live (`docs`). The server
//! answers the app's server functions, which call `reef-service`, and serves reef's JSON API on
//! the same port, so the CLI, the MCP server and the app all reach one service. Every caller
//! signs in, so the bar says whom the server takes the browser for (`sign_in`), and links to
//! their account page, for their password and tokens, and every user to an admin (`account`).
//!
//! dx builds two sides from this crate: the wasm client with `web`, and the server with
//! `server`, or `embedded` for shoal in-process.
//!
//! ```sh
//! dx serve @server --platform server --features embedded
//! ```

// a handler's future over shoal's client nests past the compiler's default depth
#![recursion_limit = "256"]

pub mod account;
pub mod app;
pub mod calls;
pub mod canvas;
pub mod docs;
pub mod explorer;
pub mod live;
pub mod panel;
pub mod queue;
#[cfg(feature = "server")]
pub mod server;
pub mod session;
pub mod side;
pub mod sign_in;

pub use app::App;
