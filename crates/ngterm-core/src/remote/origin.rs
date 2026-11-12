//! Which kind of shell an origin's sshd hands a remote command to, and the remote command's form on a Windows origin
//! (an extension beyond upstream, whose origins are Macs).
//!
//! A Unix account has some POSIX-family or fish shell and always `/bin/sh`, so its forms ([`super::ssh`]) go through
//! `/bin/sh -c`. Windows' OpenSSH server runs `cmd.exe` or, with its `DefaultShell` set, PowerShell, and neither has
//! `/bin/sh`, reads `'…'` the same way, or expands the same variables. So a Windows origin's every remote command is
//! the one form both read alike: plain words, `ngtermctl origin-run WORD`, `WORD` the base64url of the [`OriginRun`]
//! saying what to run, which `ngtermctl` (found on the account's `PATH`: the installer puts it there, a staged
//! `ngterm-headless` adds its own folder) does on the far side. The user's text never reaches a shell there.
//!
//! The form is chosen per host. A Unix host keeps its first try unchanged: the form is only learned when a `live tree`
//! in the POSIX form fails the way a Windows shell rejects it ([`rejected_by_windows_shell`]) and the
//! shell-neutral [`WINDOWS_PROBE`] then says Windows. An endpoint already names its machine's kind
//! ([`OriginShell::of_endpoint`]), a Windows path.

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, PoisonError};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ngterm_proto::ControlLiveEndpoint;
use serde::{Deserialize, Serialize};

use super::tree::{REMOTE_CLI, RemoteCommandResult, RemoteCommandRunner, ssh_arguments};

/// The `ngtermctl` subcommand that carries out an [`OriginRun`]. Not a user's command: not in the help.
pub const ORIGIN_RUN: &str = "origin-run";

/// The command that tells a Windows origin from any other, whichever shell runs it: `cmd.exe` expands the first half
/// and PowerShell the second, to the `OS` variable only Windows sets, and a Unix shell prints neither.
pub const WINDOWS_PROBE: &str = "echo %OS%$env:OS";
const WINDOWS_ANSWER: &str = "Windows_NT";

/// How long the [`WINDOWS_PROBE`] may take.
pub const WINDOWS_PROBE_DEADLINE: Duration = Duration::from_secs(10);

/// What a Windows origin's attach prints before it exits 1, the POSIX form's guard in `cmd.exe`'s words (`cmd.exe`
/// because it is in every Windows and starts at once).
pub(crate) const WINDOWS_GONE_GUARD: [&str; 5] = [
    "cmd.exe",
    "/d",
    "/s",
    "/c",
    "echo ngterm: remote session is gone& exit 1",
];

/// The shell family that runs a host's remote commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OriginShell {
    /// Any Unix account's shell, through `/bin/sh -c`.
    #[default]
    Posix,
    /// Windows' `cmd.exe` or PowerShell, through [`OriginRun`].
    Windows,
}

impl OriginShell {
    /// The kind of machine `endpoint` is on, which its paths show: a Windows path has a backslash or a drive letter and
    /// does not start at a `/`.
    #[must_use]
    pub fn of_endpoint(endpoint: &ControlLiveEndpoint) -> Self {
        let bytes = endpoint.executable.as_bytes();
        let drive = bytes.len() > 1 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
        // a Unix path may hold a backslash, but it starts at the root
        if !endpoint.executable.starts_with('/') && (drive || endpoint.executable.contains('\\')) {
            Self::Windows
        } else {
            Self::Posix
        }
    }
}

/// How a host's remote commands are formed: the shell that runs them and the app that answers them. An agterm origin
/// is a Mac, so its commands always take the POSIX form.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct OriginForm {
    pub shell: OriginShell,
    pub peer: super::peer::RemotePeer,
}

