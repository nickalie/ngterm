//! The local command lines of a Windows viewer's remote panes and overlays (an extension beyond upstream, whose viewers
//! are Macs).
//!
//! A Unix viewer runs a remote pane's attach as a `/bin/sh -c` script ([`super::ssh::attach_pane_command`]) and a remote
//! overlay's job as its quoted ssh argv. A Windows pane runs its command line through `cmd.exe` (`ngterm_host`'s
//! `ShellCommand`), which has no `/bin/sh` and reads neither `'…'` nor `$?`, so there each of them is
//! `"<bin>\ngtermctl.exe" viewer-run WORD`: `WORD` the base64url of the [`ViewerRun`] saying what to run, which the
//! `ngtermctl` bundled beside this app carries out ([`local_cli`]). The record's shape is this build's own, so the
//! helper is named by its path: another `ngtermctl` earlier on `PATH` or in the pane's directory (an older install's)
//! would refuse it. `WORD`'s alphabet holds nothing `cmd.exe` reads, so no host, session name or remote command reaches
//! a shell here.

use std::path::Path;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};

use super::tree::REMOTE_CLI;

/// The `ngtermctl` subcommand that carries out a [`ViewerRun`]. Not a user's command: not in the help.
pub const VIEWER_RUN: &str = "viewer-run";

/// The keep-alive a pane's attach takes when the user's ssh config sets none: a dead link is noticed in about ten
/// seconds, not at the next keystroke.
pub const KEEP_ALIVE: [&str; 4] = ["-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2"];

/// The line `ssh -G` prints for a config that sets no keep-alive.
pub const NO_KEEP_ALIVE: &str = "serveraliveinterval 0";

/// ssh's exit status when it lost the connection or never made one.
pub const SSH_LOST: i32 = 255;

/// What a lost link's pane prints before its bar: the attributes and the mouse, focus, paste and resize reporting a
/// remote program may have left on, switched off, so nothing a wheel or a paste sends prints under the bar.
pub const LOST_RESET: &str =
    "\x1b[0m\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[?2004l\x1b[?2031l\x1b[?2048l";

/// What a remote pane's command does when the connection ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LostLink {
    /// The reconnecting bar's text.
    pub bar: String,
    /// The title that tells the app the link was lost ([`super::reconnect::RemoteLinkNotice`]).
    pub title: String,
}

impl LostLink {
    /// What the pane prints once ssh lost the link, the cursor hidden last, before it waits for the app to attach it
    /// again.
    #[must_use]
    pub fn output(&self) -> String {
        format!(
            "{LOST_RESET}\r\n\x1b[30;43m {} \x1b[K\x1b[0m\n\x1b]2;{}\x07\x1b[?25l",
            self.bar, self.title
        )
    }
}

/// What `ngtermctl viewer-run` does on a Windows viewer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ViewerRun {
    /// `argv` on this stdio, ending with its status: a remote overlay's job.
    Program { argv: Vec<String> },
    /// A remote pane's attach, as [`super::ssh::attach_pane_command`]'s script does it: ssh with [`KEEP_ALIVE`] added
    /// before its destination when `ssh -G` says the config sets none, then, when ssh lost the link and `lost` is
    /// set, [`LostLink::output`] and a wait on stdin; otherwise `report`, a space and ssh's status as a line. The pane
    /// ends with ssh's status.
    Pane {
        attach: Vec<String>,
        report: String,
        lost: Option<LostLink>,
    },
}

impl ViewerRun {
    /// The command line that runs this on a Windows viewer, through the `ngtermctl` beside this app ([`local_cli`]).
    #[must_use]
    pub fn line(&self) -> String {
        self.line_with(&local_cli())
    }

    /// [`Self::line`] through the helper at `cli`, quoted as [`local_cli`] quotes it.
    #[must_use]
    pub fn line_for(&self, cli: &Path) -> String {
        self.line_with(&cli_word(Some(cli)))
    }

    /// [`Self::line`] with `cli` as its program word.
    fn line_with(&self, cli: &str) -> String {
        // a plain record of strings: to_vec cannot fail
        let encoded = serde_json::to_vec(self).unwrap_or_default();
        format!("{cli} {VIEWER_RUN} {}", URL_SAFE_NO_PAD.encode(encoded))
    }

    /// The record a [`Self::line`]'s word holds.
    ///
    /// # Errors
    ///
    /// The word is not one this build's [`Self::line`] wrote.
    pub fn decode(word: &str) -> Result<Self, String> {
        let bytes = URL_SAFE_NO_PAD
            .decode(word)
            .map_err(|error| format!("{VIEWER_RUN}: {error}"))?;
        serde_json::from_slice(&bytes).map_err(|error| format!("{VIEWER_RUN}: {error}"))
    }
}

