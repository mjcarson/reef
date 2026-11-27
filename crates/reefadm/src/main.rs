//! reefadm: central reef deployed, upgraded and run from one command
//!
//! ```sh
//! reefadm deploy                    # a team server on this host
//! reefadm deploy --user             # one person's user unit on loopback
//! reefadm --dry-run deploy          # every step it would take, taking none
//! ```

use clap::Parser;
use reefadm::exec::{Dry, Real};
use reefadm::{Cli, Env};
use std::process::ExitCode;
use std::time::SystemTime;

/// Runs one command, printing its error and exiting 1 if it fails
fn main() -> ExitCode {
    // clap prints usage and exits 2 on a command line it can't read
    let cli = Cli::parse();
    let env = Env::from_process();
    let real = Real::new(&env);
    // a dry run runs the same steps through a runner that only shows changes
    let done = match cli.dry_run {
        true => reefadm::run(cli, &mut Dry { inner: real }, env, SystemTime::now()),
        false => {
            let mut real = real;
            reefadm::run(cli, &mut real, env, SystemTime::now())
        }
    };
    match done {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("reefadm: {error:#}");
            ExitCode::FAILURE
        }
    }
}
