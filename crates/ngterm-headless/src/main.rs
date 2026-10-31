//! `ngterm-headless`: ngterm with no window (the crate's docs). It claims the state directory as the app does and
//! exits 1 when another instance owns it, before reading or writing anything there but the log.

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;
use ngterm_core::paths::{self, ProcessEnv};
use ngterm_core::state_lock::StateDirectoryLock;
use ngterm_headless::{Options, relaunch};

/// ngterm with no window: its sessions, control socket and live sessions, for Attach Remote from another machine.
#[derive(Debug, Parser)]
#[command(
    name = "ngterm-headless",
    version,
    long_about = "Runs ngterm with no window, on a machine without a display: the sessions of its state directory \
                  (NGTERM_STATE_DIR, else the platform's), each pane in a live session, and the control socket \
                  ngtermctl drives. Another machine's ngterm lists and attaches them through File ▸ Attach Remote \
                  (ssh, key-based, to this machine), as it does a desktop instance's.\n\
                  \n\
                  Create sessions with `ngtermctl session new`; they keep running when this process ends, and the \
                  next start attaches them again. SIGTERM, SIGINT or SIGHUP (on Windows the console's Ctrl+C, \
                  Ctrl+Break or close) saves the state and ends it."
)]
struct Cli {
    /// The grid every terminal starts at, as `COLUMNSxROWS`; an attached viewer's own grid replaces it.
    #[arg(long, value_name = "COLUMNSxROWS", value_parser = parse_grid, default_value = "80x24")]
    grid: (u16, u16),
    /// Started by an instance that quit for a Live sessions reset: wait for its state lock.
    #[arg(long, hide = true)]
    relaunched: bool,
}

fn parse_grid(value: &str) -> Result<(u16, u16), String> {
    let (columns, rows) = value
        .split_once(['x', 'X'])
        .ok_or_else(|| "expected COLUMNSxROWS, as 120x40".to_owned())?;
    let parse = |part: &str| {
        part.trim()
            .parse::<u16>()
            .ok()
            .filter(|count| *count >= 2)
            .ok_or_else(|| format!("{part:?} is not a count of 2 or more"))
    };
    Ok((parse(columns)?, parse(rows)?))
}

/// The state directory's lock; a relaunched instance waits for its predecessor to release it.
fn claim(state_dir: &Path, relaunched: bool) -> Result<StateDirectoryLock, String> {
    let deadline = Instant::now() + relaunch::LOCK_WAIT;
    loop {
        match StateDirectoryLock::acquire(state_dir).claim(state_dir) {
            Err(_) if relaunched && Instant::now() < deadline => std::thread::sleep(relaunch::LOCK_POLL),
            claimed => return claimed,
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    ngterm_app::logging::init(&ProcessEnv);
    let state_dir = paths::state_dir(&ProcessEnv);
    // held for the whole process; the kernel releases it at exit
    let _lock = match claim(&state_dir, cli.relaunched) {
        Ok(lock) => lock,
        Err(reason) => {
            tracing::error!("{reason}");
            eprintln!("ngterm-headless: {reason}");
            return ExitCode::FAILURE;
        }
    };
    let status = ngterm_headless::run(&Options { grid: cli.grid });
    if status == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::{Cli, Options, parse_grid, relaunch};

    #[test]
    fn a_grid_is_columns_by_rows() {
        assert_eq!(parse_grid("120x40"), Ok((120, 40)));
        assert_eq!(parse_grid("80X24"), Ok((80, 24)));
        assert!(parse_grid("120").is_err());
        assert!(parse_grid("1x24").is_err());
        assert!(parse_grid("axb").is_err());
    }

    #[test]
    fn a_relaunch_starts_with_the_same_grid_and_waits_for_the_lock() {
        let mut arguments = vec!["ngterm-headless".into()];
        arguments.extend(relaunch::arguments(&Options { grid: (120, 40) }));
        let cli = Cli::try_parse_from(arguments).unwrap();
        assert_eq!((cli.grid, cli.relaunched), ((120, 40), true));
        let cli = Cli::try_parse_from(["ngterm-headless"]).unwrap();
        assert_eq!((cli.grid, cli.relaunched), ((80, 24), false));
    }
}
