//! The shell a user's command line runs through, chosen here once for every caller: a hook's (`hooks.conf`) and a
//! custom command's (`keymap.conf`) in `ngterm-core`, a pane's command in `ngterm-term`'s launch, and the overlay
//! job's program in `ngtermctl`.
//!
//! - **Unix**: `/bin/sh -c <line>`, as upstream runs a hook (`HookProcessRunner`), a custom command
//!   (`CustomCommandRunner.spawn`) and libghostty a surface's command.
//! - **Windows**: `%ComSpec% /D /S /C "<line>"` (`cmd.exe`, found on `PATH`, when `ComSpec` names none): the line is
//!   `cmd`'s command text, its variables `%NAME%` where `sh` reads `$NAME`.
//!   - Why `cmd` and not PowerShell: `ComSpec` is the system's own command interpreter, present on every Windows, and
//!     the one a configured command line runs through elsewhere: Node's `child_process` with `shell: true` (npm's
//!     scripts with it), and Ghostty's command on Windows. It starts in milliseconds where PowerShell takes hundreds
//!     (a hook runs on every event), runs a `program args` line and `a && b` chains as written, and a PowerShell line
//!     still runs through it as `pwsh -c "…"`.
//!   - `/D` skips the `AutoRun` commands, as `sh -c` reads no rc file. `/S` with the outer quotes makes `cmd` strip
//!     exactly the first and the last quote and keep the rest as written, so a quoted program path and quoted
//!     arguments survive.
//!   - The quoted line is the command line's tail as written ([`ShellCommand::raw_tail`], `CommandExt::raw_arg`),
//!     never through the C runtime's argument quoting, which would escape its quotes as `\"`, which `cmd` does not
//!     read.

use std::ffi::OsStr;
use std::path::PathBuf;

/// The POSIX shell a command line runs through on Unix.
pub const POSIX_SHELL: &str = "/bin/sh";
/// The Windows command interpreter when `ComSpec` names none.
pub const CMD_SHELL: &str = "cmd.exe";
/// `cmd`'s switches before a command line: no `AutoRun`, and exactly the outer quotes stripped.
pub const CMD_SWITCHES: [&str; 3] = ["/D", "/S", "/C"];

/// What runs one command line: the program, its arguments, and on Windows the quoted line passed as written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellCommand {
    /// The shell.
    pub program: PathBuf,
    /// Its arguments, quoted as the platform quotes arguments.
    pub args: Vec<String>,
    /// Appended to the command line as written after [`ShellCommand::args`] (Windows' `CommandExt::raw_arg`); `None`
    /// on Unix.
    pub raw_tail: Option<String>,
}

impl ShellCommand {
    /// This OS's run of `line`, `ComSpec` read from the app's environment on Windows.
    #[must_use]
    pub fn new(line: &str) -> Self {
        if cfg!(windows) {
            Self::cmd(line, std::env::var_os("ComSpec").as_deref())
        } else {
            Self::posix(line)
        }
    }

    /// `/bin/sh -c <line>`.
    #[must_use]
    pub fn posix(line: &str) -> Self {
        Self {
            program: POSIX_SHELL.into(),
            args: vec!["-c".to_owned(), line.to_owned()],
            raw_tail: None,
        }
    }

    /// `<comspec> /D /S /C "<line>"`, with `cmd.exe` for a missing or empty `comspec`.
    #[must_use]
    pub fn cmd(line: &str, comspec: Option<&OsStr>) -> Self {
        let program = comspec
            .filter(|comspec| !comspec.is_empty())
            .map_or_else(|| PathBuf::from(CMD_SHELL), PathBuf::from);
        Self {
            program,
            args: CMD_SWITCHES.iter().map(|switch| (*switch).to_owned()).collect(),
            raw_tail: Some(format!("\"{line}\"")),
        }
    }

    /// The whole command line as one string, for a report naming what ran (no quoting added to the arguments).
    #[must_use]
    pub fn display(&self) -> String {
        let mut words = vec![self.program.to_string_lossy().into_owned()];
        words.extend(self.args.iter().cloned());
        words.extend(self.raw_tail.iter().cloned());
        words.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unix_runs_the_line_through_sh() {
        let shell = ShellCommand::posix("printf '%s' \"$AGT_PANE\"");
        assert_eq!(shell.program, PathBuf::from("/bin/sh"));
        assert_eq!(shell.args, ["-c", "printf '%s' \"$AGT_PANE\""]);
        assert_eq!(shell.raw_tail, None);
        assert_eq!(shell.display(), "/bin/sh -c printf '%s' \"$AGT_PANE\"");
    }

    #[test]
    fn windows_runs_the_quoted_line_through_comspec() {
        let line = r#""C:\Program Files\tool.exe" "a b" > "%TEMP%\out""#;
        let shell = ShellCommand::cmd(line, Some(OsStr::new(r"C:\Windows\system32\cmd.exe")));
        assert_eq!(shell.program, PathBuf::from(r"C:\Windows\system32\cmd.exe"));
        assert_eq!(shell.args, ["/D", "/S", "/C"]);
        assert_eq!(shell.raw_tail.as_deref(), Some(format!("\"{line}\"").as_str()));
        assert_eq!(
            shell.display(),
            format!(r#"C:\Windows\system32\cmd.exe /D /S /C "{line}""#)
        );
    }

    #[test]
    fn windows_without_comspec_runs_cmd_from_path() {
        for comspec in [None, Some(OsStr::new(""))] {
            assert_eq!(ShellCommand::cmd("ver", comspec).program, PathBuf::from("cmd.exe"));
        }
    }

    #[test]
    fn this_os_picks_its_own_form() {
        let shell = ShellCommand::new("exit 3");
        if cfg!(windows) {
            assert_eq!(shell.args, CMD_SWITCHES);
            assert_eq!(shell.raw_tail.as_deref(), Some("\"exit 3\""));
        } else {
            assert_eq!(shell, ShellCommand::posix("exit 3"));
        }
    }
}