impl OriginForm {
    /// `host`'s form, as far as this process has learned it ([`shell_of`], [`super::peer::peer_of`]).
    #[must_use]
    pub fn of(host: &str) -> Self {
        Self {
            shell: shell_of(host),
            peer: super::peer::peer_of(host),
        }
    }

    /// Whether the commands go to an agterm origin.
    #[must_use]
    pub fn is_agterm(self) -> bool {
        self.peer == super::peer::RemotePeer::Agterm
    }
}

impl From<OriginShell> for OriginForm {
    fn from(shell: OriginShell) -> Self {
        Self {
            shell,
            peer: super::peer::RemotePeer::Ngterm,
        }
    }
}

/// What was learned about each host in this process. A host not here is [`OriginShell::Posix`].
static LEARNED: LazyLock<Mutex<HashMap<String, OriginShell>>> = LazyLock::new(Mutex::default);

/// The shell `host`'s remote commands go to, as far as this process has learned.
#[must_use]
pub fn shell_of(host: &str) -> OriginShell {
    LEARNED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(host)
        .copied()
        .unwrap_or_default()
}

/// Records what `host` turned out to be.
pub fn learn(host: &str, shell: OriginShell) {
    LEARNED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(host.to_owned(), shell);
}

/// Whether a command sent in the POSIX form may have been a Windows shell's refusal of it: an ordinary failure with
/// nothing printed (a Unix host's usual failures are 64, 127, 255 or a JSON answer on stdout). The refusal's text is
/// no help: it is in the system's language, `cmd.exe` cuts the command at its first `/`, so it cannot be told from a
/// Unix host's failure here, and [`WINDOWS_PROBE`] decides. One that merely looks so costs that probe and no more.
#[must_use]
pub fn rejected_by_windows_shell(result: &RemoteCommandResult) -> bool {
    !matches!(result.status, 0 | -1 | 64 | 127 | 255) && result.stdout.trim().is_empty()
}

/// Whether [`WINDOWS_PROBE`]'s result says Windows.
#[must_use]
pub fn is_windows_answer(result: &RemoteCommandResult) -> bool {
    result.status == 0 && result.stdout.contains(WINDOWS_ANSWER)
}

/// [`WINDOWS_PROBE`] for `host`, as an ssh argv.
#[must_use]
pub fn windows_probe_command(host: &str) -> Vec<String> {
    let mut argv = ssh_arguments(host, 5, false);
    argv.push(WINDOWS_PROBE.to_owned());
    argv
}

/// Runs `argv_for(shell)`, this host's `live tree`, learning a Windows origin from a refusal of the POSIX form. Blocks.
/// Its answer is always the listing's own, so a Unix host's failure reads as it always has.
pub fn run_learning(
    runner: &dyn RemoteCommandRunner,
    host: &str,
    deadline: Duration,
    argv_for: impl Fn(OriginShell) -> Vec<String>,
) -> RemoteCommandResult {
    let shell = shell_of(host);
    let first = runner.run(&argv_for(shell), deadline);
    if shell == OriginShell::Windows || !rejected_by_windows_shell(&first) {
        return first;
    }
    if !is_windows_answer(&runner.run(&windows_probe_command(host), WINDOWS_PROBE_DEADLINE)) {
        return first;
    }
    learn(host, OriginShell::Windows);
    runner.run(&argv_for(OriginShell::Windows), deadline)
}

/// What `ngtermctl origin-run` does on a Windows origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum OriginRun {
    /// `ngtermctl` itself, with these words.
    Cli { words: Vec<String> },
    /// Another program, on the same stdio. An empty value removes the variable.
    Program {
        program: String,
        environment: Vec<(String, String)>,
        words: Vec<String>,
    },
    /// Prints `ready` as a line, then waits for stdin to end: the page forward's held session.
    Hold { ready: String },
    /// Stores stdin in a private file named `clipboard.EXTENSION` and prints the file's path.
    Paste { extension: String },
}

