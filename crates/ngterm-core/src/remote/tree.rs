//! `live.tree`: the attachable sessions, this app's own or another machine's (upstream's `RemoteTree.swift`, the
//! `remoteTree`/`localAttachableSessions` half of `agterm/Control/ControlServer+Zmx.swift`, and the parts of
//! `RemoteSession.swift` and `RemoteCommandProcessRunner.swift` the remote form runs on).
//!
//! The local form joins every open window's projection against the daemon inventory: only a session whose every pane
//! resolves to a claimed, running daemon is offered, since an attach creates a missing session and would hand back a
//! fresh shell wearing the session's name. The remote form runs that same bare form on the far side over ssh and reads
//! its one document back.

use std::io::{Read, Seek, SeekFrom};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use ngterm_proto::{
    ControlLiveInventory, ControlRemoteMember, ControlRemotePane, ControlRemoteSession, ControlRemoteTree,
    ControlResult, ControlSessionNode, ControlWorkspaceNode, Response,
};

use super::frames;
use crate::command_restore::shell_quoted_line;
use crate::live::is_daemon_name;

/// How long the remote form waits for the far side's answer (upstream's `remoteTreeDeadline`).
pub const TREE_DEADLINE: Duration = Duration::from_secs(10);
/// What a remote command that did not finish within its deadline answers.
pub const NO_ANSWER_IN_TIME: &str = "the remote did not answer in time";
/// Why a remote session id that is not one plain token ([`is_plain`]) is refused before anything runs.
pub const INVALID_REMOTE_SESSION: &str = "invalid remote session";

/// A host or remote-session token: not empty, no whitespace, no control characters (upstream's
/// `RemoteSession.isPlain`). One predicate decides what may reach both an argv and an error message.
#[must_use]
pub fn is_plain(value: &str) -> bool {
    !value.is_empty()
        && !value
            .chars()
            .any(|ch| ch.is_whitespace() || u32::from(ch) < 0x20 || u32::from(ch) == 0x7f)
}

/// Whether `value` may be handed to ssh as its destination: plain, and no leading `-` that ssh would read as an option
/// (upstream's `RemoteSession.isHost`), past a peer pin ([`super::peer::destination`]) too.
#[must_use]
pub fn is_host(value: &str) -> bool {
    let destination = super::peer::ssh_destination(value);
    is_plain(value) && !destination.is_empty() && !destination.starts_with('-')
}

/// Why an ssh invocation could not be built (upstream's `RemoteSession.InvocationError`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationError {
    EmptyHost,
    InvalidHost,
    /// A session, job or daemon name that is not one plain token of the expected shape.
    InvalidSession,
    /// A live-session endpoint path carrying control characters.
    InvalidEndpoint,
}

pub(crate) fn validate(host: &str) -> Result<(), InvocationError> {
    if host.is_empty() {
        return Err(InvocationError::EmptyHost);
    }
    if !is_host(host) {
        return Err(InvocationError::InvalidHost);
    }
    Ok(())
}

/// sshd runs a remote command with a bare system PATH and a non-interactive shell reads no profile, so an installed
/// CLI is otherwise not found. The Linux build's install directories are appended, so a user's own CLI earlier
/// on PATH still wins (upstream's `cliPathPrefix`, with `CommandPath.standardDirectories` for Linux).
#[must_use]
pub fn cli_path_prefix() -> String {
    "PATH=\"$PATH:$HOME/.local/bin:/usr/local/bin\"".to_owned()
}

/// This build's CLI, which the remote calls run on an ngterm far side. A listing that pins no app falls back to agterm's
/// when this one is missing or fails ([`super::ssh::tree_tail_for`], [`super::peer`]); a far side with neither fails
/// with the shell's exit 127.
pub const REMOTE_CLI: &str = "ngtermctl";

/// The remote `/bin/sh -c CHAIN` the account's shell runs as one ordinary command: the widened `PATH`, then
/// `invocation`, which calls [`REMOTE_CLI`]. The chain is wrapped in `/bin/sh -c`, since sshd runs it through the
/// account's shell, where a bare `VAR=value` is a syntax error in fish.
#[must_use]
pub fn remote_cli_chain(invocation: &str) -> String {
    let chain = format!("{} && {invocation}", cli_path_prefix());
    shell_quoted_line(&["/bin/sh", "-c", chain.as_str()])
}

/// The `PATH` widening for `peer`'s CLI: this build's install directories, or an agterm Mac's (upstream's
/// `cliPathPrefix`).
#[must_use]
pub fn peer_path_prefix(peer: super::peer::RemotePeer) -> String {
    match peer {
        super::peer::RemotePeer::Ngterm => cli_path_prefix(),
        super::peer::RemotePeer::Agterm => {
            format!(
                "PATH=\"$PATH:$HOME/.local/bin:{}\"",
                super::peer::agterm::PATH_DIRECTORIES
            )
        }
    }
}

/// [`remote_cli_chain`] for `peer`, whose `invocation` calls [`super::peer::cli`]`(peer)`.
#[must_use]
pub fn peer_cli_chain(peer: super::peer::RemotePeer, invocation: &str) -> String {
    let chain = format!("{} && {invocation}", peer_path_prefix(peer));
    shell_quoted_line(&["/bin/sh", "-c", chain.as_str()])
}

/// The ssh options every invocation carries: no pty unless `interactive`, never a prompt, a bounded handshake.
#[must_use]
pub fn ssh_arguments(host: &str, connect_timeout: u32, interactive: bool) -> Vec<String> {
    vec![
        "ssh".to_owned(),
        if interactive { "-tt" } else { "-T" }.to_owned(),
        "-o".to_owned(),
        "BatchMode=yes".to_owned(),
        "-o".to_owned(),
        format!("ConnectTimeout={connect_timeout}"),
        super::peer::ssh_destination(host).to_owned(),
    ]
}

