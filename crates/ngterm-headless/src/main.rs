//! `ngterm-headless`: ngterm with no window (the crate's docs). It claims the state directory as the app does and
//! exits 1 when another instance owns it, before reading or writing anything there but the log.
//!
//! Each start of a staged build then installs the agent skill and status hooks, or brings them up to the build's
//! bundle, before any pane's shell starts ([`set_up_agents`]): a server has no Help menu or welcome to run them from,
//! and the panes' agents report to this instance only through them. `--skip-agent-setup` leaves the configs alone; a
//! build run from cargo's target directory always does, so a dev run never points the real home's hooks at a
//! checkout's `ngtermctl`.

use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;
use ngterm_app::integrations::hooks::{self, OpenCodeStart};
use ngterm_app::integrations::{Environment, InstallError, opencode, skill};
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
                  Ctrl+Break or close) saves the state and ends it.\n\
                  \n\
                  Each start installs the agent skill and status hooks into this machine's agent and shell configs, \
                  or updates them to this build's, as Help's installers do on the desktop (--skip-agent-setup to \
                  leave them alone)."
)]
struct Cli {
    /// Install or update the agent skill and status hooks in this machine's agent and shell configs, then exit.
    #[arg(long, conflicts_with = "skip_agent_setup")]
    install_agent_hooks: bool,
    /// `OpenCode` version when automatic detection fails; otherwise its hooks are skipped with a notice.
    #[arg(long, value_name = "v1|v2", value_parser = parse_opencode_version, conflicts_with = "skip_agent_setup")]
    opencode_version: Option<opencode::Version>,
    /// Start without installing or updating the agent skill and status hooks.
    #[arg(long)]
    skip_agent_setup: bool,
    /// The grid every terminal starts at, as `COLUMNSxROWS`; an attached viewer's own grid replaces it.
    #[arg(long, value_name = "COLUMNSxROWS", value_parser = parse_grid, default_value = "80x24")]
    grid: (u16, u16),
    /// Started by an instance that quit for a Live sessions reset: wait for its state lock.
    #[arg(long, hide = true)]
    relaunched: bool,
}

fn parse_opencode_version(value: &str) -> Result<opencode::Version, String> {
    match value {
        "v1" => Ok(opencode::Version::V1),
        "v2" => Ok(opencode::Version::V2),
        _ => Err("expected v1 or v2".to_owned()),
    }
}

