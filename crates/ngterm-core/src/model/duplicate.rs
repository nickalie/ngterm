//! Duplicating a session (upstream's `AppStore+Duplicate.swift`), and the store half of Duplicate as Companion
//! (upstream's `duplicateAsCompanion` in `AppStore+Companions.swift`), which `crate::duplicate_replay` plans.

use std::path::Path;

use uuid::Uuid;

use super::SplitAxis;
use super::companions::CompanionRefusal;
use super::session::Session;
use super::store::{AppStore, NewSession};

/// One pane of a Duplicate as Companion copy: where it starts, and the creation command it ends with (`None` for a
/// plain shell) with the argv that command quotes ([`super::PaneLaunch::argv`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneCopy {
    pub cwd: String,
    pub command: Option<String>,
    pub argv: Option<Vec<String>>,
}

/// What Duplicate as Companion creates: the left pane, the right one for a split source with its axis and ratio.
#[derive(Debug, Clone, PartialEq)]
pub struct CompanionCopy {
    pub left: PaneCopy,
    pub right: Option<PaneCopy>,
    pub axis: SplitAxis,
    pub ratio: Option<f64>,
    /// Select the copy; `false` creates it in the background.
    pub select: bool,
}

impl Session {
    /// Where a local shell started for this session runs: `path` itself, except that a remote session's path names
    /// a directory on its host, so it is used only when it exists here too, else `home` (upstream's
    /// `localWorkingDirectory`).
    #[must_use]
    pub fn local_working_directory(&self, path: &str, home: &str) -> String {
        if self.remote_host().is_none() || Path::new(path).is_dir() {
            path.to_owned()
        } else {
            home.to_owned()
        }
    }
}

impl AppStore {
    /// Duplicates a session: a fresh shell in the same workspace, right after the source, rooted at the source's
    /// focused-pane cwd. Only the directory carries over: an automatic name, no split, scratch, status, flag or
    /// command. Selected, as a new session is. `None` when no session matches.
    pub fn duplicate_session(&mut self, id: Uuid, home: &str) -> Option<Uuid> {
        let session = self.session(id)?;
        let cwd = session.local_working_directory(session.focused_cwd(), home);
        let location = self.session_location(id)?;
        self.add_session(
            location.workspace,
            NewSession {
                at: Some(location.index + 1),
                ..NewSession::at_cwd(cwd)
            },
        )
    }

    /// Creates `copy` as a companion of `source`, after its group (a sibling of a companion source): each pane runs its
    /// command and closes when it exits, and a split copy keeps the source's axis and ratio with the left pane focused
    /// (#842).
    ///
    /// # Errors
    ///
    /// As [`AppStore::add_companion`].
    pub fn duplicate_as_companion(
        &mut self,
        source: Uuid,
        copy: CompanionCopy,
        home: &str,
    ) -> Result<Uuid, CompanionRefusal> {
        let slot = self.companion_slot(source, Some(copy.left.cwd), home)?;
        let spec = NewSession {
            command: copy.left.command,
            at: Some(slot.index),
            select: copy.select,
            parent: Some(slot.parent),
            ..NewSession::at_cwd(slot.cwd)
        };
        let id = self
            .add_session(slot.workspace, spec)
            .ok_or(CompanionRefusal::NoSuchSession)?;
        if let Some(session) = self.session_mut(id) {
            session.launch.argv = copy.left.argv;
        }
        if let Some(right) = copy.right {
            if let Some(session) = self.session_mut(id) {
                session.initial_split_cwd = Some(right.cwd);
                session.split_launch.command = right.command;
                session.split_launch.argv = right.argv;
            }
            self.set_split_visibility(id, true, Some(copy.axis));
            self.set_split_focus(id, false);
            if let Some(ratio) = copy.ratio {
                self.apply_split_ratio(ratio, id);
            }
        }
        Ok(id)
    }
}
