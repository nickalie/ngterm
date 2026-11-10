//! The ssh command lines a viewer runs against another machine's ngterm (the attach half of upstream's
//! `RemoteSession.swift`; `live tree`'s own argv lives in [`super::tree`]). Pure and host-free: callers run what these
//! return.
//!
//! The remote commands of the tree, present, attach and control lines, the strings ssh hands the far account's shell,
//! are built on their own by `*_tail` functions ([`tree_tail`], [`present_tail`], [`attach_tail`], [`control_tail`]),
//! so a client with its own ssh transport runs exactly what desktop runs. Desktop's argv for those lines is
//! [`ssh_arguments`] plus the tail, except that attach also puts `-o LogLevel=ERROR` before the host (see
//! [`attach_command`]); a client attaching over its own transport should keep ssh's banners off the pane likewise. The
//! job, page-serve, tunnel and probe lines build their remote commands inline and have no tail builder. Each line has
//! the form of the shell its host's commands go to ([`super::origin`]).

use ngterm_proto::ControlLiveEndpoint;

use super::origin::{self, OriginRun, OriginShell, windows_cli_tail};
use super::reconnect::RemoteLinkNotice;
use super::tree::{InvocationError, REMOTE_CLI, is_plain, remote_cli_chain, ssh_arguments, validate};
use super::viewer_run::{KEEP_ALIVE, LOST_RESET, LostLink, NO_KEEP_ALIVE, SSH_LOST, ViewerRun, config_probe};
use crate::command_restore::shell_quoted_line;
use crate::live::lead::LeadAttachment;
use crate::live::{
    DIRECTORY_VARIABLE, NO_DETACH_KEY_VARIABLE, SESSION_PREFIX_VARIABLE, SESSION_VARIABLE, is_daemon_name,
};

/// A filesystem path, which may hold spaces: only control characters and NUL are refused, neither of which survives a
/// path anyway (upstream's `isPath`).
fn is_path(value: &str) -> bool {
    !value.is_empty() && !value.chars().any(|ch| u32::from(ch) < 0x20 || u32::from(ch) == 0x7f)
}

/// One ssh invocation carrying `session`'s presentation stream for as long as the row is shown (upstream's
/// `presentCommand`): `-T`, as frames travel on plain stdio a pty would mangle, no lifetime deadline, and `exec` so the
/// far shell does not linger between ssh and the bridge.
///
/// # Errors
///
/// A host ssh may not take, or a session id that is not one plain token.
pub fn present_command(host: &str, session: &str) -> Result<Vec<String>, InvocationError> {
    validate(host)?;
    let tail = present_tail(origin::shell_of(host), session)?;
    let mut argv = ssh_arguments(host, 5, false);
    argv.push(tail);
    Ok(argv)
}

/// The remote command of [`present_command`]: `ngtermctl live present SESSION` through [`remote_cli_chain`], run with
/// `exec`, or `shell`'s own form.
///
/// # Errors
///
/// A session id that is not one plain token.
pub fn present_tail(shell: OriginShell, session: &str) -> Result<String, InvocationError> {
    if !is_plain(session) {
        return Err(InvocationError::InvalidSession);
    }
    if shell == OriginShell::Windows {
        return Ok(windows_cli_tail(&["live", "present", session]));
    }
    let invocation = format!("exec {REMOTE_CLI} live present {}", shell_quoted_line(&[session]));
    Ok(remote_cli_chain(&invocation))
}

/// The remote command of [`super::tree::tree_argv`]: the far side's bare `ngtermctl live tree --json` through
/// [`remote_cli_chain`], or `shell`'s own form.
#[must_use]
pub fn tree_tail(shell: OriginShell) -> String {
    match shell {
        OriginShell::Posix => remote_cli_chain(&format!("{REMOTE_CLI} live tree --json")),
        OriginShell::Windows => windows_cli_tail(&["live", "tree", "--json"]),
    }
}

/// One ssh invocation running the helper of `job`, an overlay the origin handed to this machine, for as long as the
/// overlay is up (upstream's `runJobCommand`): `-tt`, as the program is the user's and reads the pty, and `exec` so the
/// helper's hangup check sees the ssh session's own pty close.
///
/// # Errors
///
/// A host ssh may not take, or a job id that is not one plain token.
pub fn run_job_command(host: &str, job: &str) -> Result<Vec<String>, InvocationError> {
    validate(host)?;
    if !is_plain(job) {
        return Err(InvocationError::InvalidSession);
    }
    let tail = match origin::shell_of(host) {
        OriginShell::Posix => remote_cli_chain(&format!(
            "exec {REMOTE_CLI} session overlay run-job {}",
            shell_quoted_line(&[job])
        )),
        OriginShell::Windows => windows_cli_tail(&["session", "overlay", "run-job", job]),
    };
    let mut argv = ssh_arguments(host, 5, true);
    argv.push(tail);
    Ok(argv)
}

/// One ssh invocation carrying the channel of `token`'s page, a page the origin handed to this machine, for as long as
/// the page is shown (upstream's `pageServeCommand`): `-T`, as frames travel on plain stdio a pty would mangle, the
/// same PATH chain as the presentation stream, and `exec` so the far shell does not linger between ssh and the helper.
///
/// # Errors
///
/// A host ssh may not take, or a token that is not one plain token.
pub fn page_serve_command(host: &str, token: &str) -> Result<Vec<String>, InvocationError> {
    validate(host)?;
    if !is_plain(token) {
        return Err(InvocationError::InvalidSession);
    }
    let tail = match origin::shell_of(host) {
        OriginShell::Posix => remote_cli_chain(&format!(
            "exec {REMOTE_CLI} session overlay page-serve {}",
            shell_quoted_line(&[token])
        )),
        OriginShell::Windows => windows_cli_tail(&["session", "overlay", "page-serve", token]),
    };
    let mut argv = ssh_arguments(host, 5, false);
    argv.push(tail);
    Ok(argv)
}

