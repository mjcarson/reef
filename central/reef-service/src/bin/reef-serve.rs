//! reef's JSON API on its own, until reef-app mounts the router beside its pages
//!
//! ```sh
//! cargo +nightly run --release -p reef-service --features embedded --bin reef-serve
//! cargo +stable run -p reef-service --bin reef-serve -- --conf reef.toml   # an external node
//! ```
//!
//! It reads reef's config (`--conf`, else `$REEF_CONFIG`, else the default path), makes or reads
//! back reef's own certificate, listens over HTTPS on `api.address`, which must be a loopback
//! address, and on `api.tls.address` if `[api.tls]` names one, then opens the service: shoal in
//! this process with `embedded`, or the external node `shoal.address` names. It says where
//! reef's own CA is, to trust, and with `[auth] required` that callers must sign in, and while
//! reef has no user, how to add the first. It listens before shoal starts, so a port that is
//! taken, an address that isn't local, or a certificate that can't be read stops it before a
//! pool locks the data directory. On SIGINT or SIGTERM it ends the change stream, waits for the requests
//! still open, and closes the service, stopping an in-process pool cleanly.
//!
//! ```sh
//! reef-serve recover ada [--password-file PATH] [--conf PATH]
//! ```
//!
//! `recover` gets a lost admin back from the store itself, never over the API: it asks for a new
//! password twice with echo off, or reads `--password-file`, sets it, makes the user an admin and
//! revokes their tokens, or adds them as an admin if no user has the name. Whoever can open the
//! store holds the service's data already, so nobody else is asked. An in-process pool locks the
//! data directory, so stop the service first; an external node can be reached while it runs, and
//! the running service then trusts a revoked token for up to a minute.
//!
//! ```sh
//! reef-serve version [--json]
//! ```
//!
//! `version` prints the schema this binary reads and writes: its structural id, then each table's
//! fingerprint, in the order the schema serves them, or the same as one JSON object. It reads no
//! config and opens nothing, so reefadm can ask a binary it just built whether the data directory
//! it is about to be given was written by a schema it reads.

// a handler's future over shoal's client nests past the compiler's default depth
#![recursion_limit = "256"]

use reef_service::api::{self, Listeners};
use reef_service::schema::ReefClient;
use reef_service::service::Recovered;
use reef_service::{Conf, Mode, Service};
use shoal::shared::traits::QuerySupport;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;
use tracing::instrument;

/// How long to wait for the last request's hold on the service to go once the server stops
const RELEASE_WAIT: Duration = Duration::from_secs(2);

/// What a command line can ask for, and how
const USAGE: &str = "usage: reef-serve [--conf PATH]\n       \
                     reef-serve recover NAME [--password-file PATH] [--conf PATH]\n       \
                     reef-serve version [--json]";

/// What reef-serve is asked to do
#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    /// Serve the JSON API until told to stop
    Serve,
    /// Recover a lost admin from the store itself
    Recover {
        /// The user's name
        name: String,
        /// Where to read their new password from instead of the terminal
        password_file: Option<PathBuf>,
    },
    /// Print the schema this binary reads and writes
    Version {
        /// Whether to print it as one JSON object
        json: bool,
    },
}

/// A command line, read
#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    /// The config file it names, if it names one
    conf: Option<PathBuf>,
    /// What it asks for
    command: Command,
}

