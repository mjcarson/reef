//! reef's JSON API on its own, until reef-app mounts the router beside its pages
//!
//! ```sh
//! cargo +nightly run --release -p reef-service --features embedded --bin reef-serve
//! cargo +stable run -p reef-service --bin reef-serve -- --conf reef.toml   # an external node
//! ```
//!
//! It reads reef's config (`--conf`, else `$REEF_CONFIG`, else the default path), listens on
//! `api.address`, which must be a loopback address, then opens the service: shoal in this
//! process with `embedded`, or the external node `shoal.address` names. With `[auth] required`
//! it says callers must sign in, and while reef has no user, how to add the first. It listens before shoal
//! starts, so a port that is taken or an address that isn't local stops it before a pool locks
//! the data directory. On SIGINT or SIGTERM it ends the change stream, waits for the requests
//! still open, and closes the service, stopping an in-process pool cleanly.

// a handler's future over shoal's client nests past the compiler's default depth
#![recursion_limit = "256"]

use reef_service::{Conf, Service, api};
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;
use tracing::instrument;

/// How long to wait for the last request's hold on the service to go once the server stops
const RELEASE_WAIT: Duration = Duration::from_secs(2);

/// Serves the JSON API until told to stop
fn main() -> ExitCode {
    // shoal's shards get the stack reef's schema needs, before any thread starts
    #[cfg(feature = "embedded")]
    // SAFETY: nothing has started a thread yet
    unsafe {
        reef_service::pool::widen_stacks()
    };
    // `--conf PATH` alone; anything else is a mistake worth saying
    let conf = match conf_path(std::env::args_os().skip(1)) {
        Ok(conf) => conf,
        Err(message) => {
            eprintln!("reef-serve: {message}\nusage: reef-serve [--conf PATH]");
            return ExitCode::from(2);
        }
    };
    // every request, and the service, on one tokio runtime
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("reef-serve: starting tokio: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(conf)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("reef-serve: {error:#}");
            ExitCode::FAILURE
        }
    }
}

/// Listens, opens the service, serves until a signal, then closes the service
///
/// # Arguments
///
/// * `conf` - The config file named on the command line, if one was
#[instrument(name = "reef_serve::run", skip_all, err(Debug))]
async fn run(conf: Option<PathBuf>) -> anyhow::Result<()> {
    // reef's config, and the address it names, before shoal starts
    let conf = Conf::load(conf.as_deref())?;
    let listener = api::bind(&conf).await?;
    // shoal in this process, or the external node the config names
    let service = Arc::new(Service::open(&conf).await?);
    eprintln!(
        "reef-serve: serving reef's JSON API on http://{}, shoal {}",
        listener.local_addr()?,
        service.mode().name()
    );
    // that callers must sign in, if they must, and how to add the first user
    if let Some(note) = service.sign_in_note().await? {
        eprintln!("reef-serve: {note}");
    }
    // serve until told to stop, then wait for the requests still open, fetching every remote
    // project on its interval meanwhile, and stopping any fetch still running once it ends
    tokio::select! {
        served = api::serve(listener, service.clone(), stopped()) => served?,
        () = service.follow_remotes() => {}
    }
    // the service is this task's alone once every request has let go of it
    match released(service).await {
        Some(service) => service.close().await?,
        None => eprintln!(
            "reef-serve: a request still holds the service, so shoal stops with the process"
        ),
    }
    Ok(())
}

/// Resolves on the first SIGINT or SIGTERM
async fn stopped() {
    // a terminal's ctrl-c, or a supervisor's stop
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(term) => term,
        Err(error) => {
            // without SIGTERM, ctrl-c still stops it
            eprintln!("reef-serve: SIGTERM can't stop this server: {error}");
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
    eprintln!("reef-serve: stopping");
}

/// Takes the service back once nothing else holds it, waiting a moment for the last request
///
/// A finished connection lets go of its handle as its task ends, which can be just after the
/// server says it has stopped.
///
/// # Arguments
///
/// * `service` - The handle the server was given clones of
async fn released(mut service: Arc<Service>) -> Option<Service> {
    let deadline = tokio::time::Instant::now() + RELEASE_WAIT;
    loop {
        // the last handle unwraps
        match Arc::try_unwrap(service) {
            Ok(service) => return Some(service),
            Err(held) => service = held,
        }
        // give up once the wait is over
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The config file a command line names, or `None` for the default
///
/// # Arguments
///
/// * `args` - The arguments after the program's name
fn conf_path(mut args: impl Iterator<Item = OsString>) -> Result<Option<PathBuf>, String> {
    let mut conf = None;
    while let Some(arg) = args.next() {
        // `--conf X` or `--conf=X`, once
        let path = if arg == "--conf" {
            args.next()
                .map(PathBuf::from)
                .ok_or_else(|| "`--conf` needs a path".to_string())?
        } else if let Some(path) = arg.to_str().and_then(|arg| arg.strip_prefix("--conf=")) {
            PathBuf::from(path)
        } else {
            return Err(format!("unknown argument `{}`", arg.to_string_lossy()));
        };
        if conf.replace(path).is_some() {
            return Err("`--conf` is given twice".to_string());
        }
    }
    Ok(conf)
}

#[cfg(test)]
mod tests {
    use super::conf_path;
    use std::ffi::OsString;
    use std::path::PathBuf;

    /// Reads the config path out of a command line
    ///
    /// # Arguments
    ///
    /// * `args` - The arguments after the program's name
    fn path(args: &[&str]) -> Result<Option<PathBuf>, String> {
        conf_path(args.iter().map(OsString::from))
    }

    /// Both spellings of `--conf`, the default, and the mistakes it refuses
    #[test]
    fn conf_path_reads_one_conf() {
        // either spelling names the file, and none means the default
        assert_eq!(path(&["--conf", "a.toml"]), Ok(Some("a.toml".into())));
        assert_eq!(path(&["--conf=b.toml"]), Ok(Some("b.toml".into())));
        assert_eq!(path(&[]), Ok(None));
        // a missing path, a second config or anything else is refused
        assert!(path(&["--conf"]).is_err());
        assert!(path(&["--conf", "a", "--conf=b"]).is_err());
        assert!(path(&["serve"]).is_err());
    }
}