/// One ssh invocation forwarding `plan`'s port of this machine's loopback to the page server on the origin's, for as
/// long as the page is shown ([`super::page_tunnel`]): the forward listens on 127.0.0.1 only, a forward that cannot
/// bind ends ssh at once (`ExitOnForwardFailure`), keepalives notice a dead link, and the remote command prints the
/// ready line, then holds the session open until this machine closes ssh's stdin. Connection sharing is off: a forward
/// joined to a shared master (`ControlMaster`, `ControlPersist`) is the master's and outlives this process, so the
/// page's close would leave it listening. Quoted for any login shell, as the origin control lines are.
///
/// # Errors
///
/// A host ssh may not take, or a target host that is not one plain token.
pub fn tunnel_command(host: &str, plan: &super::page_tunnel::TunnelPlan) -> Result<Vec<String>, InvocationError> {
    validate(host)?;
    if !is_plain(&plan.target_host) {
        return Err(InvocationError::InvalidSession);
    }
    let mut argv = ssh_arguments(host, 5, false);
    let destination = argv.pop().unwrap_or_default();
    let forward = format!(
        "127.0.0.1:{}:{}:{}",
        plan.local_port, plan.target_host, plan.target_port
    );
    argv.extend(
        [
            "-o",
            "ControlMaster=no",
            "-o",
            "ControlPath=none",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ServerAliveInterval=15",
            "-o",
            "ServerAliveCountMax=3",
            "-L",
            &forward,
        ]
        .map(str::to_owned),
    );
    argv.push(destination);
    argv.push(match origin::shell_of(host) {
        OriginShell::Posix => {
            let script = format!("printf '%s\\n' {}; exec cat >/dev/null", super::page_tunnel::READY_LINE);
            ["/bin/sh", "-c", script.as_str()].map(account_shell_quoted).join(" ")
        }
        OriginShell::Windows => OriginRun::Hold {
            ready: super::page_tunnel::READY_LINE.to_owned(),
        }
        .tail(),
    });
    Ok(argv)
}

/// What the attach's gone guard prints before it exits 1: a pane whose attach ends so names a session gone from its
/// machine.
pub const SESSION_GONE: &str = "ngterm: remote session is gone";

/// The create-only guard after the attach: an existing daemon ignores it, a vanished one runs it and fails, so the
/// attach never hands back a fresh shell wearing the old session's name. It prints [`SESSION_GONE`].
const GONE_GUARD: &str = "printf '%s\\n' 'ngterm: remote session is gone'; exit 1";

/// One ssh invocation attaching to `daemon` on `host` for the lifetime of the pane (upstream's `attachCommand`, which
/// runs the far machine's zmx; here its `ngterm-session attach`): `-tt`, as `ngterm-session attach` reads termios and
/// the window size, no lifetime deadline, and `LogLevel=ERROR` so ssh's own banners stay off the pane. The attach
/// contract's variables the account may carry are cleared, as an inherited `NGTERM_LIVE_SESSION` refuses the attach
/// (upstream's zmx switched session there) and a prefix would rename the daemon; `lead` opts the far attach client into
/// explicit leadership.
///
/// # Errors
///
/// A host ssh may not take, a name that is not an ngterm daemon's, or an endpoint path with control characters.
pub fn attach_command(
    host: &str,
    endpoint: &ControlLiveEndpoint,
    daemon: &str,
    lead: Option<&LeadAttachment>,
) -> Result<Vec<String>, InvocationError> {
    validate(host)?;
    let tail = attach_tail(endpoint, daemon, lead)?;
    let mut argv = ssh_arguments(host, 5, true);
    argv.push(tail);
    let at = argv.len() - 2;
    argv.splice(at..at, ["-o".to_owned(), "LogLevel=ERROR".to_owned()]);
    Ok(argv)
}

/// The remote command of [`attach_command`]: `ngterm-session attach DAEMON` under `/usr/bin/env` with the attach
/// contract's variables cleared, `lead`'s words ahead of the executable, and the create-only gone guard after it.
/// An endpoint whose paths are Windows' ([`OriginShell::of_endpoint`]) gets that machine's form instead, the same contract
/// through [`OriginRun::Program`].
///
/// # Errors
///
/// A name that is not an ngterm daemon's, or an endpoint path with control characters.
pub fn attach_tail(
    endpoint: &ControlLiveEndpoint,
    daemon: &str,
    lead: Option<&LeadAttachment>,
) -> Result<String, InvocationError> {
    if !is_daemon_name(daemon) {
        return Err(InvocationError::InvalidSession);
    }
    if !is_path(&endpoint.executable) || !is_path(&endpoint.socket_directory) {
        return Err(InvocationError::InvalidEndpoint);
    }
    if OriginShell::of_endpoint(endpoint) == OriginShell::Windows {
        return Ok(windows_attach_tail(endpoint, daemon, lead));
    }
    let mut words = vec![
        "/usr/bin/env".to_owned(),
        format!("{SESSION_VARIABLE}="),
        format!("{SESSION_PREFIX_VARIABLE}="),
        format!("{NO_DETACH_KEY_VARIABLE}=1"),
    ];
    words.push(format!("{DIRECTORY_VARIABLE}={}", endpoint.socket_directory));
    words.extend(lead.map(LeadAttachment::assignments).unwrap_or_default());
    words.extend(
        [
            endpoint.executable.as_str(),
            "attach",
            daemon,
            "/bin/sh",
            "-c",
            GONE_GUARD,
        ]
        .into_iter()
        .map(str::to_owned),
    );
    Ok(shell_quoted_line(&words))
}

