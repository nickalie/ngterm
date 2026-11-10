//! `ngterm-headless`: ngterm with no window (the crate's docs). It claims the state directory as the app does and
//! exits 1 when another instance owns it, before reading or writing anything there but the log.
//!
//! Each start of a staged build then installs the agent skill and status hooks, or brings them up to the build's
//! bundle, before any pane's shell starts ([`set_up_agents`]): a server has no Help menu or welcome to run them from,
//! and the panes' agents report to this instance only through them. `--skip-agent-setup` leaves the configs alone; a
//! build run from cargo's target directory always does, so a dev run never points the real home's hooks at a
//! checkout's `ngtermctl`. On Windows a staged start also puts its own folder on the user's `PATH`,
//! whatever `--skip-agent-setup` says ([`put_cli_on_path`]).

use std::ffi::OsString;
use std::path::Path;
use std::process::ExitCode;
use std::time::Instant;

use clap::Parser;
#[cfg(windows)]
use ngterm_app::integrations::cli;
use ngterm_app::integrations::hooks::{self, OpenCodeStart};
use ngterm_app::integrations::{Environment, InstallError, opencode, skill};
use ngterm_core::paths::{self, ProcessEnv};
use ngterm_core::state_lock::StateDirectoryLock;
use ngterm_headless::{Options, relaunch};

/// ngterm with no window: its sessions, control socket and live sessions, for Attach Remote from another machine.
#[derive(Debug, Parser)]
#[expect(clippy::struct_excessive_bools, reason = "each is a command-line flag of its own")]
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
                  Ctrl+Break or close) saves the state and ends it. --detach starts it in the background, outliving \
                  the session that ran it (an ssh login, say); on Windows, where an ssh session takes everything \
                  started in it down with it, that is how a server is started over ssh.\n\
                  \n\
                  Each start installs the agent skill and status hooks into this machine's agent and shell configs, \
                  or updates them to this build's, as Help's installers do on the desktop (--skip-agent-setup to \
                  leave them alone). On Windows a start of a staged build also puts its own folder on the user's \
                  PATH, so another machine's ssh finds ngtermctl there, unless NGTERM_STATE_DIR names a private \
                  state directory."
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
    /// Start in the background and return: the server outlives this command and the session that ran it.
    #[arg(long, conflicts_with = "install_agent_hooks")]
    detach: bool,
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

/// Whether a start puts its own folder on the user's `PATH`, so a viewer's ssh finds `ngtermctl` there (an installer's
/// `PATH` entry has it already; a portable folder has none): with a bundled CLI, and not on a private state directory,
/// a dev run or a test, which leaves the user's real `PATH` as it is. An empty variable counts as unset, as
/// [`paths::state_dir`] takes it, and a Windows variable's name is in any case.
#[cfg(windows)]
fn puts_the_cli_on_path(vars: &std::collections::HashMap<String, String>, bundled_cli: bool) -> bool {
    bundled_cli
        && !vars
            .iter()
            .any(|(name, value)| name.eq_ignore_ascii_case(paths::STATE_DIR_ENV) && !value.is_empty())
}

/// A staged start's part in being reachable: its own folder on the user's `PATH`, into the log. Not an agent setting,
/// so `--skip-agent-setup` does not skip it.
#[cfg(windows)]
fn put_cli_on_path() {
    let Some(env) = Environment::process() else {
        return;
    };
    if !puts_the_cli_on_path(&env.vars, env.bundled_cli.is_some()) {
        return;
    }
    let result = cli::apply(&cli::plan(&env), None);
    if result.is_warning() {
        tracing::warn!("{}: {}", result.title(), result.text());
    } else {
        tracing::info!("{}: {}", result.title(), result.text());
    }
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

/// The command line of the background server `--detach` starts: this one's, without the flag.
fn detached_arguments(arguments: impl Iterator<Item = OsString>) -> Vec<OsString> {
    arguments.filter(|word| word != "--detach").collect()
}

/// `--detach`: starts this command again as a background server, says which and exits. The instance's own failures
/// (another instance on the state directory, say) go to its log, as they do when it starts in the foreground.
fn detach() -> ExitCode {
    let arguments = detached_arguments(std::env::args_os().skip(1));
    match std::env::current_exe().and_then(|exe| ngterm_session::server::spawn::spawn_detached(&exe, &arguments)) {
        Ok(pid) => {
            println!("ngterm-headless: started in the background (pid {pid})");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("ngterm-headless: could not start in the background: {error}");
            ExitCode::FAILURE
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.detach {
        return detach();
    }
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
    let staged = Environment::process_is_staged();
    if sets_up_agents(&options, staged) {
        set_up_agents(options.opencode_version);
    } else {
        tracing::info!("the agent skill and hooks are left as they are");
    }
    #[cfg(windows)]
    if staged {
        put_cli_on_path();
    }
    let status = ngterm_headless::run(&options);
    if status == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
#[path = "main_tests.rs"]
mod tests;
