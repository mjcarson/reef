//! reef's UI: a Dioxus 0.7 fullstack app on axum
//!
//! Its front page picks a project's plan, and each plan opens full-window. The server answers
//! the app's server functions, which call `reef-service`, and serves reef's JSON API on the same
//! port, so the CLI, the MCP server and the app all reach one service.
//!
//! dx builds two sides from this crate: the wasm client with `web`, and the server with
//! `server`, or `embedded` for shoal in-process.
//!
//! ```sh
//! dx serve @server --platform server --features embedded
//! ```

// a handler's future over shoal's client nests past the compiler's default depth
#![recursion_limit = "256"]

pub mod app;
pub mod calls;
pub mod live;
pub mod panel;
#[cfg(feature = "server")]
pub mod server;
pub mod view;

pub use app::App;