/// [`attach_tail`] for a Windows origin: the executable under the same contract's environment, its create-only guard
/// in `cmd.exe`'s words ([`origin::WINDOWS_GONE_GUARD`]).
fn windows_attach_tail(endpoint: &ControlLiveEndpoint, daemon: &str, lead: Option<&LeadAttachment>) -> String {
    let mut environment = vec![
        (SESSION_VARIABLE.to_owned(), String::new()),
        (SESSION_PREFIX_VARIABLE.to_owned(), String::new()),
        (NO_DETACH_KEY_VARIABLE.to_owned(), "1".to_owned()),
        (DIRECTORY_VARIABLE.to_owned(), endpoint.socket_directory.clone()),
    ];
    environment.extend(lead.map(LeadAttachment::environment).unwrap_or_default());
    let mut words = vec!["attach".to_owned(), daemon.to_owned()];
    words.extend(origin::WINDOWS_GONE_GUARD.map(str::to_owned));
    OriginRun::Program {
        program: endpoint.executable.clone(),
        environment,
        words,
    }
    .tail()
}

/// What a remote pane's command needs to name it in its last line.
#[derive(Debug, Clone, Copy)]
pub struct PaneLabel<'a> {
    /// The origin's session name.
    pub session: &'a str,
    /// `left` or `right`.
    pub pane: &'a str,
}

/// The pane's command (upstream's `attachPaneCommand`): the attach, then one line naming the host, the session and
/// the exit status, the pane exiting with ssh's status. A keep-alive is added when the user's ssh config sets none.
/// With a lead nonce, ssh's own failure (255) shows a reconnecting bar, reports the loss as a [`RemoteLinkNotice`] title
/// and waits on `cat` for the app to attach the pane again; the mouse and focus reporting a remote program left on is
/// switched off first, and echo with the cursor, so nothing a paste or a wheel sends prints under the bar.
///
/// That is a `/bin/sh` script on Unix; a Windows viewer's pane runs the same steps as a [`ViewerRun::Pane`].
///
/// # Errors
///
/// As [`attach_command`].
pub fn attach_pane_command(
    host: &str,
    endpoint: &ControlLiveEndpoint,
    daemon: &str,
    label: PaneLabel<'_>,
    lead: Option<&LeadAttachment>,
) -> Result<String, InvocationError> {
    let argv = attach_command(host, endpoint, daemon, lead)?;
    let report = format!(
        "ngterm: {} ({}) on {host} disconnected, exit",
        label.session, label.pane
    );
    let lost = lead.map(|lead| LostLink {
        bar: format!("Connection to {host} lost · reconnecting… · any key retries now"),
        title: RemoteLinkNotice::title(&lead.nonce),
    });
    if cfg!(windows) {
        return Ok(ViewerRun::Pane {
            attach: argv,
            report,
            lost,
        }
        .line());
    }
    Ok(posix_pane_command(&argv, &report, lost.as_ref()))
}

/// [`attach_pane_command`]'s `/bin/sh` script. Its escapes are `printf`'s, so the command line holds no control
/// character.
fn posix_pane_command(argv: &[String], report: &str, lost: Option<&LostLink>) -> String {
    // $ka is unquoted on purpose: four words or none
    let keep_alive = format!(
        "ka=; {} 2>/dev/null | grep -qx {} && ka={}",
        shell_quoted_line(&config_probe(argv)),
        shell_quoted_line(&[NO_KEEP_ALIVE]),
        shell_quoted_line(&[KEEP_ALIVE.join(" ")])
    );
    let split = argv.len() - 2;
    let attach = format!(
        "{} $ka {}",
        shell_quoted_line(&argv[..split]),
        shell_quoted_line(&argv[split..])
    );
    // the pane exits with ssh's status, not printf's zero, or a failed connection reads as a clean one
    let report = format!("printf '%s %s\\n' {} \"$status\"", shell_quoted_line(&[report]));
    let script = match lost {
        None => format!("{keep_alive}; {attach}; status=$?; {report}; exit \"$status\""),
        Some(lost) => {
            let bar = shell_quoted_line(&[lost.bar.as_str()]);
            let title = shell_quoted_line(&[lost.title.as_str()]);
            let reset = LOST_RESET.replace('\x1b', "\\033");
            format!(
                "{keep_alive}; {attach}; status=$?; if [ \"$status\" -eq {SSH_LOST} ]; then \
                 printf '{reset}\\r\\n\\033[30;43m %s \\033[K\\033[0m\\n' {bar}; printf '\\033]2;%s\\007' {title}; \
                 printf '\\033[?25l'; stty -echo 2>/dev/null; cat >/dev/null; else {report}; fi; exit \"$status\""
            )
        }
    };
    // `env` takes the exec a terminal runs its command under, and a plain `sh` beneath it reads no login profile
    shell_quoted_line(&["/usr/bin/env", "/bin/sh", "-c", script.as_str()])
}

/// One ssh invocation that only proves the host answers again, before a waiting pane is attached anew (upstream's
/// `probeCommand`).
///
/// # Errors
///
/// A host ssh may not take.
pub fn probe_command(host: &str) -> Result<Vec<String>, InvocationError> {
    validate(host)?;
    let mut argv = ssh_arguments(host, 5, false);
    // `true` is no command in `cmd.exe` or PowerShell, whose `exit 0` is every shell's
    argv.push(
        if origin::shell_of(host) == OriginShell::Windows {
            "exit 0"
        } else {
            "true"
        }
        .to_owned(),
    );
    Ok(argv)
}

