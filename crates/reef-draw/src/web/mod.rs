//! What only a browser can do for a viewer: read a doc's rendered page for the sidebar, and
//! measure text in the view's font

pub mod docs;
mod measure;

pub use measure::Measure;