impl OriginRun {
    /// The remote command that runs this on a Windows origin.
    #[must_use]
    pub fn tail(&self) -> String {
        // a plain record of strings: to_vec cannot fail
        let encoded = serde_json::to_vec(self).unwrap_or_default();
        format!("{REMOTE_CLI} {ORIGIN_RUN} {}", URL_SAFE_NO_PAD.encode(encoded))
    }

    /// The record a [`Self::tail`]'s word holds.
    ///
    /// # Errors
    ///
    /// The word is not one this build's [`Self::tail`] wrote.
    pub fn decode(word: &str) -> Result<Self, String> {
        let bytes = URL_SAFE_NO_PAD
            .decode(word)
            .map_err(|error| format!("{ORIGIN_RUN}: {error}"))?;
        serde_json::from_slice(&bytes).map_err(|error| format!("{ORIGIN_RUN}: {error}"))
    }
}

/// A Windows origin's `ngtermctl WORDS`.
#[must_use]
pub fn windows_cli_tail<S: AsRef<str>>(words: &[S]) -> String {
    OriginRun::Cli {
        words: words.iter().map(|word| word.as_ref().to_owned()).collect(),
    }
    .tail()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn result(status: i32, stdout: &str, stderr: &str) -> RemoteCommandResult {
        RemoteCommandResult {
            status,
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        }
    }

    #[test]
    fn a_windows_endpoint_is_known_by_its_paths() {
        let endpoint = |executable: &str| ControlLiveEndpoint {
            peer: None,
            attach_bridge: None,
            executable: executable.to_owned(),
            socket_directory: "x".to_owned(),
        };
        for windows in [
            r"C:\Users\me\ngterm-session.exe",
            "c:/ngterm-session.exe",
            r"\\host\share\s.exe",
        ] {
            assert_eq!(
                OriginShell::of_endpoint(&endpoint(windows)),
                OriginShell::Windows,
                "{windows}"
            );
        }
        for posix in [
            "/opt/ngterm/bin/ngterm-session",
            "/usr/bin/a:b",
            r"/opt/a\b/ngterm-session",
            "ngterm-session",
        ] {
            assert_eq!(
                OriginShell::of_endpoint(&endpoint(posix)),
                OriginShell::Posix,
                "{posix}"
            );
        }
    }

    #[test]
    fn only_a_silent_ordinary_failure_suggests_a_windows_shell() {
        // what cmd.exe and PowerShell answer the POSIX form
        for refused in [
            result(
                1,
                "",
                r"''/bin/sh'' is not recognized as an internal or external command",
            ),
            result(
                1,
                "  \n",
                "At line:1 char:12\n+ '/bin/sh' '-c' 'PATH=...\nUnexpected token",
            ),
        ] {
            assert!(rejected_by_windows_shell(&refused), "{refused:?}");
        }
        for unix in [
            result(0, "{}", ""),
            result(255, "", "ssh: connect to host"),
            result(127, "", "/bin/sh: ngtermctl: not found"),
            result(64, "", "usage"),
            result(-1, "", "no answer"),
            result(1, r#"{"error":"x"}"#, "the server said no"),
        ] {
            assert!(!rejected_by_windows_shell(&unix), "{unix:?}");
        }
    }

    #[test]
    fn the_probe_answer_is_the_windows_variable_and_a_clean_exit() {
        assert!(is_windows_answer(&result(0, "Windows_NT$env:OS\r\n", "")));
        assert!(is_windows_answer(&result(0, "%OS%Windows_NT\r\n", "")));
        assert!(!is_windows_answer(&result(0, "%OS%:OS\n", "")));
        assert!(!is_windows_answer(&result(1, "Windows_NT", "")));
    }

    #[test]
    fn every_run_survives_its_word_and_the_word_is_plain() {
        for run in [
            OriginRun::Cli {
                words: vec!["session".into(), "new".into(), "--name=it's $(a) \"b\" %c% é".into()],
            },
            OriginRun::Program {
                program: r"C:\Program Files\ngterm\ngterm-session.exe".into(),
                environment: vec![("A".into(), String::new()), ("B".into(), "c d".into())],
                words: vec!["attach".into(), "x".into()],
            },
            OriginRun::Hold { ready: "ready".into() },
            OriginRun::Paste {
                extension: "png".into(),
            },
        ] {
            let tail = run.tail();
            let word = tail.strip_prefix(&format!("{REMOTE_CLI} {ORIGIN_RUN} ")).unwrap();
            assert!(
                word.chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'),
                "{word}"
            );
            assert_eq!(OriginRun::decode(word).unwrap(), run);
        }
        assert!(OriginRun::decode("not base64 !").is_err());
        assert!(
            OriginRun::decode("e30").is_err(),
            "valid base64 of a record no build writes"
        );
    }

    /// Answers each ssh line by its remote command, and records them.
    struct Scripted {
        answers: Vec<(&'static str, RemoteCommandResult)>,
        asked: Mutex<Vec<String>>,
    }

    impl RemoteCommandRunner for Scripted {
        fn run(&self, argv: &[String], _deadline: Duration) -> RemoteCommandResult {
            let remote = argv.last().unwrap().clone();
            self.asked.lock().unwrap().push(remote.clone());
            self.answers
                .iter()
                .find(|(tail, _)| remote == *tail)
                .map_or_else(|| result(-1, "", "unscripted"), |(_, answer)| answer.clone())
        }
    }

    fn argv_for(shell: OriginShell) -> Vec<String> {
        vec![
            if shell == OriginShell::Posix {
                "posix"
            } else {
                "windows"
            }
            .to_owned(),
        ]
    }

    #[test]
    fn a_refused_listing_learns_the_windows_origin_and_is_asked_again() {
        let host = "learns-windows.example";
        let runner = Arc::new(Scripted {
            answers: vec![
                ("posix", result(1, "", "''' is not recognized")),
                (WINDOWS_PROBE, result(0, "Windows_NT$env:OS", "")),
                ("windows", result(0, "{\"listing\":1}", "")),
            ],
            asked: Mutex::default(),
        });
        let answer = run_learning(&*runner, host, Duration::from_secs(1), argv_for);
        assert_eq!(answer.stdout, "{\"listing\":1}");
        assert_eq!(shell_of(host), OriginShell::Windows);
        assert_eq!(*runner.asked.lock().unwrap(), ["posix", WINDOWS_PROBE, "windows"]);
        // and the next listing goes straight to the Windows form
        runner.asked.lock().unwrap().clear();
        run_learning(&*runner, host, Duration::from_secs(1), argv_for);
        assert_eq!(*runner.asked.lock().unwrap(), ["windows"]);
    }

    #[test]
    fn a_unix_host_keeps_its_own_failure_and_learns_nothing() {
        let host = "stays-posix.example";
        let refusal = result(1, "", "/bin/sh: something else");
        let runner = Scripted {
            answers: vec![("posix", refusal.clone()), (WINDOWS_PROBE, result(0, "%OS%:OS\n", ""))],
            asked: Mutex::default(),
        };
        assert_eq!(run_learning(&runner, host, Duration::from_secs(1), argv_for), refusal);
        assert_eq!(shell_of(host), OriginShell::Posix);
        assert_eq!(*runner.asked.lock().unwrap(), ["posix", WINDOWS_PROBE]);
    }

    #[test]
    fn a_failure_that_is_no_refusal_sends_no_probe() {
        let runner = Scripted {
            answers: vec![("posix", result(255, "", "ssh: connect: refused"))],
            asked: Mutex::default(),
        };
        run_learning(&runner, "no-probe.example", Duration::from_secs(1), argv_for);
        assert_eq!(*runner.asked.lock().unwrap(), ["posix"]);
    }
}
