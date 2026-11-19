//! X1's external shoal node
//!
//! ```sh
//! cargo +nightly run --release -p x1-node -- serve --conf node/shoal.yml
//! ```
//!
//! The whole program is `shoal::server::node::main` over the same tables the app includes, so
//! the app's client and this server agree on the schema fingerprint the hello checks.

use deepsize2::DeepSizeOf;
use rkyv::{Archive, Deserialize, Serialize};
use shoal::{FileSystem, PersistentUnsortedTable, ShoalUnsortedTable};

// the tables, shared with the app
include!("../../tables.rs");

/// The spike's database
#[shoal::db]
pub struct X1 {
    /// The notes the app round-trips
    pub notes: PersistentUnsortedTable<Note, FileSystem>,
}

/// Serve the spike's schema until told to stop
fn main() -> Result<(), shoal::server::ServerError> {
    shoal::server::node::main::<X1>()
}
