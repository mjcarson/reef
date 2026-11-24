//! reef's external shoal node
//!
//! ```sh
//! cargo +nightly run --release -p reef-shoal -- serve --conf shoal.yml
//! ```
//!
//! The program is `shoal::server::node::main` over reef's schema, so a reef-service client and
//! this node agree on the fingerprint their hello checks. Point reef at it with `[shoal] address`
//! in reef's config. Before shoal reads its config, reef checks it, and refuses one shoal would
//! fail on obscurely, naming what to change.

use reef_service::checks;
use reef_service::schema::Reef;
use std::ffi::OsString;
use std::path::PathBuf;
use std::process::ExitCode;

/// The config shoal's node program reads when none is named
const DEFAULT_CONF: &str = "shoal.yml";

/// The exit code for a config refused before shoal read it
const REFUSED: u8 = 2;

/// Checks the node's config, then serves reef's schema until told to stop
fn main() -> ExitCode {
    // shoal's shards get the stack reef's schema needs, before any thread starts
    // SAFETY: nothing has started a thread yet
    unsafe { reef_service::pool::widen_stacks() };
    // check the config `serve` or `claim` would read, if it names one that exists
    if let Some(conf) = conf_path(std::env::args_os().skip(1)).filter(|conf| conf.is_file()) {
        match checks::check_file(&conf) {
            Ok(problems) => {
                // say every problem, and stop before shoal starts on a fatal one
                for problem in &problems {
                    let level = if problem.fatal { "error" } else { "warning" };
                    eprintln!(
                        "reef-shoal: {level}: {}\n  fix: {}",
                        problem.what, problem.fix
                    );
                }
                if problems.iter().any(|problem| problem.fatal) {
                    return ExitCode::from(REFUSED);
                }
            }
            // a config that can't be read is shoal's to refuse, by its own message
            Err(error) => eprintln!("reef-shoal: could not check {}: {error:#}", conf.display()),
        }
    }
    // shoal's node program: `serve` or `claim`, each with `--conf`
    match shoal::server::node::main::<Reef>() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Error: {error:?}");
            ExitCode::FAILURE
        }
    }
}

/// The config a `serve` or `claim` command reads, or `None` for any other command
///
/// This peeks at the arguments shoal's own parser will read, and anything it doesn't follow is
/// left to that parser.
///
/// # Arguments
///
/// * `args` - The arguments after the program's name
fn conf_path(mut args: impl Iterator<Item = OsString>) -> Option<PathBuf> {
    // only `serve` and `claim` read a config
    let command = args.next()?;
    if command != "serve" && command != "claim" {
        return None;
    }
    // `--conf X` or `--conf=X`, else the default beside the working directory
    while let Some(arg) = args.next() {
        if arg == "--conf" {
            return args.next().map(PathBuf::from);
        }
        if let Some(path) = arg.to_str().and_then(|arg| arg.strip_prefix("--conf=")) {
            return Some(PathBuf::from(path));
        }
    }
    Some(PathBuf::from(DEFAULT_CONF))
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
    fn path(args: &[&str]) -> Option<PathBuf> {
        conf_path(args.iter().map(OsString::from))
    }

    /// Both spellings of `--conf`, the default, and commands that read no config
    #[test]
    fn conf_path_follows_shoals_arguments() {
        assert_eq!(path(&["serve", "--conf", "a.yml"]), Some("a.yml".into()));
        assert_eq!(path(&["claim", "--conf=b.yml"]), Some("b.yml".into()));
        assert_eq!(path(&["serve"]), Some("shoal.yml".into()));
        assert_eq!(path(&["--help"]), None);
        assert_eq!(path(&[]), None);
    }
}
