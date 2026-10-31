//! The process table a pane's foreground is read through ([`ngterm_host::ProcessTable`]) on every OS, standing in for
//! upstream's `sysctl` reads in `Ghostty/ForegroundProcess.swift`: the quit-time restore capture, the `tree`'s
//! foreground fields and a live restart's working directory.
//!
//! - Linux ([`linux`]): `/proc/<pid>/cmdline` for an argv, `/proc/<pid>/stat` for the ids and the terminal's
//!   foreground group ([`crate::procfs`]), a scan of every stat line for a group's members, `/proc/<pid>/cwd`.
//! - macOS: `sysctl(KERN_PROCARGS2)` for an argv ([`parse_procargs`], upstream's), `sysctl(KERN_PROC_PID)`'s
//!   `kinfo_proc` for the ids and the terminal's foreground group, `sysctl(KERN_PROC_PGRP)` for a group's members
//!   (upstream's `processGroup`), `getsid` for the session, `proc_pidinfo(PROC_PIDVNODEPATHINFO)` for the working
//!   directory (upstream's `workingDirectory`).
//! - Windows: `NtQueryInformationProcess(ProcessCommandLineInformation)` split with `CommandLineToArgvW` for an argv
//!   (herdr's), a tool-help snapshot for the parent, the process environment block's current directory for the
//!   working directory (herdr's). Windows has neither process groups nor sessions, so they are modelled for core's
//!   foreground reads (`ngterm_core::restore::capture`): a process stands in for its own group, as in herdr, which
//!   holds it and its live descendants (a shell without job control keeps its children in its group); a process this
//!   one started (the app's pane program on its pseudoconsole) leads its session, as a terminal's spawned child does
//!   on Unix, and any other's session is zero. A console has no foreground group either; [`console_foreground`]
//!   stands in for it, over an in-app pane's job ([`job_foreground`]) or a live pane's shell's descendants
//!   ([`ProcessTable::terminal_foreground`](ngterm_host::ProcessTable::terminal_foreground)).
//!
//! An argv is decoded lossily everywhere, so an undecodable byte (or an unpaired UTF-16 surrogate) becomes U+FFFD,
//! which the restore then refuses to replay, as upstream's lossy decode does.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub use linux::ProcTable;
#[cfg(target_os = "macos")]
pub use macos::ProcTable;
#[cfg(windows)]
pub use windows::{ProcTable, job_foreground};

use ngterm_core::command_restore::{is_command_wrapper, is_idle_shell};

/// Upstream's cap on a `KERN_PROCARGS2` argc (`CommandRestore.maxArgCount`): anything above is a corrupt blob.
const MAX_ARG_COUNT: i32 = 4096;

/// The argv in a macOS `KERN_PROCARGS2` blob (upstream's `CommandRestore.parseProcArgs`): a host-order `i32` argc,
/// the NUL-terminated executable path, zero or more NUL padding bytes, then `argc` NUL-terminated arguments (the
/// environment follows, ignored), each decoded lossily. `None` for a truncated or implausible blob.
#[must_use]
pub fn parse_procargs(bytes: &[u8]) -> Option<Vec<String>> {
    if bytes.len() <= 4 {
        return None;
    }
    let count = i32::from_ne_bytes(bytes[..4].try_into().ok()?);
    if count <= 0 || count >= MAX_ARG_COUNT {
        return None;
    }
    let count = usize::try_from(count).ok()?;
    let mut rest = &bytes[4..];
    // skip the executable path up to its NUL, then the NUL padding before argv[0]
    let path_end = rest.iter().position(|byte| *byte == 0).unwrap_or(rest.len());
    rest = &rest[path_end..];
    let padding = rest.iter().position(|byte| *byte != 0).unwrap_or(rest.len());
    rest = &rest[padding..];
    let mut argv = Vec::with_capacity(count);
    while argv.len() < count && !rest.is_empty() {
        let end = rest.iter().position(|byte| *byte == 0).unwrap_or(rest.len());
        argv.push(String::from_utf8_lossy(&rest[..end]).into_owned());
        rest = rest.get(end + 1..).unwrap_or_default();
    }
    (argv.len() == count).then_some(argv)
}

/// One process of a console, as [`console_foreground`] weighs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsoleMember {
    pub pid: i32,
    /// Its creation time, in any unit that orders the members.
    pub started: u64,
    /// Its argv; `None` when it cannot be read (gone, or protected).
    pub argv: Option<Vec<String>>,
}

/// What stands in on Windows for the foreground group of the console `shell` holds, among `members` (the processes
/// of that console: an in-app pane's job, a live pane's shell's descendants), `shell` included:
///
/// - a `shell` that is a command wrapper (`cmd.exe /D /S /C`, [`is_command_wrapper`]) has no job control, so it holds
///   the foreground over whatever it runs, as `/bin/sh -c` leads the group of every child on Unix;
/// - else the OLDEST member started since the shell whose argv is readable and no idle shell ([`is_idle_shell`]):
///   the first program of what the shell runs, as the job's leader is on Unix (`git log`, not the pager it starts; an
///   agent, not the tools it runs);
/// - else the shell itself, which holds its own foreground as a Unix shell at its prompt does.
///
/// An idle shell never stands in, so a launcher's shell (Git for Windows' `bin\sh.exe` runs `usr\bin\bash.exe`),
/// an MSYS shell's fork (a copy of the shell, which then starts the program) and a shell typed into the pane read as
/// the pane's shell. A member older than the shell holds a pid the shell's parent once had. `None` when `shell` is
/// not among `members`.
///
/// A console has no job control, so a program left running in the background (`sleep 600 &`) reads as the
/// foreground until it ends, and so does a GUI program the shell started.
#[must_use]
pub fn console_foreground(shell: i32, members: &[ConsoleMember]) -> Option<i32> {
    let root = members.iter().find(|member| member.pid == shell)?;
    if root.argv.as_deref().is_some_and(is_command_wrapper) {
        return Some(shell);
    }
    let oldest = members
        .iter()
        .filter(|member| member.pid != shell && member.started >= root.started)
        .filter(|member| {
            member
                .argv
                .as_deref()
                .is_some_and(|argv| !argv.is_empty() && !is_idle_shell(argv, None))
        })
        .min_by_key(|member| (member.started, member.pid));
    Some(oldest.map_or(shell, |member| member.pid))
}