/// What [`install_agent_integrations`] did: each installer's result (a warning, its title, its text), whether
/// `OpenCode`'s hooks were skipped for want of its version, and why the hooks install failed, when it did (the skill's
/// result stands either way).
struct Installed {
    results: Vec<(bool, &'static str, String)>,
    opencode_skipped: bool,
    hooks_failed: Option<InstallError>,
}

/// The notice for `OpenCode` hooks skipped because its version could not be detected.
const OPENCODE_SKIPPED: &str =
    "OpenCode version could not be detected; skipping its hooks. Pass --opencode-version v1 or v2 to choose it.";

/// The desktop Help menu's skill and hooks installers, without a display or a running instance: the skill into each
/// coding agent's skills directory, then the hooks package and every agent's wiring. Both are idempotent and leave
/// what belongs to the user alone, so a run over a current install changes nothing.
///
/// # Errors
///
/// No home directory to install into: nothing was installed.
fn install_agent_integrations(version: Option<opencode::Version>) -> Result<Installed, InstallError> {
    let mut env = Environment::process()
        .ok_or_else(|| InstallError("No home directory was found to install into.".to_owned()))?;
    // The login-shell probe must not inherit the caller's terminal identity or reach its control socket.
    env.vars.retain(|key, _| !key.starts_with("NGTERM_"));
    let mut results = Vec::new();
    match skill::plan(&env) {
        Ok(plan) => {
            let report = skill::apply(&plan);
            results.push((report.is_warning(), report.title(), report.text()));
        }
        Err(error) => results.push((true, "Skill Not Installed", error.0)),
    }
    let probe = opencode::detect(&env.vars, &env.home_str());
    let start = match version {
        Some(version) => OpenCodeStart::Install(version),
        None => hooks::opencode_start(&env.home, probe.as_ref()),
    };
    let opencode_skipped = start == OpenCodeStart::Ask;
    let hooks_failed = match hooks::plan(&env, &start, probe.as_ref()).and_then(|plan| hooks::apply(&plan)) {
        Ok(outcome) => {
            results.push((outcome.is_warning(), outcome.title(), outcome.text()));
            None
        }
        Err(error) => Some(error),
    };
    Ok(Installed {
        results,
        opencode_skipped,
        hooks_failed,
    })
}

/// `--install-agent-hooks`: the installers' results on standard output; a hooks failure is the error.
fn install_agent_hooks(version: Option<opencode::Version>) -> Result<(), InstallError> {
    let installed = install_agent_integrations(version)?;
    if installed.opencode_skipped {
        eprintln!("{OPENCODE_SKIPPED}");
    }
    let texts: Vec<String> = installed
        .results
        .iter()
        .map(|(_, title, text)| format!("{title}\n\n{text}"))
        .collect();
    println!("{}", texts.join("\n\n"));
    installed.hooks_failed.map_or(Ok(()), Err)
}

/// Whether a start installs or updates the agent skill and hooks: from a staged build, unless asked not to. A relaunch
/// does too, since it may run an upgraded binary ([`relaunch::target`]), and it is given the same flags.
fn sets_up_agents(options: &Options, staged: bool) -> bool {
    !options.skip_agent_setup && staged
}

/// A start's install or update of the agent skill and hooks, into the log: a failure there does not stop the server.
fn set_up_agents(version: Option<opencode::Version>) {
    match install_agent_integrations(version) {
        Ok(installed) => {
            for (warning, title, text) in installed.results {
                if warning {
                    tracing::warn!("{title}: {text}");
                } else {
                    tracing::info!("{title}: {text}");
                }
            }
            if installed.opencode_skipped {
                tracing::warn!("{OPENCODE_SKIPPED}");
            }
            if let Some(error) = installed.hooks_failed {
                tracing::error!(error = %error.0, "the agent status hooks were not installed");
            }
        }
        Err(error) => tracing::error!(error = %error.0, "the agent skill and hooks were not installed"),
    }
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
    if cli.install_agent_hooks {
        return match install_agent_hooks(cli.opencode_version) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("ngterm-headless: {}", error.0);
                ExitCode::FAILURE
            }
        };
    }
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
    let options = Options {
        grid: cli.grid,
        skip_agent_setup: cli.skip_agent_setup,
        opencode_version: cli.opencode_version,
    };
    // before any pane's shell starts, so a first start's shells already source the shell integration
    if sets_up_agents(&options, Environment::process_is_staged()) {
        set_up_agents(options.opencode_version);
    } else {
        tracing::info!("the agent skill and hooks are left as they are");
    }
    let status = ngterm_headless::run(&options);
    if status == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser as _;

    use super::{Cli, Options, parse_grid, relaunch, sets_up_agents};

    #[test]
    fn a_grid_is_columns_by_rows() {
        assert_eq!(parse_grid("120x40"), Ok((120, 40)));
        assert_eq!(parse_grid("80X24"), Ok((80, 24)));
        assert!(parse_grid("120").is_err());
        assert!(parse_grid("1x24").is_err());
        assert!(parse_grid("axb").is_err());
    }

    #[test]
    fn hooks_install_options_are_explicit_and_validate_the_opencode_version() {
        let cli = Cli::try_parse_from(["ngterm-headless", "--install-agent-hooks"]).unwrap();
        assert!(cli.install_agent_hooks);
        assert!(cli.opencode_version.is_none());
        for version in ["v1", "v2"] {
            let cli = Cli::try_parse_from([
                "ngterm-headless",
                "--install-agent-hooks",
                "--opencode-version",
                version,
            ])
            .unwrap();
            assert_eq!(cli.opencode_version.unwrap().raw(), version);
        }
        assert!(Cli::try_parse_from(["ngterm-headless", "--install-agent-hooks", "--opencode-version", "v3"]).is_err());
        assert!(Cli::try_parse_from(["ngterm-headless", "--install-agent-hooks", "--skip-agent-setup"]).is_err());
    }

    #[test]
    fn a_staged_start_sets_up_the_agents_unless_skipped() {
        let parse = |arguments: &[&str]| {
            let cli = Cli::try_parse_from([&["ngterm-headless"], arguments].concat()).unwrap();
            Options {
                grid: cli.grid,
                skip_agent_setup: cli.skip_agent_setup,
                opencode_version: cli.opencode_version,
            }
        };
        let start = parse(&[]);
        assert!(sets_up_agents(&start, true));
        assert!(
            !sets_up_agents(&start, false),
            "a cargo-target build leaves the configs alone"
        );
        let pinned = parse(&["--opencode-version", "v2"]);
        assert!(sets_up_agents(&pinned, true));
        assert_eq!(pinned.opencode_version.unwrap().raw(), "v2");
        assert!(!sets_up_agents(&parse(&["--skip-agent-setup"]), true));
        assert!(
            sets_up_agents(&parse(&["--relaunched"]), true),
            "a relaunch may be an upgraded binary"
        );
        assert!(Cli::try_parse_from(["ngterm-headless", "--skip-agent-setup", "--opencode-version", "v1"]).is_err());
    }

    #[test]
    fn a_relaunch_starts_with_the_same_grid_and_waits_for_the_lock() {
        let mut arguments = vec!["ngterm-headless".into()];
        arguments.extend(relaunch::arguments(&Options {
            grid: (120, 40),
            skip_agent_setup: true,
            ..Options::default()
        }));
        let cli = Cli::try_parse_from(arguments).unwrap();
        assert_eq!(
            (cli.grid, cli.relaunched, cli.skip_agent_setup),
            ((120, 40), true, true)
        );
        let cli = Cli::try_parse_from(["ngterm-headless"]).unwrap();
        assert_eq!((cli.grid, cli.relaunched), ((80, 24), false));
    }
}