/// One ssh invocation running the far side's own `live tree`, which answers across every open window in one document
/// (upstream's `RemoteSession.treeCommand`), for a host already validated, in `shell`'s form: [`ssh_arguments`] plus
/// [`super::ssh::tree_tail_for`] the app the host's destination pins, if any.
pub(crate) fn tree_argv(host: &str, shell: super::origin::OriginShell) -> Vec<String> {
    let mut argv = ssh_arguments(host, 5, false);
    argv.push(super::ssh::tree_tail_for(shell, super::peer::destination(host).0));
    argv
}

/// Runs `live tree` on `host`, already validated, learning a Windows origin as it does
/// ([`super::origin::run_learning`]). Blocks.
#[must_use]
pub fn run_tree(runner: &dyn RemoteCommandRunner, host: &str, deadline: Duration) -> RemoteCommandResult {
    super::origin::run_learning(runner, host, deadline, |shell| tree_argv(host, shell))
}

/// `ngtermctl`'s exit status for an argument error, `EX_USAGE` as upstream's.
const USAGE_EXIT: i32 = 64;

/// What running a remote command produced. `status` is ssh's exit code, which carries the far side's own once the
/// connection itself succeeded; -1 when it never ran or did not finish in time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteCommandResult {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

impl RemoteCommandResult {
    /// The far `ngtermctl` rejecting its own command line before sending anything (upstream's `refusedArguments`):
    /// the usage exit, 64, with nothing on stdout, which is what a CLI older than an option or subcommand answers.
    #[must_use]
    pub fn refused_arguments(&self) -> bool {
        self.status == USAGE_EXIT && self.stdout.trim().is_empty()
    }

    fn failed(stderr: impl Into<String>) -> Self {
        Self {
            status: -1,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }
}

/// Runs an ssh invocation off the main thread (upstream's `RemoteCommandRunner`). The app installs
/// [`ProcessRemoteRunner`]; tests install a fake, which is what lets the remote form be covered without a second host.
pub trait RemoteCommandRunner: Send + Sync {
    /// Runs `argv` to its end or `deadline`, whichever comes first. Blocks.
    fn run(&self, argv: &[String], deadline: Duration) -> RemoteCommandResult;
}

/// Runs the command as a child process (upstream's `RemoteCommandProcessRunner`): through `/usr/bin/env` on Unix and by
/// name on Windows ([`local_command`]), so a user's own `ssh` earlier on PATH wins; stdin closed; the deadline the
/// caller's, since ssh's `ConnectTimeout` bounds only the handshake.
///
/// The output goes to unlinked temporary files rather than pipes, read once the command exited: nothing blocks reading
/// it, so a background helper of a PATH-resolved ssh wrapper that keeps the output open strands no reader thread or
/// descriptor here (upstream cancels its pipe readers instead), and what it wrote by the exit is the answer.
#[derive(Debug, Default, Clone, Copy)]
pub struct ProcessRemoteRunner;

impl RemoteCommandRunner for ProcessRemoteRunner {
    fn run(&self, argv: &[String], deadline: Duration) -> RemoteCommandResult {
        let Some(executable) = argv.first() else {
            return RemoteCommandResult::failed("no command to run");
        };
        let could_not_run =
            |error: std::io::Error| RemoteCommandResult::failed(format!("could not run {executable}: {error}"));
        let (mut stdout, mut stderr) = match (tempfile::tempfile(), tempfile::tempfile()) {
            (Ok(stdout), Ok(stderr)) => (stdout, stderr),
            (Err(error), _) | (_, Err(error)) => return could_not_run(error),
        };
        let spawned = stdout.try_clone().and_then(|out| {
            let err = stderr.try_clone()?;
            local_command(argv)?
                .stdin(Stdio::null())
                .stdout(Stdio::from(out))
                .stderr(Stdio::from(err))
                .spawn()
        });
        let mut child = match spawned {
            Ok(child) => child,
            Err(error) => return could_not_run(error),
        };
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) if started.elapsed() < deadline => thread::sleep(Duration::from_millis(20)),
                Ok(None) | Err(_) => break None,
            }
        };
        let Some(status) = status else {
            end_child(&mut child);
            // stdout stays empty on a miss: the caller prefers a remote error parsed from stdout, and a partial one
            // must not replace this diagnostic. What ssh last said is kept, since a stall is often ssh waiting on
            // something it printed, such as Tailscale SSH's re-authentication link.
            return RemoteCommandResult::failed(no_answer_in_time(&mut stderr));
        };
        RemoteCommandResult {
            status: status.code().unwrap_or(-1),
            stdout: read_capture(&mut stdout),
            stderr: read_capture(&mut stderr),
        }
    }
}

/// The command that runs `argv` on this machine with its program found on `PATH`, so a user's own `ssh` earlier there
/// wins: through `/usr/bin/env` on Unix; by name on Windows, which has no `env`, and without a console window, which a
/// GUI app's console child would otherwise open. Every ssh a viewer runs in the background starts this way.
///
/// # Errors
///
/// `argv` is empty.
pub fn local_command(argv: &[String]) -> std::io::Result<Command> {
    let (program, arguments) = argv
        .split_first()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::InvalidInput, "an empty command"))?;
    if cfg!(windows) {
        let mut command = Command::new(program);
        command.args(arguments);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            command.creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW);
        }
        Ok(command)
    } else {
        let mut command = Command::new("/usr/bin/env");
        command.args(argv);
        Ok(command)
    }
}