/// One ssh invocation running the far side's `ngtermctl` with `arguments` plus `--json`, so the reply is the server's
/// own response line (upstream's `RemoteSession.controlCommand`). The arguments carry user text, so the outer layer is
/// quoted for any login shell ([`account_shell_quoted`]), as the page forward's remote script is.
///
/// # Errors
///
/// A host ssh may not take.
pub fn control_command<S: AsRef<str>>(host: &str, arguments: &[S]) -> Result<Vec<String>, InvocationError> {
    validate(host)?;
    let mut argv = ssh_arguments(host, 5, false);
    argv.push(control_tail(origin::shell_of(host), arguments));
    Ok(argv)
}

/// The remote command of [`control_command`]: the widened `PATH`, then `ngtermctl ARGUMENTS --json`, under `/bin/sh
/// -c` quoted for any login shell, or `shell`'s own form.
#[must_use]
pub fn control_tail<S: AsRef<str>>(shell: OriginShell, arguments: &[S]) -> String {
    let mut words: Vec<&str> = arguments.iter().map(AsRef::as_ref).collect();
    words.push("--json");
    if shell == OriginShell::Windows {
        return windows_cli_tail(&words);
    }
    let chain = format!(
        "{} && {REMOTE_CLI} {}",
        super::tree::cli_path_prefix(),
        shell_quoted_line(&words)
    );
    ["/bin/sh", "-c", chain.as_str()].map(account_shell_quoted).join(" ")
}

