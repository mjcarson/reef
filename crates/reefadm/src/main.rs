//! reefadm: central reef deployed, upgraded and run from one command
//!
//! ```sh
//! reefadm deploy                    # a team server on this host
//! reefadm deploy --user             # one person's user unit on loopback
//! reefadm --host europa deploy      # the same, on another host over ssh
//! reefadm --dry-run deploy          # every step it would take, taking none
//! ```

use clap::Parser;
use reefadm::exec::{Dry, Real};
use reefadm::{Cli, Env, Exit};
use std::process::ExitCode;
use std::time::SystemTime;

/// Runs one command, printing its error and exiting 1 if it fails, or as another host's ended
fn main() -> ExitCode {
    // clap prints usage and exits 2 on a command line it can't read
    let cli = Cli::parse();
    let env = Env::from_process();
    let mut real = Real::new(&env);
    let now = SystemTime::now();
    // another host's command runs there as given; a dry run here shows changes it won't make
    let done = match (&cli.host, cli.dry_run) {
        (Some(host), _) => {
            let args: Vec<String> = std::env::args().skip(1).collect();
            let host = host.clone();
            reefadm::run_on(&host, &args, cli.yes, &mut real, env, now)
        }
        (None, true) => reefadm::run(cli, &mut Dry { inner: real }, env, now),
        (None, false) => reefadm::run(cli, &mut real, env, now),
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        // another host's reefadm said why already
        Err(error) if error.downcast_ref::<Exit>().is_some() => {
            ExitCode::from(error.downcast_ref::<Exit>().map_or(1, |exit| exit.0))
        }
        Err(error) => {
            eprintln!("reefadm: {error:#}");
            ExitCode::FAILURE
        }
    }
}