/// [`NO_ANSWER_IN_TIME`], followed by the last usable line of what the command wrote to `stderr`, if it wrote one. Only
/// the capture's tail is read, so a chatty wrapper cannot make the read large.
fn no_answer_in_time(stderr: &mut std::fs::File) -> String {
    let tail_limit = u64::try_from(crate::keymap::failure::TAIL_LIMIT).unwrap_or(u64::MAX);
    let mut tail = Vec::new();
    let start = stderr.seek(SeekFrom::End(0)).unwrap_or(0).saturating_sub(tail_limit);
    if stderr.seek(SeekFrom::Start(start)).is_ok() {
        let _ = stderr.take(tail_limit).read_to_end(&mut tail);
    }
    match crate::keymap::failure::detail(&tail) {
        Some(said) => format!("{NO_ANSWER_IN_TIME} ({said})"),
        None => NO_ANSWER_IN_TIME.to_owned(),
    }
}

/// What a capture file holds from its start, decoded lossily.
fn read_capture(file: &mut std::fs::File) -> String {
    let mut bytes = Vec::new();
    if file.seek(SeekFrom::Start(0)).is_ok() {
        let _ = file.read_to_end(&mut bytes);
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Kills and reaps a child past its deadline. Upstream terminates first and kills past a grace; the core stays free
/// of signal APIs, and an ssh client has nothing to flush.
fn end_child(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// One open window's projection, with the identity the tree payload does not carry.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteWindowProjection {
    pub id: String,
    pub name: String,
    pub tree: ngterm_proto::ControlTree,
}

/// Why a session list could not be produced or read (upstream's `RemoteTreeMerger.MergeError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// The remote answered, but not with a readable `live tree` document: an older CLI, or a truncated read.
    MalformedProjection,
    /// The inventory does not say where the session host and its daemons live, so nothing can be attached.
    MissingEndpoint,
    /// The remote command itself failed; carries what it said.
    RemoteFailed(String),
}

impl MergeError {
    /// The error a caller reads.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::MalformedProjection => "the remote did not return a readable session list".to_owned(),
            Self::MissingEndpoint => "the remote ngterm is too old to report its live-session endpoint".to_owned(),
            Self::RemoteFailed(reason) => reason.clone(),
        }
    }
}

/// The remote's own explanation for a nonzero exit, or `None` when it gave none (upstream's `remoteError(stdout:)`).
/// `ngtermctl --json` prints a refusal to stdout and exits nonzero, so stderr is empty on exactly these failures.
#[must_use]
pub fn remote_error(stdout: &str) -> Option<String> {
    let response: Response<ControlResult> = serde_json::from_str(super::peer::split_answer(stdout).1).ok()?;
    if response.ok {
        return None;
    }
    response
        .error
        .map(|error| error.trim().to_owned())
        .filter(|error| !error.is_empty())
}

/// Reads back what the far side's own `live tree` printed (upstream's `decode(stdout:)`), stamping on its endpoint the
/// app that answered ([`super::peer::split_answer`]). The caller rejects a nonzero exit before calling this, and stamps
/// the host it was given onto the result.
///
/// # Errors
///
/// The output is not a response, the far side refused, or an older far side answered without a session list.
pub fn decode(stdout: &str) -> Result<ControlRemoteTree, MergeError> {
    let (peer, stdout) = super::peer::split_answer(stdout);
    let mut tree = decode_answer(stdout)?;
    tree.endpoint.peer = (peer != super::peer::RemotePeer::Ngterm).then_some(peer);
    Ok(tree)
}

fn decode_answer(stdout: &str) -> Result<ControlRemoteTree, MergeError> {
    let response: Response<ControlResult> =
        serde_json::from_str(stdout).map_err(|_| MergeError::MalformedProjection)?;
    if !response.ok {
        return Err(MergeError::RemoteFailed(
            response
                .error
                .unwrap_or_else(|| "the remote session list could not be read".to_owned()),
        ));
    }
    response
        .result
        .and_then(|result| result.remote)
        .ok_or(MergeError::MalformedProjection)
}

/// Every attachable session across `windows`, flat, each naming where it lives (upstream's `candidates`). An empty
/// list is a successful answer, which does not tell a store outside Live sessions from one with nothing eligible.
///
/// # Errors
///
/// The inventory carries no endpoint.
pub fn candidates(
    windows: &[RemoteWindowProjection],
    inventory: &ControlLiveInventory,
) -> Result<ControlRemoteTree, MergeError> {
    let endpoint = inventory.endpoint.clone().ok_or(MergeError::MissingEndpoint)?;
    let mut daemons: std::collections::HashMap<&str, std::collections::HashMap<&str, &str>> =
        std::collections::HashMap::new();
    for entry in &inventory.entries {
        if entry.state != "claimed" || entry.observation != "running" {
            continue;
        }
        let (Some(session), Some(pane)) = (entry.session_id.as_deref(), entry.pane.as_deref()) else {
            continue;
        };
        // every row returned must be attachable, so a name the attach would refuse never reaches the caller
        if !is_daemon_name(&entry.daemon) {
            continue;
        }
        daemons.entry(session).or_default().insert(pane, entry.daemon.as_str());
    }
    let none = std::collections::HashMap::new();
    let terminals: Vec<ControlRemoteSession> = windows
        .iter()
        .flat_map(|window| {
            window
                .tree
                .workspaces
                .iter()
                .flat_map(move |workspace| workspace.sessions.iter().map(move |node| (window, workspace, node)))
        })
        .filter_map(|(window, workspace, node)| {
            remote_session(node, window, workspace, daemons.get(node.id.as_str()).unwrap_or(&none))
        })
        .collect();
    // a page companion owns no daemon, so it is listed with no panes when its parent is: it attaches under that
    // parent's row through `session.companion.page.attach`, never on its own
    let attachable: std::collections::HashSet<&str> = terminals.iter().map(|session| session.id.as_str()).collect();
    let pages: Vec<ControlRemoteSession> = nodes(windows)
        .filter(|(_, _, node)| node.content.as_deref() == Some("page"))
        .filter_map(|(window, workspace, node)| {
            let parent = node.parent.as_deref().filter(|parent| attachable.contains(parent))?;
            Some(ControlRemoteSession {
                id: node.id.clone(),
                name: node.name.clone(),
                window_id: window.id.clone(),
                window_name: window.name.clone(),
                workspace_id: workspace.id.clone(),
                workspace_name: workspace.name.clone(),
                context: None,
                cwd: node.cwd.clone(),
                split_axis: None,
                panes: Vec::new(),
                status: None,
                status_color: None,
                status_shape: None,
                parent: Some(parent.to_owned()),
                content: Some("page".to_owned()),
                companions: None,
            })
        })
        .collect();
    Ok(ControlRemoteTree {
        host: None,
        endpoint,
        sessions: terminals.into_iter().chain(pages).collect(),
        presentation: Some(frames::VERSION),
    })
}