/// Serves the JSON API until told to stop
fn main() -> ExitCode {
    // shoal's shards get the stack reef's schema needs, before any thread starts
    #[cfg(feature = "embedded")]
    // SAFETY: nothing has started a thread yet
    unsafe {
        reef_service::pool::widen_stacks()
    };
    // serving, or recovering an admin; anything else is a mistake worth saying
    let args = match read_args(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("reef-serve: {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    // the schema is known without a config, a runtime or the store
    if let Command::Version { json } = args.command {
        println!("{}", version(json));
        return ExitCode::SUCCESS;
    }
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
    let done = match args.command {
        Command::Serve => runtime.block_on(run(args.conf)),
        Command::Recover {
            name,
            password_file,
        } => runtime.block_on(recover(args.conf, &name, password_file.as_deref())),
        // printed above, before the runtime started
        Command::Version { .. } => Ok(()),
    };
    match done {
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
    // reef's config, and where it says to listen, before shoal starts
    let conf = Conf::load(conf.as_deref())?;
    let listeners = Listeners::bind(&conf).await?;
    // shoal in this process, or the external node the config names
    let service = Arc::new(Service::open(&conf).await?);
    eprintln!(
        "reef-serve: serving reef's JSON API on {}, shoal {}",
        listeners.roots()?.join(" and "),
        service.mode().name()
    );
    // where reef's own CA is, to trust
    if let Some(note) = listeners.trust_note() {
        eprintln!("reef-serve: {note}");
    }
    // that callers must sign in, if they must, and how to add the first user
    if let Some(note) = service.sign_in_note().await? {
        eprintln!("reef-serve: {note}");
    }
    // serve until told to stop, then wait for the requests still open, fetching every remote
    // project on its interval meanwhile, and stopping any fetch still running once it ends
    tokio::select! {
        served = api::serve(listeners, service.clone(), stopped()) => served?,
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

/// Recovers a lost admin from the store itself: their new password set, made an admin, their
/// tokens revoked, or added as an admin if no user has the name
///
/// # Arguments
///
/// * `conf` - The config file named on the command line, if one was
/// * `name` - The user's name
/// * `password_file` - Where to read the new password from instead of the terminal
#[instrument(name = "reef_serve::recover", skip_all, err(Debug))]
async fn recover(
    conf: Option<PathBuf>,
    name: &str,
    password_file: Option<&std::path::Path>,
) -> anyhow::Result<()> {
    // the config, and the new password before anything is opened, so a slip opens nothing
    let conf = Conf::load(conf.as_deref())?;
    let prompt = format!("new password for `{name}`: ");
    let password = reef_api::prompt::read_new_password(&prompt, password_file)
        .map_err(|error| anyhow::anyhow!("{error}, so nothing was changed"))?;
    // the store, which a running service holds when shoal runs inside it
    let service = Service::open(&conf)
        .await
        .map_err(|error| held(error, &conf))?;
    let recovered = service.recover_admin(name, password).await;
    let mode = service.mode();
    service.close().await?;
    match recovered? {
        Recovered::Reset { revoked, was_admin } => {
            let made = if was_admin { "" } else { ", made an admin," };
            eprintln!(
                "reef-serve: set the password of `{name}`{made} and revoked {revoked} token(s)"
            );
        }
        Recovered::Added => eprintln!("reef-serve: added `{name}` as an admin"),
    }
    // a running service over an external node trusted a token it looked up for a minute
    if mode == Mode::External {
        eprintln!("reef-serve: a service running now may trust a revoked token for up to a minute");
    }
    Ok(())
}

/// The schema this binary reads and writes: its structural id, then each table's fingerprint
///
/// # Arguments
///
/// * `json` - Whether to say it as one JSON object rather than a line each
fn version(json: bool) -> String {
    // the id moves with any change to a row, a table or a projection
    let schema = <ReefClient as QuerySupport>::SCHEMA_ID;
    // and each table's own fingerprint says which one moved, in the order the schema serves them
    let tables = <ReefClient as QuerySupport>::table_fingerprints();
    match json {
        // numbers past 2^53 stay exact, since serde_json keeps a u64 as written
        true => serde_json::json!({
            "schema": schema,
            "tables": tables
                .iter()
                .map(|(name, fingerprint)| serde_json::json!([name, fingerprint]))
                .collect::<Vec<_>>(),
            "features": compiled_features(),
        })
        .to_string(),
        // a line for the schema, then one for each table
        false => std::iter::once(format!("schema {schema}"))
            .chain(
                tables
                    .iter()
                    .map(|(name, fingerprint)| format!("{name} {fingerprint}")),
            )
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

/// The x86-64 features this binary was compiled with, which a CPU it runs on must have
///
/// reefadm builds a release for the target's CPU, and refuses to install one whose features the
/// target lacks; the names are the ones its own run-time detection asks a CPU about.
fn compiled_features() -> Vec<&'static str> {
    // each feature, as the compiler was told
    macro_rules! compiled {
        ($($name:tt),* $(,)?) => {
            [$(($name, cfg!(target_feature = $name))),*]
        };
    }
    compiled!(
        "aes",
        "pclmulqdq",
        "rdrand",
        "rdseed",
        "sse",
        "sse2",
        "sse3",
        "ssse3",
        "sse4.1",
        "sse4.2",
        "sse4a",
        "sha",
        "sha512",
        "sm3",
        "sm4",
        "avx",
        "avx2",
        "avx512f",
        "avx512cd",
        "avx512bw",
        "avx512dq",
        "avx512vl",
        "avx512ifma",
        "avx512vbmi",
        "avx512vpopcntdq",
        "avx512vbmi2",
        "gfni",
        "vaes",
        "vpclmulqdq",
        "avx512vnni",
        "avx512bitalg",
        "avx512bf16",
        "avx512vp2intersect",
        "avx512fp16",
        "avxvnni",
        "avxifma",
        "avxneconvert",
        "avxvnniint8",
        "avxvnniint16",
        "f16c",
        "fma",
        "bmi1",
        "bmi2",
        "lzcnt",
        "tbm",
        "popcnt",
        "fxsr",
        "xsave",
        "xsaveopt",
        "xsaves",
        "xsavec",
        "cmpxchg16b",
        "kl",
        "widekl",
        "adx",
        "rtm",
        "movbe",
        "ermsb",
    )
    .into_iter()
    .filter(|(_, on)| *on)
    .map(|(name, _)| name)
    .collect()
}

/// Says a store another process holds is the running service's, and to stop it first
///
/// # Arguments
///
/// * `error` - Why the store didn't open
/// * `conf` - reef's config, whose data directory it is
fn held(error: anyhow::Error, conf: &Conf) -> anyhow::Error {
    // shoal says so when its storage directory is locked, by its error's name or in words
    let said = format!("{error:#}");
    match said.contains("StorageDirectoryLocked") || said.contains("another process holds") {
        true => error.context(format!(
            "reef is running over {}, so stop it first (`systemctl --user stop reef`) and run \
             this again",
            conf.data_dir()
                .map(|dir| dir.display().to_string())
                .unwrap_or_else(|_| "its data directory".to_string())
        )),
        false => error,
    }
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

/// Reads a command line: `--conf PATH` alone to serve, `recover NAME`, with
/// `--password-file PATH` and `--conf PATH` in any order, or `version`, with `--json` after it
///
/// # Arguments
///
/// * `args` - The arguments after the program's name
fn read_args(mut args: impl Iterator<Item = OsString>) -> Result<Args, String> {
    let mut conf = None;
    let mut recover = false;
    let mut name = None;
    let mut password_file = None;
    let mut version = false;
    let mut json = false;
    while let Some(arg) = args.next() {
        let text = arg.to_str().unwrap_or_default();
        // `--conf X` or `--conf=X`, once
        if !version && let Some(path) = flag(&arg, "--conf", &mut args)? {
            if conf.replace(path).is_some() {
                return Err("`--conf` is given twice".to_string());
            }
        // and a recovery's password file, once
        } else if recover && let Some(path) = flag(&arg, "--password-file", &mut args)? {
            if password_file.replace(path).is_some() {
                return Err("`--password-file` is given twice".to_string());
            }
        // `version` reads no config, so it takes only `--json`, once, after it
        } else if text == "version" && !version && !recover && conf.is_none() {
            version = true;
        } else if version && text == "--json" && !json {
            json = true;
        // the command comes first, then the name
        } else if text == "recover" && !recover && !version {
            recover = true;
        } else if recover && name.is_none() && !text.starts_with('-') && !text.is_empty() {
            name = Some(text.to_string());
        } else {
            return Err(format!("unknown argument `{}`", arg.to_string_lossy()));
        }
    }
    let command = match (recover, name) {
        (false, _) if version => Command::Version { json },
        (false, _) => Command::Serve,
        (true, Some(name)) => Command::Recover {
            name,
            password_file,
        },
        (true, None) => return Err("`recover` needs the name of the user to recover".to_string()),
    };
    Ok(Args { conf, command })
}

/// A flag's path, if the argument is the flag, as `--flag X` or `--flag=X`
///
/// # Arguments
///
/// * `arg` - The argument
/// * `name` - The flag, such as `--conf`
/// * `args` - The arguments after it, the next of which is its path when spelt apart
fn flag(
    arg: &OsString,
    name: &str,
    args: &mut impl Iterator<Item = OsString>,
) -> Result<Option<PathBuf>, String> {
    // the flag, then its path
    if arg == name {
        return args
            .next()
            .map(|path| Some(PathBuf::from(path)))
            .ok_or_else(|| format!("`{name}` needs a path"));
    }
    // or the two joined by `=`
    let joined = arg
        .to_str()
        .and_then(|arg| arg.strip_prefix(name))
        .and_then(|rest| rest.strip_prefix('='));
    Ok(joined.map(PathBuf::from))
}

#[cfg(test)]
mod tests {
    use super::{Args, Command, read_args, version};
    use std::ffi::OsString;
    use std::path::PathBuf;

    /// Reads a command line
    ///
    /// # Arguments
    ///
    /// * `args` - The arguments after the program's name
    fn read(args: &[&str]) -> Result<Args, String> {
        read_args(args.iter().map(OsString::from))
    }

    /// Serving takes one `--conf` in either spelling, and refuses anything else
    #[test]
    fn serving_reads_one_conf() {
        // either spelling names the file, and none means the default
        let serve = |conf: Option<&str>| Args {
            conf: conf.map(PathBuf::from),
            command: Command::Serve,
        };
        assert_eq!(read(&["--conf", "a.toml"]), Ok(serve(Some("a.toml"))));
        assert_eq!(read(&["--conf=b.toml"]), Ok(serve(Some("b.toml"))));
        assert_eq!(read(&[]), Ok(serve(None)));
        // a missing path, a second config or anything else is refused
        assert!(read(&["--conf"]).is_err());
        assert!(read(&["--conf", "a", "--conf=b"]).is_err());
        assert!(read(&["serve"]).is_err());
        assert!(read(&["--password-file", "p"]).is_err());
    }

    /// `recover` takes a name, and a password file and a config in any order
    #[test]
    fn recover_reads_a_name_and_its_flags() {
        // the name alone asks on the terminal
        assert_eq!(
            read(&["recover", "ada"]),
            Ok(Args {
                conf: None,
                command: Command::Recover {
                    name: "ada".into(),
                    password_file: None,
                },
            })
        );
        // with both flags, before and after the name, either spelling
        let both = Args {
            conf: Some("r.toml".into()),
            command: Command::Recover {
                name: "ada".into(),
                password_file: Some("-".into()),
            },
        };
        assert_eq!(
            read(&["--conf", "r.toml", "recover", "ada", "--password-file", "-"]),
            Ok(both.clone())
        );
        assert_eq!(
            read(&["recover", "--password-file=-", "ada", "--conf=r.toml"]),
            Ok(both)
        );
        // no name, two names, a flag twice or a flag of its own are refused
        assert!(read(&["recover"]).is_err());
        assert!(read(&["recover", "ada", "bob"]).is_err());
        assert!(
            read(&[
                "recover",
                "ada",
                "--password-file",
                "a",
                "--password-file",
                "b"
            ])
            .is_err()
        );
        assert!(read(&["recover", "ada", "--admin"]).is_err());
        assert!(read(&["recover", "ada", "--password-file"]).is_err());
    }

    /// `version` takes `--json` after it, and nothing else
    #[test]
    fn version_reads_json_alone() {
        // plain, or as JSON
        let version = |json| Args {
            conf: None,
            command: Command::Version { json },
        };
        assert_eq!(read(&["version"]), Ok(version(false)));
        assert_eq!(read(&["version", "--json"]), Ok(version(true)));
        // a config, a flag twice, a flag before it or another command is refused
        assert!(read(&["version", "--conf", "a.toml"]).is_err());
        assert!(read(&["--conf", "a.toml", "version"]).is_err());
        assert!(read(&["version", "--json", "--json"]).is_err());
        assert!(read(&["--json", "version"]).is_err());
        assert!(read(&["version", "recover", "ada"]).is_err());
        assert!(read(&["recover", "ada", "version"]).is_err());
    }

    /// `version --json` prints the schema's id and each table's fingerprint, as its pin has them
    #[test]
    fn version_prints_the_schema_and_each_table() {
        // one object, the id exact past 2^53
        let printed: serde_json::Value = serde_json::from_str(&version(true)).unwrap();
        assert_eq!(printed["schema"].as_u64(), Some(13509127152932721421));
        // each table as a name and its fingerprint, in the schema's order
        let tables = printed["tables"].as_array().unwrap();
        assert_eq!(
            tables[0],
            serde_json::json!(["ProjectRow", 12832186597557680855u64])
        );
        assert_eq!(tables.len(), 16);
        // and the x86-64 features it was compiled with, every x86-64 having sse2
        let features = printed["features"].as_array().unwrap();
        assert!(
            features.contains(&serde_json::json!("sse2")),
            "{features:?}"
        );
        // and the plain form says the same a line each
        let plain = version(false);
        let mut lines = plain.lines();
        assert_eq!(lines.next(), Some("schema 13509127152932721421"));
        assert_eq!(lines.next(), Some("ProjectRow 12832186597557680855"));
        assert_eq!(lines.count(), 15);
    }
}
