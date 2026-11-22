//! What reef's viewers draw a plan from.
//!
//! reef-view, the static mode's wasm viewer, and reef-app's Dioxus canvas draw the same boxes,
//! sidebar and charts, so everything that decides what they draw lives here, apart from either
//! one's DOM: how big each box is and where the layout puts it ([`Scene`]), the pan and zoom
//! ([`Camera`]), the search, the acceptance filter, the Performance section's charts, and the
//! diagram and sidebar as markup, which reef-view writes into the page and the canvas is checked
//! against. It is pure, so it is tested natively.
//!
//! The `web` feature adds what only a browser can do: read a doc's rendered page for the
//! sidebar, and measure text in the view's font.

pub mod accept;
pub mod markup;
pub mod perf;
mod view;
#[cfg(feature = "web")]
pub mod web;

pub use accept::Filter;
pub use markup::BoxSize;
pub use view::{
    Camera, MAX_K, MIN_K, Scene, groups_from_stored, groups_to_stored, plan_key, search, side_w_key,
};