/// Every session node of `windows`, with the window and workspace it sits in.
fn nodes(
    windows: &[RemoteWindowProjection],
) -> impl Iterator<Item = (&RemoteWindowProjection, &ControlWorkspaceNode, &ControlSessionNode)> {
    windows.iter().flat_map(|window| {
        window
            .tree
            .workspaces
            .iter()
            .flat_map(move |workspace| workspace.sessions.iter().map(move |node| (window, workspace, node)))
    })
}

fn remote_session(
    node: &ControlSessionNode,
    window: &RemoteWindowProjection,
    workspace: &ControlWorkspaceNode,
    daemons: &std::collections::HashMap<&str, &str>,
) -> Option<ControlRemoteSession> {
    // a claimed, running daemon is no proof the owner shows it: a launch that fell back from live keeps its daemons
    // while showing ordinary shells, and `backedByLive` is the store's own answer
    if node.backed_by_live != Some(true) {
        return None;
    }
    let pane = |role: &str, daemon: &str, foreground: Option<&Vec<String>>| ControlRemotePane {
        pane: role.to_owned(),
        daemon: daemon.to_owned(),
        foreground: foreground.cloned(),
    };
    let mut panes = vec![pane("left", daemons.get("left")?, node.foreground.as_ref())];
    let split = node.has_split == Some(true);
    if split {
        panes.push(pane("right", daemons.get("right")?, node.split_foreground.as_ref()));
    }
    Some(ControlRemoteSession {
        id: node.id.clone(),
        name: node.name.clone(),
        window_id: window.id.clone(),
        window_name: window.name.clone(),
        workspace_id: workspace.id.clone(),
        workspace_name: workspace.name.clone(),
        context: node.context.clone(),
        cwd: node.cwd.clone(),
        split_axis: if split { node.split_axis.clone() } else { None },
        panes,
        status: node.status.clone(),
        status_color: node.status_color.clone(),
        status_shape: node.status_shape.clone(),
        parent: node.parent.clone(),
        content: None,
        // Membership comes from the unfiltered workspace, including rows that cannot attach.
        companions: node.parent.is_none().then(|| {
            workspace
                .sessions
                .iter()
                .filter(|member| member.parent.as_deref() == Some(node.id.as_str()))
                .map(|member| ControlRemoteMember {
                    id: member.id.clone(),
                    name: member.name.clone(),
                })
                .collect()
        }),
    })
}

#[cfg(test)]
mod tests {
    use ngterm_proto::{ControlLiveEndpoint, ControlTree};
    use serde_json::json;
    use uuid::Uuid;

    use super::*;
    use crate::live::daemon_name;

    fn endpoint() -> ControlLiveEndpoint {
        ControlLiveEndpoint {
            peer: None,
            attach_bridge: None,
            executable: "/opt/ngterm/bin/ngterm-session".to_owned(),
            socket_directory: "/tmp/ngterm-live-abc".to_owned(),
        }
    }

    fn node(id: &str, name: &str, cwd: &str, split: bool, backed: Option<bool>) -> ControlSessionNode {
        let mut node: ControlSessionNode = serde_json::from_value(json!({
            "id": id, "name": name, "cwd": cwd, "active": false, "split": split, "overlay": false,
            "scratch": false, "flagged": false
        }))
        .unwrap();
        node.has_split = split.then_some(true);
        node.backed_by_live = backed;
        node.split_axis = split.then(|| "vertical".to_owned());
        node
    }

    fn workspace(id: &str, name: &str, sessions: Vec<ControlSessionNode>) -> ControlWorkspaceNode {
        let mut workspace: ControlWorkspaceNode =
            serde_json::from_value(json!({"id": id, "name": name, "active": true, "sessions": []})).unwrap();
        workspace.sessions = sessions;
        workspace
    }

    fn window(id: &str, name: &str, workspaces: Vec<ControlWorkspaceNode>) -> RemoteWindowProjection {
        let mut tree: ControlTree = serde_json::from_value(json!({"workspaces": []})).unwrap();
        tree.workspaces = workspaces;
        RemoteWindowProjection {
            id: id.to_owned(),
            name: name.to_owned(),
            tree,
        }
    }

    fn entry(session: &str, pane: &str, daemon: &str, state: &str, observation: &str) -> serde_json::Value {
        json!({"daemon": daemon, "state": state, "observation": observation, "sessionID": session, "pane": pane})
    }

    /// Built from raw JSON, so the fixture pins the wire shape rather than whatever the encoder emits.
    fn inventory(entries: &[serde_json::Value], with_endpoint: bool) -> ControlLiveInventory {
        let mut payload = json!({
            "restore": {"configured": "live", "requestedAtLaunch": "live", "active": "live", "restartRequired": false},
            "inventoryComplete": true,
            "entries": entries,
        });
        if with_endpoint {
            payload["endpoint"] = serde_json::to_value(endpoint()).unwrap();
        }
        serde_json::from_value(payload).unwrap()
    }

    fn running(session: &str, pane: &str, daemon: &str) -> serde_json::Value {
        entry(session, pane, daemon, "claimed", "running")
    }

