//! The structural actions a remote window asks its server to make (upstream's `RemoteOperation` in
//! `RemoteCatalogActions.swift`): what each one names, the server's `ngtermctl` words for it, and the row its pending or
//! unknown mark shows with.

use uuid::Uuid;

use crate::pane_env::uuid_string;

/// Where [`RemoteOperation::MoveSession`] puts a session: another workspace, beside an anchor session, or one step
/// within its own workspace (`up`, `down`, `top`, `bottom`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteSessionPlacement {
    Workspace(Uuid),
    After(Uuid),
    Before(Uuid),
    Step(String),
}

/// What a new session or companion starts with on the server; `None` leaves the server's default.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RemoteShellOptions {
    pub name: Option<String>,
    pub cwd: Option<String>,
    pub command: Option<String>,
}

impl RemoteShellOptions {
    /// Each value joined to its flag, so a value that starts with `-` is never read as an option.
    fn words(&self) -> Vec<String> {
        [
            ("--name", &self.name),
            ("--cwd", &self.cwd),
            ("--command", &self.command),
        ]
        .into_iter()
        .filter_map(|(flag, value)| value.as_ref().map(|value| format!("{flag}={value}")))
        .collect()
    }
}

/// The row an operation acts on, which its pending or unknown mark is shown with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RemoteOperationTarget {
    Session(Uuid),
    Workspace(Uuid),
}

/// One structural action a remote window asks its server to make.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteOperation {
    /// Lands in `workspace`, beside `anchor` when it is `After` or `Before`.
    NewSession {
        workspace: Uuid,
        anchor: Option<RemoteSessionPlacement>,
        shell: RemoteShellOptions,
    },
    NewCompanion {
        parent: Uuid,
        shell: RemoteShellOptions,
    },
    SetSplit {
        session: Uuid,
        shown: bool,
        axis: Option<String>,
    },
    /// Shows or hides the server's scratch; a command replaces the scratch there with one that runs it.
    SetScratch {
        session: Uuid,
        shown: bool,
        command: Option<String>,
    },
    CloseSplit(Uuid),
    Swap(Uuid),
    RenameSession {
        session: Uuid,
        name: String,
    },
    MoveSession {
        session: Uuid,
        to: RemoteSessionPlacement,
    },
    CloseSession(Uuid),
    NewWorkspace {
        name: Option<String>,
    },
    RenameWorkspace {
        workspace: Uuid,
        name: String,
    },
    DeleteWorkspace(Uuid),
    MoveWorkspace {
        workspace: Uuid,
        to: String,
    },
}

impl RemoteOperation {
    /// A new session in `workspace` with the server's defaults.
    #[must_use]
    pub fn new_session(workspace: Uuid) -> Self {
        Self::NewSession {
            workspace,
            anchor: None,
            shell: RemoteShellOptions::default(),
        }
    }