/// The program word of a [`ViewerRun::line`]: the `ngtermctl` beside this process's executable (the app's and
/// `ngterm-headless`'s `bin`, where the installer and `scripts/stage.sh` put it), as [`cli_word`] quotes it.
#[must_use]
pub fn local_cli() -> String {
    let beside = std::env::current_exe()
        .ok()
        .map(|executable| executable.with_file_name(format!("{REMOTE_CLI}{}", std::env::consts::EXE_SUFFIX)))
        .filter(|cli| cli.is_file());
    cli_word(beside.as_deref())
}

/// The pane variable naming the app's executable directory, the one [`local_cli`] looks in (`ngterm-term`'s launch
/// sets it from the same `current_exe`, for the app and `ngterm-headless` alike).
const BIN_DIR_VARIABLE: &str = "GHOSTTY_BIN_DIR";

/// `cli` as `cmd.exe` reads one program word: in double quotes, which keep its spaces and any `&`, `^`, `(`, `)` or
/// `!` literal (a `!` is read only under delayed expansion, which `cmd.exe` leaves off unless the registry turns it on).
/// A `%` it would still expand inside them, so a path holding one is named through [`BIN_DIR_VARIABLE`], whose value
/// `cmd.exe` does not read again. No path at all gives the bare name, found as `PATH` finds it.
fn cli_word(cli: Option<&Path>) -> String {
    let Some(path) = cli.and_then(Path::to_str).filter(|path| !path.contains('"')) else {
        return REMOTE_CLI.to_owned();
    };
    if !path.contains('%') {
        return format!("\"{path}\"");
    }
    match path.rsplit(['\\', '/']).next() {
        Some(name) if !name.is_empty() && !name.contains('%') => format!("\"%{BIN_DIR_VARIABLE}%\\{name}\""),
        _ => REMOTE_CLI.to_owned(),
    }
}

/// The `ssh -G` run that prints the config `attach`, an ssh argv, connects with.
#[must_use]
pub fn config_probe(attach: &[String]) -> Vec<String> {
    let mut probe = vec!["ssh".to_owned(), "-G".to_owned()];
    probe.extend(attach.iter().skip(1).cloned());
    probe
}

/// `attach` with [`KEEP_ALIVE`] before its destination and remote command when `config`, what [`config_probe`]
/// printed, sets no keep-alive; unchanged otherwise, an unreadable config included.
#[must_use]
pub fn with_keep_alive(attach: &[String], config: Option<&str>) -> Vec<String> {
    let mut argv = attach.to_vec();
    let unset = config.is_some_and(|config| config.lines().any(|line| line.trim_end() == NO_KEEP_ALIVE));
    if unset && argv.len() >= 2 {
        let at = argv.len() - 2;
        argv.splice(at..at, KEEP_ALIVE.map(str::to_owned));
    }
    argv
}

/// The local command line that runs `argv` on this viewer: its quoted words for `/bin/sh` on Unix, a [`ViewerRun`] on
/// Windows.
#[must_use]
pub fn program_line(argv: &[String]) -> String {
    if cfg!(windows) {
        ViewerRun::Program { argv: argv.to_vec() }.line()
    } else {
        crate::command_restore::shell_quoted_line(argv)
    }
}

