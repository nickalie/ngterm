//! Updates for installations the system owns: `.deb`, `.rpm` and Arch packages and the root-owned macOS PKG. The
//! system's own installer runs with administrator rights, so its package records stay correct, while the app is still
//! open: a dismissed password prompt changes nothing. Rights come from the desktop's password popup; a Linux desktop
//! with no way to show one runs `sudo` in a terminal pane instead ([`PrivilegedInstall::terminal`]).
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use sha2::{Digest as _, Sha256};

use crate::{Error, error};

/// How long a terminal pane may take to start its script before the install counts as abandoned.
const PANE_START_LIMIT: Duration = Duration::from_secs(30);
const PANE_PID: &str = "terminal-pid";
const PANE_RESULT: &str = "terminal-result";
const PANE_SCRIPT_FILE: &str = "install-in-terminal.sh";
const ROOT_SCRIPT_FILE: &str = "install-as-root.sh";
const MAC_PACKAGE: &str = "pkg";

/// What pkexec's own failures say when no popup can be shown. pkexec does not translate these.
const NO_PROMPT_ERRORS: [&str; 3] = [
    "No authentication agent",
    "Error getting authority",
    "must be setuid root",
];

// Runs as root over a package in a directory its user can write: the copy in a root-owned directory is what gets
// hashed and installed. The popup routes hand this script and the checksums over from memory; the `sudo` route keeps
// them in the stage, trusting the user's own account as any `sudo` in that account's terminal does. The identity is
// compared last, since a prompt can wait indefinitely and `pacman -U` would downgrade a newer installation.
// Arguments are positional, never interpolated into shell code.
pub(crate) const ROOT_SCRIPT: &str = r#"set -eu
package=$1; expected=$2; kind=$3; identity=$4; installed=$5
digest() {
  if command -v sha256sum >/dev/null 2>&1; then sum=$(sha256sum "$1"); else sum=$(shasum -a 256 "$1"); fi
  echo "${sum%% *}"
}
stage=$(mktemp -d /var/tmp/ngterm-update.XXXXXX)
trap 'rm -rf "$stage"' EXIT
file="$stage/${package##*/}"
cp "$package" "$file"
if [ "$(digest "$file")" != "$expected" ]; then
  echo 'The downloaded package changed after it was verified.' >&2
  exit 3
fi
chmod 755 "$stage"
chmod 644 "$file"
if [ "$(digest "$identity")" != "$installed" ]; then
  echo 'The installed version changed after the download started. Check for updates again.' >&2
  exit 4
fi
case "$kind" in
  deb) DEBIAN_FRONTEND=noninteractive apt-get install -y -o Dpkg::Options::=--force-confdef -o Dpkg::Options::=--force-confold "$file" ;;
  rpm)
    if command -v dnf >/dev/null 2>&1; then dnf install -y "$file"
    elif command -v zypper >/dev/null 2>&1; then zypper --non-interactive --no-gpg-checks install --allow-unsigned-rpm "$file"
    else rpm -U "$file"
    fi ;;
  pacman) pacman -U --noconfirm "$file" ;;
  pkg) /usr/sbin/installer -pkg "$file" -target / ;;
  *) echo "Unknown package kind: $kind" >&2; exit 2 ;;
esac
"#;

// The result file is written whole, so a reader never sees half of it. A failure holds the pane so its output can be read.
const PANE_SCRIPT: &str = r#"work=${0%/*}
echo "$$" > "$work/terminal-pid"
echo 'ngterm needs administrator rights to install this update.'
echo
code=0
sudo /bin/sh "$work/install-as-root.sh" "$@" || code=$?
echo "$code" > "$work/terminal-result.tmp"
mv "$work/terminal-result.tmp" "$work/terminal-result"
if [ "$code" != 0 ]; then
  echo
  echo 'The update was not installed. Press Enter to close this pane.'
  read -r _
fi
"#;

const MAC_RUN: &str = r#"do shell script "/bin/sh -c " & quoted form of item 1 of argv & " ngterm-update " & quoted form of item 2 of argv & " " & quoted form of item 3 of argv & " " & quoted form of item 4 of argv & " " & quoted form of item 5 of argv & " " & quoted form of item 6 of argv with administrator privileges"#;

/// How a password popup ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptOutcome {
    Installed,
    /// The user dismissed the popup; nothing changed.
    Cancelled,
    /// This desktop cannot show a popup; nothing changed.
    Unavailable,
}

/// How far a terminal pane's install has come.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminalStatus {
    Running,
    Installed,
    /// The script reported this exit code; the pane stays up showing why.
    Failed(String),
    /// The pane closed, or never started, before reporting a result.
    Cancelled,
}

/// A verified package and how to hand it to the system's installer.
#[derive(Debug, Clone)]
pub struct PrivilegedInstall {
    pub(crate) package: PathBuf,
    pub(crate) sha256: String,
    pub(crate) kind: &'static str,
    pub(crate) work: PathBuf,
    pub(crate) identity: PathBuf,
    /// The checksum of the identity the download was made for, kept in memory like the package's.
    pub(crate) identity_sha256: String,
    pub(crate) executable: PathBuf,
    pub(crate) prompter: Option<PathBuf>,
}