    /// The action's name in its mark and its messages.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::NewSession { .. } => "New Session",
            Self::NewCompanion { .. } => "New Companion",
            Self::SetSplit { shown: true, .. } => "Show Split",
            Self::SetSplit { shown: false, .. } => "Hide Split",
            Self::SetScratch { shown: true, .. } => "Show Scratch",
            Self::SetScratch { shown: false, .. } => "Hide Scratch",
            Self::CloseSplit(_) => "Close Split",
            Self::Swap(_) => "Swap Panes",
            Self::RenameSession { .. } => "Rename Session",
            Self::MoveSession { .. } => "Move Session",
            Self::CloseSession(_) => "Close Session",
            Self::NewWorkspace { .. } => "New Workspace",
            Self::RenameWorkspace { .. } => "Rename Workspace",
            Self::DeleteWorkspace(_) => "Delete Workspace",
            Self::MoveWorkspace { .. } => "Move Workspace",
        }
    }

    /// The row the operation acts on; `None` for a new workspace.
    #[must_use]
    pub const fn target(&self) -> Option<RemoteOperationTarget> {
        match self {
            Self::NewSession { workspace, .. }
            | Self::RenameWorkspace { workspace, .. }
            | Self::DeleteWorkspace(workspace)
            | Self::MoveWorkspace { workspace, .. } => Some(RemoteOperationTarget::Workspace(*workspace)),
            Self::NewCompanion { parent: session, .. }
            | Self::SetSplit { session, .. }
            | Self::SetScratch { session, .. }
            | Self::CloseSplit(session)
            | Self::Swap(session)
            | Self::RenameSession { session, .. }
            | Self::MoveSession { session, .. }
            | Self::CloseSession(session) => Some(RemoteOperationTarget::Session(*session)),
            Self::NewWorkspace { .. } => None,
        }
    }

    /// Whether the operation ends the scratch the window saw, which the server must still have.
    pub(crate) fn replaces_scratch(&self) -> bool {
        matches!(self, Self::SetScratch { shown: true, command: Some(command), .. } if !command.is_empty())
    }

    /// Whether the server must find the session's panes as the window saw them.
    pub(crate) const fn pins_panes(&self) -> bool {
        matches!(self, Self::SetSplit { .. } | Self::CloseSplit(_) | Self::Swap(_))
    }

    /// The repeats a pending operation holds back: the same action on the same row.
    pub(crate) fn key(&self) -> String {
        let row = match self.target() {
            Some(RemoteOperationTarget::Session(id) | RemoteOperationTarget::Workspace(id)) => uuid_string(id),
            None => String::new(),
        };
        format!("{}:{row}", self.label())
    }

    /// The server's `ngtermctl` words; user text rides in [`RemoteOperation::operands`] after a `--`, so a name that
    /// starts with `-` stays a name.
    pub(crate) fn words(&self) -> Vec<String> {
        let words: Vec<String> = match self {
            Self::NewSession {
                workspace,
                anchor,
                shell,
            } => {
                let place = match anchor {
                    Some(RemoteSessionPlacement::After(id)) => ["--after".to_owned(), uuid_string(*id)],
                    Some(RemoteSessionPlacement::Before(id)) => ["--before".to_owned(), uuid_string(*id)],
                    _ => ["--workspace".to_owned(), uuid_string(*workspace)],
                };
                let mut words = vec!["session".to_owned(), "new".to_owned()];
                words.extend(place);
                words.extend(shell.words());
                words
            }
            Self::NewCompanion { parent, shell } => {
                let mut words = owned(&["session", "new", "--parent"]);
                words.push(uuid_string(*parent));
                words.extend(shell.words());
                words
            }
            Self::SetSplit { session, shown, axis } => {
                let mut words = owned(&["session", "split", "visibility", on_off(*shown), "--target"]);
                words.push(uuid_string(*session));
                if let Some(axis) = axis {
                    words.extend(["--axis".to_owned(), axis.clone()]);
                }
                words
            }
            Self::SetScratch {
                session,
                shown,
                command,
            } => {
                let mut words = owned(&["session", "scratch", on_off(*shown), "--target"]);
                words.push(uuid_string(*session));
                if let Some(command) = command {
                    words.push(format!("--command={command}"));
                }
                words
            }
            Self::CloseSplit(session) => targeted(&["session", "split", "close"], *session),
            Self::Swap(session) => targeted(&["session", "swap"], *session),
            Self::RenameSession { session, .. } => targeted(&["session", "rename"], *session),
            Self::MoveSession { session, to } => {
                let mut words = targeted(&["session", "move"], *session);
                match to {
                    RemoteSessionPlacement::Workspace(_) => {}
                    RemoteSessionPlacement::After(anchor) => words.extend(["--after".to_owned(), uuid_string(*anchor)]),
                    RemoteSessionPlacement::Before(anchor) => {
                        words.extend(["--before".to_owned(), uuid_string(*anchor)]);
                    }
                    RemoteSessionPlacement::Step(step) => words.extend(["--to".to_owned(), step.clone()]),
                }
                words
            }
            Self::CloseSession(session) => targeted(&["session", "close"], *session),
            Self::NewWorkspace { .. } => owned(&["workspace", "new"]),
            Self::RenameWorkspace { workspace, .. } => targeted(&["workspace", "rename"], *workspace),
            Self::DeleteWorkspace(workspace) => targeted(&["workspace", "delete"], *workspace),
            Self::MoveWorkspace { workspace, to } => {
                let mut words = targeted(&["workspace", "move"], *workspace);
                words.extend(["--to".to_owned(), to.clone()]);
                words
            }
        };
        words
    }

    /// What follows the words after `--`: user text that may start with `-`.
    pub(crate) fn operands(&self) -> Vec<String> {
        match self {
            Self::RenameSession { name, .. } | Self::RenameWorkspace { name, .. } => vec![name.clone()],
            Self::NewWorkspace { name } => name.iter().cloned().collect(),
            Self::MoveSession {
                to: RemoteSessionPlacement::Workspace(workspace),
                ..
            } => vec![uuid_string(*workspace)],
            _ => Vec::new(),
        }
    }

    /// A sidebar drop of `id` at `destination`, an index among `workspace`'s other sessions (`sessions`), as the
    /// anchor the server places it by.
    #[must_use]
    pub fn drop_placement(id: Uuid, workspace: Uuid, sessions: &[Uuid], destination: usize) -> RemoteSessionPlacement {
        let others: Vec<Uuid> = sessions.iter().copied().filter(|other| *other != id).collect();
        if let Some(anchor) = others.get(destination) {
            return RemoteSessionPlacement::Before(*anchor);
        }
        others
            .last()
            .map_or(RemoteSessionPlacement::Workspace(workspace), |last| {
                RemoteSessionPlacement::After(*last)
            })
    }

    /// A workspace drop from `source` to `destination` among `count` workspaces as the one `workspace.move --to` step
    /// that lands it there; `None` when no single step does.
    #[must_use]
    pub fn workspace_step(source: usize, destination: usize, count: usize) -> Option<&'static str> {
        if destination == source {
            return None;
        }
        if destination == 0 {
            return Some("top");
        }
        if destination + 1 == count {
            return Some("bottom");
        }
        if destination + 1 == source {
            return Some("up");
        }
        (destination == source + 1).then_some("down")
    }
}

fn owned(words: &[&str]) -> Vec<String> {
    words.iter().map(|word| (*word).to_owned()).collect()
}

fn targeted(words: &[&str], id: Uuid) -> Vec<String> {
    let mut words = owned(words);
    words.extend(["--target".to_owned(), uuid_string(id)]);
    words
}

const fn on_off(shown: bool) -> &'static str {
    if shown { "on" } else { "off" }
}

#[cfg(test)]
#[path = "operation_tests.rs"]
mod tests;