/// `line`, a pane's or overlay's command line, with a Windows [`ViewerRun::line`] spelled out as the Unix form's
/// words: what it runs, quoted as `/bin/sh` would see it, then a pane's exit line, bar and title. Any other line is
/// returned as it is, so a test reads the same content whichever form this OS builds.
#[cfg(test)]
pub(crate) fn readable(line: &str) -> String {
    use crate::command_restore::shell_quoted_line;
    let Some((_, word)) = line.split_once(&format!(" {VIEWER_RUN} ")) else {
        return line.to_owned();
    };
    match ViewerRun::decode(word) {
        Ok(ViewerRun::Program { argv }) => shell_quoted_line(&argv),
        Ok(ViewerRun::Pane { attach, report, lost }) => {
            let lost = lost
                .map(|lost| format!(" {} {}", lost.bar, lost.title))
                .unwrap_or_default();
            format!("{} {report}{lost}", shell_quoted_line(&attach))
        }
        Err(_) => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn a_line_is_plain_words_and_decodes_to_its_record() {
        let run = ViewerRun::Pane {
            attach: words(&["ssh", "-tt", "box", "it's $(a) \"b\" %c% ^d & é"]),
            report: "ngterm: x'y (left) on box disconnected, exit".to_owned(),
            lost: Some(LostLink {
                bar: "Connection to box lost".to_owned(),
                title: "ngterm-remote;n1:lost".to_owned(),
            }),
        };
        let line = run.line_with(REMOTE_CLI);
        let parts: Vec<&str> = line.split(' ').collect();
        assert_eq!(parts[..2], [REMOTE_CLI, VIEWER_RUN]);
        assert_eq!(parts.len(), 3, "{line}");
        assert!(
            parts[2]
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'),
            "nothing cmd.exe reads: {line}"
        );
        assert_eq!(ViewerRun::decode(parts[2]).unwrap(), run);
        assert!(ViewerRun::decode("?!").is_err());
    }

    #[test]
    fn the_helper_is_named_by_its_path_in_quotes_cmd_keeps_whole() {
        let installed = Path::new(r"C:\Users\a b\AppData\Local\Programs\ngterm (x64)\bin\ngtermctl.exe");
        assert_eq!(
            cli_word(Some(installed)),
            r#""C:\Users\a b\AppData\Local\Programs\ngterm (x64)\bin\ngtermctl.exe""#
        );
        let run = ViewerRun::Program {
            argv: words(&["ssh", "box"]),
        };
        let line = run.line_with(&cli_word(Some(installed)));
        assert!(line.starts_with(r#""C:\Users\a b\"#), "{line}");
        assert!(line.contains(&format!(".exe\" {VIEWER_RUN} ")), "{line}");
        assert_eq!(
            cli_word(Some(Path::new(r"C:\wow!\ngtermctl.exe"))),
            r#""C:\wow!\ngtermctl.exe""#
        );
        // a `%` cmd.exe would expand even in quotes goes through the pane's variable, whose value it reads once
        assert_eq!(
            cli_word(Some(Path::new(r"C:\100%\bin\ngtermctl.exe"))),
            r#""%GHOSTTY_BIN_DIR%\ngtermctl.exe""#
        );
        assert_eq!(cli_word(None), REMOTE_CLI);
    }

    #[test]
    fn this_processs_helper_is_the_one_beside_it_when_there_is_one() {
        let beside = std::env::current_exe()
            .unwrap()
            .with_file_name(format!("{REMOTE_CLI}{}", std::env::consts::EXE_SUFFIX));
        let expected = if beside.is_file() {
            cli_word(Some(&beside))
        } else {
            REMOTE_CLI.to_owned()
        };
        assert_eq!(local_cli(), expected);
    }

    #[test]
    fn the_keep_alive_goes_before_the_destination_only_when_the_config_sets_none() {
        let attach = words(&["ssh", "-tt", "-o", "LogLevel=ERROR", "box", "tail"]);
        assert_eq!(
            config_probe(&attach),
            words(&["ssh", "-G", "-tt", "-o", "LogLevel=ERROR", "box", "tail"])
        );
        let unset = "user me\r\nserveraliveinterval 0\r\nport 22\r\n";
        assert_eq!(
            with_keep_alive(&attach, Some(unset)),
            words(&[
                "ssh",
                "-tt",
                "-o",
                "LogLevel=ERROR",
                "-o",
                "ServerAliveInterval=5",
                "-o",
                "ServerAliveCountMax=2",
                "box",
                "tail"
            ])
        );
        assert_eq!(with_keep_alive(&attach, Some("serveraliveinterval 30\n")), attach);
        assert_eq!(with_keep_alive(&attach, None), attach);
    }

    #[test]
    fn a_lost_link_prints_the_reset_the_bar_and_the_title() {
        let lost = LostLink {
            bar: "Connection to box lost".to_owned(),
            title: "ngterm-remote;n1:lost".to_owned(),
        };
        assert_eq!(
            lost.output(),
            "\u{1b}[0m\u{1b}[?1000l\u{1b}[?1002l\u{1b}[?1003l\u{1b}[?1006l\u{1b}[?1004l\u{1b}[?2004l\u{1b}[?2031l\
             \u{1b}[?2048l\r\n\u{1b}[30;43m Connection to box lost \u{1b}[K\u{1b}[0m\n\u{1b}]2;ngterm-remote;n1:lost\
             \u{7}\u{1b}[?25l"
        );
    }

    #[test]
    fn a_readable_line_spells_out_what_a_windows_line_runs() {
        let pane = ViewerRun::Pane {
            attach: words(&["ssh", "-tt", "box", "tail"]),
            report: "ngterm: b (left) on box disconnected, exit".to_owned(),
            lost: Some(LostLink {
                bar: "Connection to box lost".to_owned(),
                title: "ngterm-remote;n1:lost".to_owned(),
            }),
        };
        assert_eq!(
            readable(&pane.line_with(REMOTE_CLI)),
            "'ssh' '-tt' 'box' 'tail' ngterm: b (left) on box disconnected, exit Connection to box lost \
             ngterm-remote;n1:lost"
        );
        let program = ViewerRun::Program {
            argv: words(&["ssh", "-tt", "box"]),
        };
        assert_eq!(readable(&program.line_with(REMOTE_CLI)), "'ssh' '-tt' 'box'");
        assert_eq!(readable("'/usr/bin/env' 'x'"), "'/usr/bin/env' 'x'");
    }

    #[test]
    fn a_program_line_is_this_viewers_form() {
        let argv = words(&["ssh", "-tt", "box", "it's"]);
        let line = program_line(&argv);
        if cfg!(windows) {
            let word = line.rsplit(' ').next().unwrap();
            assert_eq!(ViewerRun::decode(word).unwrap(), ViewerRun::Program { argv });
        } else {
            assert_eq!(line, crate::command_restore::shell_quoted_line(&argv));
        }
    }
}