    #[test]
    fn a_plain_session_resolves_to_one_left_daemon() {
        let left = daemon_name(Uuid::new_v4());
        let windows = [window(
            "w-1",
            "main",
            vec![workspace(
                "ws-1",
                "work",
                vec![node("s1", "build", "/repo", false, Some(true))],
            )],
        )];
        let tree = candidates(&windows, &inventory(&[running("s1", "left", &left)], true)).unwrap();
        assert_eq!(tree.host, None, "the local form sshed nowhere");
        assert_eq!(tree.endpoint, endpoint());
        assert_eq!(tree.presentation, Some(frames::VERSION));
        assert_eq!(tree.sessions.len(), 1);
        assert_eq!(
            tree.sessions[0].panes,
            vec![ControlRemotePane {
                pane: "left".to_owned(),
                daemon: left,
                foreground: None
            }]
        );
        assert_eq!(tree.sessions[0].split_axis, None);
        assert_eq!(
            (
                tree.sessions[0].window_name.as_str(),
                tree.sessions[0].workspace_name.as_str()
            ),
            ("main", "work")
        );
    }

    /// Upstream's `RemoteTreeTests.onlyAUsageExitWithNothingOnStdoutIsAnArgumentRefusal`.
    #[test]
    fn only_a_usage_exit_with_nothing_on_stdout_is_an_argument_refusal() {
        let result = |status: i32, stdout: &str, stderr: &str| RemoteCommandResult {
            status,
            stdout: stdout.to_owned(),
            stderr: stderr.to_owned(),
        };
        let cases = [
            (result(64, "", "Error: Unknown option '--parent'"), true),
            (result(64, " \n", ""), true),
            (result(64, r#"{"ok":false,"error":"x"}"#, ""), false),
            (result(1, "", "Error: Unknown option '--parent'"), false),
            (result(255, "", "ssh: connect to host"), false),
        ];
        for (result, refused) in cases {
            assert_eq!(result.refused_arguments(), refused, "{result:?}");
        }
    }

    /// Upstream's `RemoteTreeTests.aPageCompanionIsListedWithNoPanesOnlyUnderAListedParent`.
    #[test]
    fn a_page_companion_is_listed_with_no_panes_only_under_a_listed_parent() {
        let left = daemon_name(Uuid::new_v4());
        let page_node = |id: &str, name: &str, parent: &str| {
            let mut page = node(id, name, "/srv", false, None);
            page.parent = Some(parent.to_owned());
            page.content = Some("page".to_owned());
            page
        };
        let windows = [window(
            "w-1",
            "main",
            vec![workspace(
                "ws-1",
                "work",
                vec![
                    node("s1", "build", "/repo", false, Some(true)),
                    page_node("pg", "report", "s1"),
                    page_node("pg2", "lost", "gone"),
                ],
            )],
        )];
        let tree = candidates(&windows, &inventory(&[running("s1", "left", &left)], true)).unwrap();
        let ids: Vec<&str> = tree.sessions.iter().map(|session| session.id.as_str()).collect();
        assert_eq!(ids, ["s1", "pg"]);
        assert!(tree.sessions[1].panes.is_empty());
        assert_eq!(tree.sessions[1].parent.as_deref(), Some("s1"));
        assert_eq!(tree.sessions[1].content.as_deref(), Some("page"));
        assert_eq!(tree.sessions[0].parent, None);
    }

    /// Upstream's `RemoteTreeTests.aParentListsEveryCompanionInSidebarOrderAttachableOrNot`.
    #[test]
    fn a_parent_lists_every_companion_in_sidebar_order_attachable_or_not() {
        let member = |id: &str, name: &str, backed, content| {
            let mut member = node(id, name, "/repo", false, backed);
            member.parent = Some("s1".to_owned());
            member.content = content;
            member
        };
        let windows = [window(
            "w",
            "main",
            vec![workspace(
                "ws",
                "work",
                vec![
                    node("s1", "build", "/repo", false, Some(true)),
                    member("c1", "logs", Some(true), None),
                    member("pg", "report", None, Some("page".to_owned())),
                    member("c2", "watch", Some(false), None),
                    node("s2", "other", "/x", false, Some(true)),
                ],
            )],
        )];
        let daemons: Vec<_> = (0..3).map(|_| daemon_name(Uuid::new_v4())).collect();
        let entries = [
            running("s1", "left", &daemons[0]),
            running("c1", "left", &daemons[1]),
            running("s2", "left", &daemons[2]),
        ];
        let tree = candidates(&windows, &inventory(&entries, true)).unwrap();
        assert_eq!(
            tree.sessions
                .iter()
                .map(|session| session.id.as_str())
                .collect::<Vec<_>>(),
            ["s1", "c1", "s2", "pg"]
        );
        let session = |id| tree.sessions.iter().find(|session| session.id == id).unwrap();
        assert_eq!(
            session("s1").companions,
            Some(vec![
                ControlRemoteMember {
                    id: "c1".to_owned(),
                    name: "logs".to_owned()
                },
                ControlRemoteMember {
                    id: "pg".to_owned(),
                    name: "report".to_owned()
                },
                ControlRemoteMember {
                    id: "c2".to_owned(),
                    name: "watch".to_owned()
                },
            ])
        );
        assert_eq!(session("s2").companions, Some(vec![]));
        assert_eq!(session("c1").companions, None);
        assert_eq!(session("pg").companions, None);
    }

    /// Upstream's `RemoteTreeTests.aSessionCarriesItsAgentStatusAndAnIdleOneCarriesNone`.
    #[test]
    fn a_session_carries_its_agent_status_and_an_idle_one_carries_none() {
        let (left, right) = (daemon_name(Uuid::new_v4()), daemon_name(Uuid::new_v4()));
        let mut busy = node("s1", "agent", "/repo", false, Some(true));
        busy.status = Some("blocked".to_owned());
        busy.status_pane = Some("left".to_owned());
        busy.status_blink = Some(true);
        busy.status_color = Some("#ff8800".to_owned());
        busy.status_shape = Some("star".to_owned());
        let windows = [window(
            "w-1",
            "main",
            vec![workspace(
                "ws-1",
                "work",
                vec![busy, node("s2", "shell", "/tmp", false, Some(true))],
            )],
        )];
        let entries = [running("s1", "left", &left), running("s2", "left", &right)];
        let tree = candidates(&windows, &inventory(&entries, true)).unwrap();
        let fields = |index: usize| {
            let session = &tree.sessions[index];
            (
                session.status.as_deref(),
                session.status_color.as_deref(),
                session.status_shape.as_deref(),
            )
        };
        assert_eq!(fields(0), (Some("blocked"), Some("#ff8800"), Some("star")));
        assert_eq!(fields(1), (None, None, None));
        let wire = serde_json::to_value(&tree.sessions[1]).unwrap();
        assert!(wire.get("status").is_none(), "an idle row omits its status: {wire}");
    }

    /// Upstream's `RemoteTreeTests.aSessionFromAnOriginWithoutStatusDecodesWithNone`.
    #[test]
    fn a_session_from_an_origin_without_status_decodes_with_none() {
        let stdout = concat!(
            r#"{"ok":true,"result":{"remote":{"endpoint":{"executable":"/z","socketDirectory":"/t"},"#,
            r#""sessions":[{"id":"s1","name":"build","windowID":"w","windowName":"main","workspaceID":"ws","#,
            r#""workspaceName":"work","cwd":"/repo","panes":[{"pane":"left","daemon":"d"}]}]}}}"#
        );
        let session = decode(stdout).unwrap().sessions.remove(0);
        assert_eq!(
            (session.status, session.status_color, session.status_shape),
            (None, None, None)
        );
        assert!(session.companions.is_none(), "old origin membership stays unknown");
    }

    #[test]
    fn every_open_window_and_workspace_is_offered_apart_by_id() {
        let (first, second, third) = (
            daemon_name(Uuid::new_v4()),
            daemon_name(Uuid::new_v4()),
            daemon_name(Uuid::new_v4()),
        );
        let windows = [
            window(
                "w-1",
                "main",
                vec![
                    workspace("ws-1", "dev", vec![node("s1", "build", "/a", false, Some(true))]),
                    workspace("ws-2", "ops", vec![node("s2", "logs", "/var", false, Some(true))]),
                ],
            ),
            window(
                "w-2",
                "main",
                vec![workspace(
                    "ws-3",
                    "dev",
                    vec![node("s3", "build", "/b", false, Some(true))],
                )],
            ),
        ];
        let tree = candidates(
            &windows,
            &inventory(
                &[
                    running("s1", "left", &first),
                    running("s2", "left", &second),
                    running("s3", "left", &third),
                ],
                true,
            ),
        )
        .unwrap();
        let ids = |field: fn(&ControlRemoteSession) -> &str| tree.sessions.iter().map(field).collect::<Vec<_>>();
        assert_eq!(ids(|session| &session.id), ["s1", "s2", "s3"]);
        assert_eq!(ids(|session| &session.window_id), ["w-1", "w-1", "w-2"]);
        assert_eq!(ids(|session| &session.workspace_id), ["ws-1", "ws-2", "ws-3"]);
        assert_eq!(ids(|session| &session.workspace_name), ["dev", "ops", "dev"]);
    }

    #[test]
    fn a_pane_carries_what_runs_in_it_and_the_session_its_context() {
        let left = daemon_name(Uuid::new_v4());
        let mut busy = node("s1", "build", "/repo", false, Some(true));
        busy.foreground = Some(vec!["/usr/bin/tail".to_owned(), "-f".to_owned()]);
        busy.context = Some("release prep".to_owned());
        let windows = [window("w-1", "main", vec![workspace("ws-1", "work", vec![busy])])];
        let tree = candidates(&windows, &inventory(&[running("s1", "left", &left)], true)).unwrap();
        assert_eq!(tree.sessions[0].context.as_deref(), Some("release prep"));
        assert_eq!(
            tree.sessions[0].panes[0].foreground,
            Some(vec!["/usr/bin/tail".to_owned(), "-f".to_owned()])
        );
    }

    #[test]
    fn a_split_carries_both_daemons_and_its_axis_or_is_dropped_half_way() {
        let (left, right) = (daemon_name(Uuid::new_v4()), daemon_name(Uuid::new_v4()));
        let windows = [window(
            "w-1",
            "main",
            vec![workspace(
                "ws-1",
                "work",
                vec![node("s1", "build", "/repo", true, Some(true))],
            )],
        )];
        let both = candidates(
            &windows,
            &inventory(&[running("s1", "left", &left), running("s1", "right", &right)], true),
        )
        .unwrap();
        assert_eq!(
            both.sessions[0]
                .panes
                .iter()
                .map(|pane| (pane.pane.as_str(), pane.daemon.as_str()))
                .collect::<Vec<_>>(),
            [("left", left.as_str()), ("right", right.as_str())]
        );
        assert_eq!(both.sessions[0].split_axis.as_deref(), Some("vertical"));
        let half = candidates(&windows, &inventory(&[running("s1", "left", &left)], true)).unwrap();
        assert!(half.sessions.is_empty());
    }

    #[test]
    fn only_a_claimed_running_ngterm_daemon_of_a_live_backed_session_is_offered() {
        let left = daemon_name(Uuid::new_v4());
        let backed = [window(
            "w-1",
            "main",
            vec![workspace(
                "ws-1",
                "work",
                vec![node("s1", "build", "/repo", false, Some(true))],
            )],
        )];
        for (state, observation) in [("orphan", "running"), ("claimed", "absent"), ("claimed", "unreadable")] {
            let tree = candidates(
                &backed,
                &inventory(&[entry("s1", "left", &left, state, observation)], true),
            )
            .unwrap();
            assert!(tree.sessions.is_empty(), "{state} {observation}");
        }
        let foreign = candidates(&backed, &inventory(&[running("s1", "left", "notes")], true)).unwrap();
        assert!(foreign.sessions.is_empty(), "a name outside ngterm's scheme");
        let plain = [window(
            "w-1",
            "main",
            vec![workspace(
                "ws-1",
                "work",
                vec![node("s1", "build", "/repo", false, Some(false))],
            )],
        )];
        let fallback = candidates(&plain, &inventory(&[running("s1", "left", &left)], true)).unwrap();
        assert!(fallback.sessions.is_empty(), "a session not backed by a live daemon");
        let other = daemon_name(Uuid::new_v4());
        let stray = candidates(
            &backed,
            &inventory(&[running("s1", "left", &left), running("gone", "left", &other)], true),
        )
        .unwrap();
        assert_eq!(stray.sessions.len(), 1, "a row naming no listed session is ignored");
    }

    #[test]
    fn listing_errors_are_named() {
        assert_eq!(
            candidates(&[], &inventory(&[], false)),
            Err(MergeError::MissingEndpoint)
        );
        assert_eq!(
            MergeError::MissingEndpoint.message(),
            "the remote ngterm is too old to report its live-session endpoint"
        );
        let empty = candidates(&[], &inventory(&[], true)).unwrap();
        assert!(
            empty.sessions.is_empty(),
            "no windows is an empty answer, not a failure"
        );
        assert_eq!(
            decode("command not found: ngtermctl"),
            Err(MergeError::MalformedProjection)
        );
        assert_eq!(
            MergeError::MalformedProjection.message(),
            "the remote did not return a readable session list"
        );
        assert_eq!(decode(r#"{"ok":true}"#), Err(MergeError::MalformedProjection));
        assert_eq!(
            decode(r#"{"ok":false,"error":"live sessions are unavailable in this instance"}"#),
            Err(MergeError::RemoteFailed(
                "live sessions are unavailable in this instance".to_owned()
            ))
        );
        assert_eq!(
            decode(r#"{"ok":false}"#).map_err(|error| error.message()),
            Err("the remote session list could not be read".to_owned())
        );
    }

    #[test]
    fn the_far_sides_answer_is_decoded_whole_and_stamped() {
        let left = daemon_name(Uuid::new_v4());
        let remote = ControlRemoteTree {
            host: None,
            endpoint: endpoint(),
            sessions: vec![ControlRemoteSession {
                id: "s1".to_owned(),
                name: "build".to_owned(),
                window_id: "w-1".to_owned(),
                window_name: "main".to_owned(),
                workspace_id: "ws-1".to_owned(),
                workspace_name: "work".to_owned(),
                context: None,
                cwd: "/repo".to_owned(),
                split_axis: None,
                panes: vec![ControlRemotePane {
                    pane: "left".to_owned(),
                    daemon: left,
                    foreground: None,
                }],
                status: Some("blocked".to_owned()),
                status_color: Some("#ff8800".to_owned()),
                status_shape: Some("star".to_owned()),
                parent: None,
                content: None,
                companions: None,
            }],
            presentation: Some(1),
        };
        let stdout = serde_json::to_string(&json!({"ok": true, "result": {"remote": remote}})).unwrap();
        let decoded = decode(&stdout).unwrap();
        assert_eq!(decoded, remote);
        let stamped = decoded.stamped("buildbox");
        assert_eq!(
            stamped,
            ControlRemoteTree {
                host: Some("buildbox".to_owned()),
                ..remote
            }
        );
        let older =
            r#"{"ok":true,"result":{"remote":{"endpoint":{"executable":"/z","socketDirectory":"/t"},"sessions":[]}}}"#;
        assert_eq!(decode(older).unwrap().presentation, None);
    }

    #[test]
    fn the_remotes_own_refusal_is_read_from_stdout() {
        assert_eq!(
            remote_error(r#"{"ok":false,"error":"could not read the live session list"}"#).as_deref(),
            Some("could not read the live session list")
        );
        assert_eq!(remote_error(r#"{"ok":false,"error":"  "}"#), None);
        assert_eq!(
            remote_error(
                r#"{"ok":true,"result":{"remote":{"endpoint":{"executable":"/z","socketDirectory":"/t"},"sessions":[]}}}"#
            ),
            None
        );
        assert_eq!(remote_error("garbage"), None);
    }

    /// Runs the tree command's remote line for `host` the way sshd hands it to the account's shell, in a home whose
    /// `~/.local/bin` holds a stand-in for each of `clis` (name, exit status), each printing its own name and
    /// arguments. Unix only: the far side's shell is `/bin/sh`.
    #[cfg(unix)]
    fn run_remote_line(host: &str, clis: &[(&str, i32)]) -> std::process::Output {
        let home = tempfile::tempdir().unwrap();
        let bin = home.path().join(".local/bin");
        std::fs::create_dir_all(&bin).unwrap();
        for (cli, status) in clis {
            let script = format!("#!/bin/sh\necho {cli} \"$@\"\nexit {status}\n");
            ngterm_host::fakes::write_executable(&bin.join(cli), &script).unwrap();
        }
        let line = tree_argv(host, super::super::origin::OriginShell::Posix).pop().unwrap();
        std::process::Command::new("/bin/sh")
            .args(["-c", &line])
            .env_clear()
            .env("HOME", home.path())
            .env("PATH", "/usr/bin:/bin")
            .output()
            .unwrap()
    }

    /// Stand-in CLIs: each name and the status it exits with.
    #[cfg(unix)]
    type Clis<'a> = &'a [(&'a str, i32)];

    #[cfg(unix)]
    fn answer(output: &std::process::Output) -> (Option<i32>, String) {
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    /// One ssh call tells the peer: ngterm's listing whenever it answers, agterm's under its marker line when ngterm is
    /// missing or fails, ngterm's failure when both fail, and the shell's command-not-found when neither is installed.
    #[cfg(unix)]
    #[test]
    fn the_remote_line_prefers_ngterm_and_marks_an_agterm_answer() {
        let ngterm = "ngtermctl live tree --json\n";
        let agterm = "ngterm-peer: agterm\nagtermctl zmx tree --json\n";
        let cases: [(Clis<'_>, Option<i32>, &str); 6] = [
            (&[("ngtermctl", 0), ("agtermctl", 0)], Some(0), ngterm),
            (&[("ngtermctl", 0)], Some(0), ngterm),
            (&[("agtermctl", 0)], Some(0), agterm),
            (&[("ngtermctl", 1), ("agtermctl", 0)], Some(0), agterm),
            (&[("ngtermctl", 1), ("agtermctl", 1)], Some(1), ngterm),
            (&[("agtermctl", 1)], Some(1), agterm),
        ];
        for (clis, status, stdout) in cases {
            assert_eq!(
                answer(&run_remote_line("buildbox", clis)),
                (status, stdout.to_owned()),
                "{clis:?}"
            );
        }
        let none = run_remote_line("buildbox", &[]);
        assert_eq!(none.status.code(), Some(127), "the shell's command-not-found: {none:?}");
        assert!(none.stdout.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn a_pinned_peer_runs_only_that_apps_listing() {
        let both: &[(&str, i32)] = &[("ngtermctl", 0), ("agtermctl", 0)];
        assert_eq!(
            answer(&run_remote_line("agterm:buildbox", both)),
            (Some(0), "ngterm-peer: agterm\nagtermctl zmx tree --json\n".to_owned())
        );
        assert_eq!(
            answer(&run_remote_line("ngterm:buildbox", &[("agtermctl", 0)])).0,
            Some(127)
        );
        assert_eq!(
            answer(&run_remote_line("ngterm:buildbox", both)),
            (Some(0), "ngtermctl live tree --json\n".to_owned())
        );
    }

    #[test]
    fn the_tree_command_runs_the_bare_form_through_sh_and_refuses_odd_hosts() {
        let argv = tree_argv("ngterm:buildbox", super::super::origin::OriginShell::Posix);
        assert_eq!(
            argv[..7],
            ["ssh", "-T", "-o", "BatchMode=yes", "-o", "ConnectTimeout=5", "buildbox"]
        );
        assert_eq!(
            argv[7],
            r#"'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin:/usr/local/bin" && ngtermctl live tree --json'"#
        );
        let agterm = tree_argv("agterm:me@studio", super::super::origin::OriginShell::Posix);
        assert_eq!(agterm[6], "me@studio");
        assert_eq!(
            agterm[7],
            r#"'/bin/sh' '-c' 'PATH="$PATH:$HOME/.local/bin:/usr/local/bin:/opt/homebrew/bin" && printf '\''%s\n'\'' '\''ngterm-peer: agterm'\'' && exec agtermctl zmx tree --json'"#
        );
        assert_eq!(validate(""), Err(InvocationError::EmptyHost));
        for host in [
            "-oProxyCommand=x",
            "bad host",
            "ho\u{7f}st",
            "a\u{1b}[31m",
            "agterm:",
            "agterm:-oProxyCommand=x",
            "ngterm:-x",
        ] {
            assert_eq!(validate(host), Err(InvocationError::InvalidHost), "{host:?}");
        }
        assert!(is_plain("user@host.example"));
        assert!(!is_plain("\u{a0}nbsp"));
    }

    #[test]
    #[cfg_attr(
        windows,
        ignore = "runs its script with the POSIX shell /bin/sh, which Windows has not"
    )]
    fn the_process_runner_captures_output_and_bounds_the_wait() {
        let runner = ProcessRemoteRunner;
        let argv = |line: &str| vec!["/bin/sh".to_owned(), "-c".to_owned(), line.to_owned()];
        let done = runner.run(&argv("echo out; echo err >&2; exit 3"), Duration::from_secs(5));
        assert_eq!(
            done,
            RemoteCommandResult {
                status: 3,
                stdout: "out\n".to_owned(),
                stderr: "err\n".to_owned()
            }
        );
        // a background helper keeping the output open neither delays the answer nor strands a reader
        let started = Instant::now();
        let lingering = runner.run(&argv("echo out; sleep 5 & exit 0"), Duration::from_secs(5));
        assert_eq!((lingering.status, lingering.stdout.as_str()), (0, "out\n"));
        assert!(started.elapsed() < Duration::from_secs(2), "{:?}", started.elapsed());
        let slow = runner.run(&argv("sleep 5"), Duration::from_millis(100));
        assert_eq!(slow, RemoteCommandResult::failed("the remote did not answer in time"));
        // a stall keeps what ssh last said, cleaned, as Tailscale SSH's re-authentication link is
        let said = r"printf '# Tailscale SSH requires an additional check.\n\033[1m# To authenticate, visit: https://login.example/a/1\033[0m\n\n' >&2";
        let waiting = runner.run(&argv(&format!("{said}; sleep 5")), Duration::from_millis(300));
        assert_eq!(
            waiting,
            RemoteCommandResult::failed(
                "the remote did not answer in time (# To authenticate, visit: https://login.example/a/1)"
            )
        );
        let missing = runner.run(&["/nonexistent/ssh".to_owned()], Duration::from_secs(5));
        assert_eq!(missing.status, 127, "env reports a missing program");
        assert_eq!(runner.run(&[], Duration::from_secs(1)).stderr, "no command to run");
    }
}
