//! reef's external shoal node
//!
//! ```sh
//! cargo +nightly run --release -p reef-shoal -- serve --conf shoal.yml
//! ```
//!
//! The whole program is `shoal::server::node::main` over reef's schema, so a reef-service client
//! and this node agree on the fingerprint their hello checks. Point reef at it with
//! `[shoal] address` in reef's config.

use reef_service::schema::Reef;

/// Serve reef's schema until told to stop
fn main() -> Result<(), shoal::server::ServerError> {
    // shoal's node program: `serve` or `claim`, each with `--conf`
    shoal::server::node::main::<Reef>()
}
