//! reef-app's binary: the wasm client in the browser, or the server that serves it
//!
//! The server reads reef's config (`$REEF_CONFIG`, else the default path) and listens over HTTPS
//! on `api.address`, where the CLI looks for the JSON API, unless dx names an address of its
//! own, which its proxy calls over plain HTTP.

// a handler's future over shoal's client nests past the compiler's default depth
#![recursion_limit = "256"]

/// Starts the wasm client, or the server
fn main() {
    // the wasm client renders the pages and draws the plan
    #[cfg(not(feature = "server"))]
    dioxus::launch(reef_app::App);
    // the server answers the pages, the server functions and the JSON API on one port
    #[cfg(feature = "server")]
    std::process::exit(serve());
}

/// Serves the app until told to stop, saying why it stopped if it failed
#[cfg(feature = "server")]
fn serve() -> i32 {
    // shoal's shards get the stack reef's schema needs, before any thread starts
    #[cfg(feature = "embedded")]
    // SAFETY: nothing has started a thread yet
    unsafe {
        reef_service::pool::widen_stacks()
    };
    // the service, the router and the listener all run on one tokio runtime
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("reef-app: starting tokio: {error}");
            return 1;
        }
    };
    match runtime.block_on(reef_app::server::serve()) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("reef-app: {error:#}");
            1
        }
    }
}