/// `value` quoted as one word that sh, zsh, tcsh and fish all read alike (upstream's `accountShellQuoted`): an
/// apostrophe or a backslash goes outside the quotes, escaped, since fish treats both specially inside single quotes.
fn account_shell_quoted(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for ch in value.chars() {
        match ch {
            '\'' => quoted.push_str(r"'\''"),
            '\\' => quoted.push_str(r"'\\'"),
            other => quoted.push(other),
        }
    }
    quoted.push('\'');
    quoted
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    use uuid::Uuid;

    use super::*;
    use crate::live::daemon_name;

    #[test]
    fn the_gone_guard_prints_the_session_gone_line() {
        assert_eq!(
            GONE_GUARD,
            format!("printf '%s\\n' '{SESSION_GONE}'; exit 1"),
            "a viewer reads the guard's line to tell a gone session from a program's exit"
        );
    }

    fn endpoint() -> ControlLiveEndpoint {
        ControlLiveEndpoint {
            executable: "/opt/ngterm/bin/ngterm-session".to_owned(),
            socket_directory: "/tmp/ngterm-live-abc".to_owned(),
        }
    }

    fn lead(nonce: &str, claim: bool) -> LeadAttachment {
        LeadAttachment {
            nonce: nonce.to_owned(),
            claim,
        }
    }

    const LABEL: PaneLabel<'static> = PaneLabel {
        session: "build",
        pane: "left",
    };

    /// The record a Windows tail carries.
    fn windows_run(tail: &str) -> OriginRun {
        let word = tail.strip_prefix("ngtermctl origin-run ").expect(tail);
        OriginRun::decode(word).unwrap()
    }

    fn cli_words(tail: &str) -> Vec<String> {
        match windows_run(tail) {
            OriginRun::Cli { words } => words,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_windows_origins_lines_carry_their_words_past_either_shell() {
        let host = "learned-windows.example";
        origin::learn(host, OriginShell::Windows);
        let words = |tail: &str| {
            assert!(
                !tail.contains(['\'', '"', '$', '%', ' '].as_slice()) || tail.starts_with("ngtermctl origin-run "),
                "{tail}"
            );
            cli_words(tail)
        };
        assert_eq!(words(&tree_tail(OriginShell::Windows)), ["live", "tree", "--json"]);
        assert_eq!(
            words(present_command(host, "S'1").unwrap().last().unwrap()),
            ["live", "present", "S'1"]
        );
        assert_eq!(
            words(
                control_command(host, &["session", "new", "--name=it's %x% $y"])
                    .unwrap()
                    .last()
                    .unwrap()
            ),
            ["session", "new", "--name=it's %x% $y", "--json"]
        );
        let job = run_job_command(host, "job1").unwrap();
        assert_eq!(job[1], "-tt");
        assert_eq!(words(job.last().unwrap()), ["session", "overlay", "run-job", "job1"]);
        assert_eq!(
            words(page_serve_command(host, "tok").unwrap().last().unwrap()),
            ["session", "overlay", "page-serve", "tok"]
        );
        assert_eq!(probe_command(host).unwrap().last().unwrap(), "exit 0");
        let plan = super::super::page_tunnel::TunnelPlan::new("http://127.0.0.1:8000/", 9000).unwrap();
        assert_eq!(
            windows_run(tunnel_command(host, &plan).unwrap().last().unwrap()),
            OriginRun::Hold {
                ready: super::super::page_tunnel::READY_LINE.to_owned()
            }
        );
    }

    #[test]
    fn a_host_never_learned_keeps_every_posix_line() {
        let host = "never-learned.example";
        assert!(
            present_command(host, "S")
                .unwrap()
                .last()
                .unwrap()
                .starts_with("'/bin/sh' '-c' ")
        );
        assert_eq!(probe_command(host).unwrap().last().unwrap(), "true");
    }

    #[test]
    fn a_windows_endpoints_attach_is_the_session_program_with_its_contract() {
        let endpoint = ControlLiveEndpoint {
            executable: r"C:\Program Files\ngterm\ngterm-session.exe".to_owned(),
            socket_directory: r"C:\Users\me\AppData\Local\ngterm\live\ab".to_owned(),
        };
        let daemon = daemon_name(Uuid::from_u128(0x42));
        let tail = attach_tail(&endpoint, &daemon, Some(&lead("n0nce", true))).unwrap();
        let OriginRun::Program {
            program,
            environment,
            words,
        } = windows_run(&tail)
        else {
            panic!("{tail}");
        };
        assert_eq!(program, endpoint.executable);
        for (name, value) in [
            (SESSION_VARIABLE, ""),
            (SESSION_PREFIX_VARIABLE, ""),
            (NO_DETACH_KEY_VARIABLE, "1"),
            (DIRECTORY_VARIABLE, endpoint.socket_directory.as_str()),
        ] {
            assert!(
                environment.iter().any(|(key, set)| key == name && set == value),
                "{name} in {environment:?}"
            );
        }
        assert!(
            environment.iter().any(|(_, value)| value == "n0nce"),
            "the lead nonce: {environment:?}"
        );
        assert_eq!(words[..2], ["attach".to_owned(), daemon]);
        // the guard's last word prints what a viewer reads the gone session by
        assert!(words.last().unwrap().contains(SESSION_GONE), "{words:?}");
        assert_eq!(words[2], "cmd.exe");
    }

    /// The destination and options every non-interactive desktop line opens with.
    fn plain_prefix() -> Vec<String> {
        ssh_arguments("me@box", 5, false)
    }

    /// `prefix` followed by `tail`, the whole argv desktop runs.
    fn joined(mut prefix: Vec<String>, tail: &str) -> Vec<String> {
        prefix.push(tail.to_owned());
        prefix
    }

    const PATH_CHAIN: &str = r#"'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin:/usr/local/bin" && "#;

    /// Snapshots of the lines desktop ran before the tails were split out: each argv is `ssh_arguments` plus its tail,
    /// byte for byte.
    #[test]
    fn the_tree_and_present_lines_are_the_ssh_options_plus_their_tails() {
        let tree = r#"'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin:/usr/local/bin" && ngtermctl live tree --json'"#;
        assert_eq!(tree_tail(OriginShell::Posix), tree);
        assert_eq!(
            super::super::tree::tree_argv("me@box", OriginShell::Posix),
            joined(plain_prefix(), tree)
        );
        let present = r#"'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin:/usr/local/bin" && exec ngtermctl live present '\''S'\''\'\'''\''1'\'''"#;
        assert!(present.starts_with(PATH_CHAIN));
        assert_eq!(present_tail(OriginShell::Posix, "S'1").unwrap(), present);
        assert_eq!(
            present_command("me@box", "S'1").unwrap(),
            joined(plain_prefix(), present)
        );
        for bad in ["", "s 1", "s1\u{1b}[31m"] {
            assert_eq!(
                present_tail(OriginShell::Posix, bad),
                Err(InvocationError::InvalidSession),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_attach_line_is_the_ssh_options_plus_its_tail_with_and_without_a_lead() {
        let endpoint = ControlLiveEndpoint {
            executable: "/opt/My Apps/ngterm-session".to_owned(),
            socket_directory: "/tmp/ngterm-live-abc".to_owned(),
        };
        let daemon = daemon_name(Uuid::from_u128(0x42));
        let cleared = "'/usr/bin/env' 'NGTERM_LIVE_SESSION=' 'NGTERM_LIVE_SESSION_PREFIX=' \
                       'NGTERM_LIVE_NO_DETACH_KEY=1' 'NGTERM_LIVE_DIR=/tmp/ngterm-live-abc' ";
        let rest = r"'/opt/My Apps/ngterm-session' 'attach' 'ngterm-00000000000000000000000000000042' '/bin/sh' '-c' 'printf '\''%s\n'\'' '\''ngterm: remote session is gone'\''; exit 1'";
        let mut prefix = ssh_arguments("me@box", 5, true);
        prefix.splice(6..6, ["-o".to_owned(), "LogLevel=ERROR".to_owned()]);
        let cases = [
            (None, format!("{cleared}{rest}")),
            (
                Some(lead("n1", true)),
                format!("{cleared}'NGTERM_LIVE_MANAGED=n1' 'NGTERM_LIVE_MANAGED_CLAIM=1' {rest}"),
            ),
            (
                Some(lead("n2", false)),
                format!("{cleared}'NGTERM_LIVE_MANAGED=n2' {rest}"),
            ),
        ];
        for (lead, want) in cases {
            assert_eq!(attach_tail(&endpoint, &daemon, lead.as_ref()).unwrap(), want);
            assert_eq!(
                attach_command("me@box", &endpoint, &daemon, lead.as_ref()).unwrap(),
                joined(prefix.clone(), &want)
            );
        }
        assert_eq!(
            attach_tail(&endpoint, "notes", None),
            Err(InvocationError::InvalidSession)
        );
        let hostile = ControlLiveEndpoint {
            executable: "/opt/ngterm-session".to_owned(),
            socket_directory: "/tmp/x\n".to_owned(),
        };
        assert_eq!(
            attach_tail(&hostile, &daemon, None),
            Err(InvocationError::InvalidEndpoint)
        );
    }

    #[test]
    fn the_control_lines_are_the_ssh_options_plus_their_tails() {
        let cases: [(&[&str], &str); 3] = [
            (
                &["restore", "mode"],
                r#"'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin:/usr/local/bin" && ngtermctl '\''restore'\'' '\''mode'\'' '\''--json'\'''"#,
            ),
            (
                &["session", "new", "--no-select"],
                r#"'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin:/usr/local/bin" && ngtermctl '\''session'\'' '\''new'\'' '\''--no-select'\'' '\''--json'\'''"#,
            ),
            (
                &["session", "close", "--target", "it's"],
                r#"'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin:/usr/local/bin" && ngtermctl '\''session'\'' '\''close'\'' '\''--target'\'' '\''it'\'''\\''\'''\''s'\'' '\''--json'\'''"#,
            ),
        ];
        for (arguments, want) in cases {
            assert!(want.starts_with(PATH_CHAIN));
            assert_eq!(control_tail(OriginShell::Posix, arguments), want);
            assert_eq!(
                control_command("me@box", arguments).unwrap(),
                joined(plain_prefix(), want)
            );
        }
    }

    /// The present tail hands the far CLI the session as one word, whatever it holds. Unix only: the far side's shell
    /// is `/bin/sh`.
    #[cfg(unix)]
    #[test]
    fn the_present_tail_reaches_the_installed_cli_with_the_session_intact() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let script = "#!/bin/sh\nfor word in \"$@\"; do printf '[%s]' \"$word\"; done\n";
        ngterm_host::fakes::write_executable(&bin.join("ngtermctl"), script).unwrap();
        let ran = Command::new("/bin/sh")
            .args(["-c", &present_tail(OriginShell::Posix, "S'1;rm").unwrap()])
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert_eq!(ran.status.code(), Some(0), "{ran:?}");
        assert_eq!(String::from_utf8_lossy(&ran.stdout), "[live][present][S'1;rm]");
    }

    #[test]
    fn present_is_non_interactive_and_runs_the_bridge_for_exactly_that_session() {
        let argv = present_command("buildbox", "S1").unwrap();
        assert_eq!(
            &argv[..7],
            ["ssh", "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "buildbox"]
        );
        let remote = argv.last().unwrap();
        assert!(remote.starts_with("'/bin/sh' '-c' "));
        assert!(remote.contains(r"exec ngtermctl live present '\''S1'\''"), "{remote}");
        for bad in ["", "s 1", "s1\u{1b}[31m"] {
            assert_eq!(present_command("buildbox", bad), Err(InvocationError::InvalidSession));
        }
        assert_eq!(
            present_command("-oProxyCommand=x", "S1"),
            Err(InvocationError::InvalidHost)
        );
    }

    #[test]
    fn run_job_forces_a_pty_and_reaches_the_installed_cli() {
        let argv = run_job_command("buildbox", "J1").unwrap();
        assert_eq!(argv[1], "-tt");
        assert!(argv.last().unwrap().contains("exec ngtermctl session overlay run-job "));
        assert!(argv.last().unwrap().contains("$HOME/.local/bin"));
        assert_eq!(run_job_command("buildbox", "j 1"), Err(InvocationError::InvalidSession));
    }

    #[test]
    #[cfg_attr(windows, ignore = "runs the far side's /bin/sh, which Windows has not")]
    fn a_tunnel_forwards_a_loopback_port_and_holds_until_its_stdin_closes() {
        let plan = crate::remote::page_tunnel::TunnelPlan::new("http://localhost:5173/", 50123).unwrap();
        let argv = tunnel_command("buildbox", &plan).unwrap();
        let at = argv.iter().position(|word| word == "-L").unwrap();
        assert_eq!(argv[at + 1], "127.0.0.1:50123:localhost:5173");
        for option in ["ExitOnForwardFailure=yes", "ControlMaster=no", "ControlPath=none"] {
            assert!(argv.windows(2).any(|pair| pair == ["-o", option]), "{option}");
        }
        assert_eq!(argv[argv.len() - 2], "buildbox", "the destination follows every option");
        let script = argv.last().unwrap();
        let ran = Command::new("/bin/sh")
            .args(["-c", script])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&ran.stdout), "ngterm-tunnel-ready\n");
        assert!(ran.status.success());
        assert_eq!(
            tunnel_command("-oProxyCommand=x", &plan),
            Err(InvocationError::InvalidHost)
        );
    }

    /// Upstream's `pageServeUsesPlainStdioAndReachesTheInstalledCli`, `pageServeRefusesATokenThatIsNotAPlainToken` and
    /// `pageServeRefusesAnInvalidHost`.
    #[test]
    fn page_serve_uses_plain_stdio_and_reaches_the_installed_cli() {
        let argv = page_serve_command("buildbox", "tok-1").unwrap();
        assert_eq!(
            &argv[..7],
            ["ssh", "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "buildbox"]
        );
        assert_eq!(argv.len(), 8);
        assert!(argv[7].contains("$HOME/.local/bin"), "{}", argv[7]);
        for bad in ["", "t 1", "t1\u{1b}[31m"] {
            assert_eq!(
                page_serve_command("buildbox", bad),
                Err(InvocationError::InvalidSession),
                "{bad:?}"
            );
        }
        assert_eq!(
            page_serve_command("-oProxyCommand=x", "t1"),
            Err(InvocationError::InvalidHost)
        );
    }

    /// Upstream's `pageServeRunsTheHelperForExactlyThatToken`: the far shell hands the helper the token as one word,
    /// whatever it holds. Unix only: the far side's shell is `/bin/sh`.
    #[cfg(unix)]
    #[test]
    fn page_serve_runs_the_helper_for_exactly_that_token() {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let script = "#!/bin/sh\nfor word in \"$@\"; do printf '[%s]' \"$word\"; done\n";
        ngterm_host::fakes::write_executable(&bin.join("ngtermctl"), script).unwrap();
        let line = page_serve_command("buildbox", "t1;rm").unwrap().pop().unwrap();
        let ran = Command::new("/bin/sh")
            .args(["-c", &line])
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap();
        assert_eq!(ran.status.code(), Some(0), "{ran:?}");
        assert_eq!(
            String::from_utf8_lossy(&ran.stdout),
            "[session][overlay][page-serve][t1;rm]"
        );
    }

    #[test]
    fn attach_forces_a_pty_and_puts_the_lead_words_ahead_of_the_executable() {
        let daemon = daemon_name(Uuid::new_v4());
        let plain = attach_command("buildbox", &endpoint(), &daemon, None).unwrap();
        assert_eq!(plain[1], "-tt");
        assert_eq!(&plain[plain.len() - 4..plain.len() - 2], ["-o", "LogLevel=ERROR"]);
        assert_eq!(plain[plain.len() - 2], "buildbox");
        assert!(!plain.last().unwrap().contains("NGTERM_LIVE_MANAGED"));
        assert!(
            plain.last().unwrap().starts_with(
                "'/usr/bin/env' 'NGTERM_LIVE_SESSION=' 'NGTERM_LIVE_SESSION_PREFIX=' 'NGTERM_LIVE_NO_DETACH_KEY=1' \
                 'NGTERM_LIVE_DIR=/tmp/ngterm-live-abc' "
            ),
            "{}",
            plain.last().unwrap()
        );
        let claimed = attach_command("buildbox", &endpoint(), &daemon, Some(&lead("n1", true))).unwrap();
        let remote = claimed.last().unwrap();
        let managed = remote.find("'NGTERM_LIVE_MANAGED=n1'").unwrap();
        assert!(managed < remote.find("'/opt/ngterm/bin/ngterm-session' 'attach'").unwrap());
        assert!(remote.contains("'NGTERM_LIVE_MANAGED_CLAIM=1'"));
    }

    #[test]
    fn anything_but_an_ngterm_daemon_name_or_a_plain_path_is_refused() {
        for name in ["ngterm 42", "ngterm\n42", "", "notes", "ngterm-nothex"] {
            assert_eq!(
                attach_command("buildbox", &endpoint(), name, None),
                Err(InvocationError::InvalidSession),
                "{name}"
            );
        }
        let daemon = daemon_name(Uuid::new_v4());
        let spaced = ControlLiveEndpoint {
            executable: "/home/me/My Apps/ngterm/bin/ngterm-session".to_owned(),
            socket_directory: "/tmp/x".to_owned(),
        };
        assert!(attach_command("buildbox", &spaced, &daemon, None).is_ok());
        let hostile = ControlLiveEndpoint {
            executable: "/opt/ngterm-session\n".to_owned(),
            socket_directory: "/tmp/x".to_owned(),
        };
        assert_eq!(
            attach_command("buildbox", &hostile, &daemon, None),
            Err(InvocationError::InvalidEndpoint)
        );
    }

    #[test]
    fn control_is_non_interactive_and_ends_in_json() {
        let argv = control_command("buildbox", &["session", "new"]).unwrap();
        assert_eq!(
            &argv[..7],
            ["ssh", "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "buildbox"]
        );
        assert_eq!(argv.len(), 8);
        assert!(argv[7].starts_with("'/bin/sh' '-c' '"), "{}", argv[7]);
        assert!(argv[7].contains("ngtermctl"), "{}", argv[7]);
        assert_eq!(
            control_command("-oProxyCommand=x", &["tree"]),
            Err(InvocationError::InvalidHost)
        );
        assert_eq!(account_shell_quoted(r"it's a\b"), r"'it'\''s a'\\'b'");
    }

    /// Upstream's `controlReachesTheCliWithEveryArgumentIntactUnderAnyLoginShell`: sshd hands the remote line to the
    /// account's login shell with `-c`, so every shell present here reads it, and the CLI gets each argument intact.
    #[cfg(unix)]
    #[test]
    fn control_reaches_the_cli_with_every_argument_intact_under_any_login_shell() {
        let shells: Vec<&str> = [
            "/bin/sh",
            "/bin/zsh",
            "/usr/bin/zsh",
            "/bin/tcsh",
            "/usr/bin/tcsh",
            "/usr/bin/fish",
            "/usr/local/bin/fish",
            "/opt/homebrew/bin/fish",
        ]
        .into_iter()
        .filter(|shell| std::path::Path::new(shell).is_file())
        .collect();
        assert!(!shells.is_empty());
        let arguments = [
            "session",
            "new",
            "--parent",
            "p1",
            "--command",
            r"tail -f 'my log' a\b \\ end\",
            "--name",
            "it's",
            "--cwd",
            "",
        ];
        let line = control_command("buildbox", &arguments).unwrap().pop().unwrap();
        let mut expected: Vec<&str> = arguments.to_vec();
        expected.push("--json");
        for shell in shells {
            let home = tempfile::tempdir().unwrap();
            let bin = home.path().join(".local/bin");
            std::fs::create_dir_all(&bin).unwrap();
            let calls = home.path().join("calls");
            let script = format!("#!/bin/sh\nprintf '%s\\0' \"$@\" > '{}'\n", calls.display());
            ngterm_host::fakes::write_executable(&bin.join("ngtermctl"), &script).unwrap();
            let ran = Command::new(shell)
                .args(["-c", &line])
                .env_clear()
                .env("HOME", home.path())
                .env("ZDOTDIR", home.path())
                .env("XDG_CONFIG_HOME", home.path())
                .env("PATH", "/usr/bin:/bin")
                .output()
                .unwrap();
            assert_eq!(ran.status.code(), Some(0), "{shell}: {ran:?}");
            let recorded = std::fs::read(&calls).unwrap();
            let words: Vec<String> = recorded
                .split(|byte| *byte == 0)
                .map(|word| String::from_utf8_lossy(word).into_owned())
                .collect();
            let mut want: Vec<String> = expected.iter().map(|word| (*word).to_owned()).collect();
            want.push(String::new());
            assert_eq!(words, want, "{shell}");
        }
    }

    #[test]
    fn the_probe_only_asks_the_host_to_answer() {
        assert_eq!(
            probe_command("mini").unwrap(),
            [
                "ssh",
                "-T",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=5",
                "mini",
                "true"
            ]
        );
        assert_eq!(probe_command(""), Err(InvocationError::EmptyHost));
    }

    /// A scratch directory with a fake `ssh` on its own `PATH`, which records its arguments and exits with `code`.
    struct FakeRemote {
        root: PathBuf,
    }

    impl FakeRemote {
        fn new(code: i32) -> Self {
            let root = std::env::temp_dir().join(format!("ngterm-fake-ssh-{}", Uuid::new_v4().simple()));
            std::fs::create_dir_all(root.join("bin")).unwrap();
            let script = format!(
                "#!/bin/sh\nif [ \"$1\" = -G ]; then echo 'serveraliveinterval 0'; exit 0; fi\n\
                 printf '%s\\n' \"$@\" > '{}'\nexit {code}\n",
                root.join("calls").display()
            );
            let ssh = root.join("bin/ssh");
            ngterm_host::fakes::write_executable(&ssh, &script).unwrap();
            Self { root }
        }

        fn path(&self) -> String {
            format!("{}:/usr/bin:/bin", self.root.join("bin").display())
        }

        /// Runs `command` as a terminal runs a pane's command line, stdin closed.
        fn run(&self, command: &str) -> (Option<i32>, String) {
            let output = Command::new("/bin/sh")
                .arg("-c")
                .arg(command)
                .env("PATH", self.path())
                .stdin(Stdio::null())
                .output()
                .unwrap();
            (
                output.status.code(),
                String::from_utf8_lossy(&output.stdout).into_owned(),
            )
        }

        fn calls(&self) -> Vec<String> {
            std::fs::read_to_string(self.root.join("calls"))
                .unwrap_or_default()
                .lines()
                .map(str::to_owned)
                .collect()
        }

        fn marker(&self) -> PathBuf {
            self.root.join("pwned")
        }
    }

    impl Drop for FakeRemote {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "runs its fake ssh and the pane command with the POSIX shell /bin/sh, which Windows has not"
    )]
    fn the_pane_command_keeps_the_ssh_exit_status_and_says_what_died() {
        let fake = FakeRemote::new(23);
        let daemon = daemon_name(Uuid::new_v4());
        let command = attach_pane_command(
            "buildbox",
            &endpoint(),
            &daemon,
            PaneLabel {
                session: "build",
                pane: "right",
            },
            None,
        )
        .unwrap();
        let (status, stdout) = fake.run(&command);
        assert_eq!(stdout, "ngterm: build (right) on buildbox disconnected, exit 23\n");
        assert_eq!(status, Some(23), "a failed connection must not read as a clean exit");
        let calls = fake.calls();
        let keep_alive = ["-o", "ServerAliveInterval=5", "-o", "ServerAliveCountMax=2"];
        assert!(
            calls.windows(4).any(|window| window == keep_alive),
            "a config with no keep-alive gets one: {calls:?}"
        );
        assert_eq!(calls.last().map(String::as_str), argv_last(&daemon, None).as_deref());
    }

    fn argv_last(daemon: &str, lead: Option<&LeadAttachment>) -> Option<String> {
        attach_command("buildbox", &endpoint(), daemon, lead)
            .ok()
            .and_then(|argv| argv.last().cloned())
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "runs its fake ssh and the pane command with the POSIX shell /bin/sh, which Windows has not"
    )]
    fn without_a_lead_nonce_a_lost_connection_exits_as_before() {
        let fake = FakeRemote::new(255);
        let command = attach_pane_command("buildbox", &endpoint(), &daemon_name(Uuid::new_v4()), LABEL, None).unwrap();
        assert_eq!(
            fake.run(&command),
            (
                Some(255),
                "ngterm: build (left) on buildbox disconnected, exit 255\n".to_owned()
            )
        );
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "runs its fake ssh and the pane command with the POSIX shell /bin/sh, which Windows has not"
    )]
    fn a_lost_connection_with_a_lead_says_it_is_reconnecting_reports_it_and_waits() {
        let fake = FakeRemote::new(255);
        let attachment = lead("n1", true);
        let command = attach_pane_command(
            "buildbox",
            &endpoint(),
            &daemon_name(Uuid::new_v4()),
            LABEL,
            Some(&attachment),
        )
        .unwrap();
        assert!(command.contains("stty -echo 2>/dev/null; cat >/dev/null"));
        // stdin is closed, so the wait on `cat` ends at once and the bar is all that printed
        let (status, stdout) = fake.run(&command);
        assert_eq!(status, Some(255));
        assert_eq!(
            stdout,
            "\u{1b}[0m\u{1b}[?1000l\u{1b}[?1002l\u{1b}[?1003l\u{1b}[?1006l\u{1b}[?1004l\u{1b}[?2004l\u{1b}[?2031l\
             \u{1b}[?2048l\r\n\u{1b}[30;43m Connection to buildbox lost · reconnecting… · any key retries now \
             \u{1b}[K\u{1b}[0m\n\u{1b}]2;ngterm-remote;n1:lost\u{7}\u{1b}[?25l"
        );
        // any other exit keeps the plain line and status
        let fake = FakeRemote::new(1);
        assert_eq!(
            fake.run(&command),
            (
                Some(1),
                "ngterm: build (left) on buildbox disconnected, exit 1\n".to_owned()
            )
        );
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "runs its fake ssh and the pane command with the POSIX shell /bin/sh, which Windows has not"
    )]
    fn a_hostile_session_name_cannot_escape_the_diagnostic_line() {
        let fake = FakeRemote::new(0);
        let name = format!("build'; touch {}; echo '", fake.marker().display());
        let command = attach_pane_command(
            "buildbox",
            &endpoint(),
            &daemon_name(Uuid::new_v4()),
            PaneLabel {
                session: &name,
                pane: "left",
            },
            None,
        )
        .unwrap();
        fake.run(&command);
        assert!(!fake.marker().exists(), "the name is data, never shell syntax");
    }
}