impl PrivilegedInstall {
    /// The installed program, to reopen once the app has quit.
    #[must_use]
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// Refuses a package downloaded for an installation something else has replaced since, which the package manager
    /// would otherwise downgrade.
    ///
    /// # Errors
    /// Returns an explanation when the installed version changed, or the filesystem's error.
    pub fn check_unchanged(&self) -> Result<(), Error> {
        if fs::read(&self.identity)? != fs::read(self.work.join("expected-identity"))? {
            return Err(error(
                "The installed version changed after the download started. Check for updates again.",
            ));
        }
        Ok(())
    }

    /// Shows the desktop's password popup and installs, blocking until both are done.
    ///
    /// # Errors
    /// Returns the installer's own explanation when it failed.
    pub fn prompt(&self) -> Result<PromptOutcome, Error> {
        match self.prompt_command().output() {
            Ok(output) => classify(output.status.code(), &String::from_utf8_lossy(&output.stderr)),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(PromptOutcome::Unavailable),
            Err(error) => Err(error.into()),
        }
    }

    fn prompt_command(&self) -> Command {
        let mac = self.kind == MAC_PACKAGE;
        let default = if mac { "/usr/bin/osascript" } else { "pkexec" };
        let mut command = Command::new(self.prompter.as_deref().unwrap_or_else(|| Path::new(default)));
        if mac {
            command.args(["-e", "on run argv", "-e", MAC_RUN, "-e", "end run", ROOT_SCRIPT]);
        } else {
            // pkexec's text agent would otherwise fail, or prompt on a terminal nobody is watching
            command.args([
                "--disable-internal-agent",
                "/bin/sh",
                "-c",
                ROOT_SCRIPT,
                "ngterm-update",
            ]);
        }
        command
            .arg(&self.package)
            .arg(&self.sha256)
            .arg(self.kind)
            .arg(&self.identity)
            .arg(&self.identity_sha256)
            .stdin(Stdio::null());
        command
    }

    /// Readies an install through `sudo` for a terminal pane to run, for desktops with no password popup.
    ///
    /// # Errors
    /// Returns the filesystem's error.
    pub fn terminal(&self) -> Result<TerminalInstall, Error> {
        for stale in [PANE_PID, PANE_RESULT] {
            let _ = fs::remove_file(self.work.join(stale));
        }
        let pane = self.work.join(PANE_SCRIPT_FILE);
        fs::write(&pane, PANE_SCRIPT)?;
        fs::write(self.work.join(ROOT_SCRIPT_FILE), ROOT_SCRIPT)?;
        let command = format!(
            "/bin/sh {} {} {} {} {} {}",
            shell_quote(&pane.to_string_lossy()),
            shell_quote(&self.package.to_string_lossy()),
            shell_quote(&self.sha256),
            shell_quote(self.kind),
            shell_quote(&self.identity.to_string_lossy()),
            shell_quote(&self.identity_sha256),
        );
        Ok(TerminalInstall {
            work: self.work.clone(),
            command,
            started: Instant::now(),
        })
    }
}

/// An install running in a terminal pane, reporting through files in the update's stage.
#[derive(Debug, Clone)]
pub struct TerminalInstall {
    work: PathBuf,
    command: String,
    started: Instant,
}

impl TerminalInstall {
    /// The POSIX command line the pane runs.
    #[must_use]
    pub fn command(&self) -> &str {
        &self.command
    }

    /// Where the pane has got to.
    #[must_use]
    pub fn status(&self) -> TerminalStatus {
        self.status_after(self.started.elapsed())
    }

    fn status_after(&self, elapsed: Duration) -> TerminalStatus {
        if let Some(result) = self.result() {
            return result;
        }
        let pid = fs::read_to_string(self.work.join(PANE_PID))
            .ok()
            .and_then(|text| text.trim().parse().ok());
        match pid {
            Some(pid) if crate::staging::process_alive(pid) => TerminalStatus::Running,
            // the script may have reported between the two reads, just before exiting
            Some(_) => self.result().unwrap_or(TerminalStatus::Cancelled),
            None if elapsed < PANE_START_LIMIT => TerminalStatus::Running,
            None => TerminalStatus::Cancelled,
        }
    }

    fn result(&self) -> Option<TerminalStatus> {
        let code = fs::read_to_string(self.work.join(PANE_RESULT)).ok()?;
        Some(match code.trim() {
            "0" => TerminalStatus::Installed,
            code => TerminalStatus::Failed(code.to_owned()),
        })
    }
}

fn classify(code: Option<i32>, stderr: &str) -> Result<PromptOutcome, Error> {
    if code == Some(0) {
        Ok(PromptOutcome::Installed)
    } else if NO_PROMPT_ERRORS.iter().any(|known| stderr.contains(known)) {
        Ok(PromptOutcome::Unavailable)
    } else if code == Some(126) || stderr.contains("(-128)") {
        // pkexec's exit for a dismissed dialog, and macOS's "User canceled" error
        Ok(PromptOutcome::Cancelled)
    } else {
        let lines: Vec<&str> = stderr.lines().map(str::trim).filter(|line| !line.is_empty()).collect();
        let tail = lines[lines.len().saturating_sub(3)..].join("\n");
        Err(error(format!("The update could not be installed.\n{tail}")))
    }
}

/// The lowercase hex SHA-256 of `bytes`.
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().fold(String::new(), |mut hex, byte| {
        let _ = write!(hex, "{byte:02x}");
        hex
    })
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

#[cfg(test)]
#[path = "privileged_tests.rs"]
mod tests;
