//! Duplicating a session (upstream's `AppStore+Duplicate.swift`), and the store half of Duplicate as Companion
//! (upstream's `duplicateAsCompanion` in `AppStore+Companions.swift`), which `crate::duplicate_replay` plans.

use std::path::Path;

use uuid::Uuid;

use super::SplitAxis;
use super::companions::CompanionRefusal;
use super::session::Session;
use super::store::{AppStore, NewSession};
use crate::duplicate_replay::rebind_arguments;
use crate::live::daemon_name;
use crate::pane_env::uuid_string;
use crate::restore::spawn_plan::ORDINARY_DIALECT;

/// One pane of a Duplicate as Companion copy: where it starts, and the creation command it ends with (`None` for a
/// plain shell) with the argv that command quotes ([`super::PaneLaunch::argv`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PaneCopy {
    pub cwd: String,
    pub command: Option<String>,
    pub argv: Option<Vec<String>>,
}

impl PaneCopy {
    /// The planner validated the argv before creating any row; substituting UUIDs and daemon names preserves its
    /// quotability. Recompute the stored line from that argv so both representations name the copy's identities.
    fn rebind(&mut self, ids: &[(String, String)]) {
        if let Some(argv) = &mut self.argv {
            rebind_arguments(argv, ids);
            self.command = ORDINARY_DIALECT.line(argv);
        }
    }
}

/// Every source identity that copied arguments may carry (upstream's `identityRebinding`).
fn identity_rebinding(source: &Session, copy: &Session) -> Vec<(String, String)> {
    let mut ids = vec![(uuid_string(source.id()), uuid_string(copy.id()))];
    let panes = std::iter::once((source.pane_identity, copy.pane_identity))
        .chain(source.split_pane_identity.zip(copy.split_pane_identity));
    for (from, to) in panes {
        ids.push((uuid_string(from), uuid_string(to)));
        ids.push((daemon_name(from), daemon_name(to)));
    }
    ids
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
    /// (#842). Session, pane and daemon identities in raw arguments are rebound to the copy before quoting and saving
    /// its commands (#843).
    ///
    /// # Errors
    ///
    /// As [`AppStore::add_companion`].
    pub fn duplicate_as_companion(
        &mut self,
        source: Uuid,
        mut copy: CompanionCopy,
        home: &str,
    ) -> Result<Uuid, CompanionRefusal> {
        let slot = self.companion_slot(source, Some(copy.left.cwd.clone()), home)?;
        let spec = NewSession {
            at: Some(slot.index),
            select: copy.select,
            parent: Some(slot.parent),
            ..NewSession::at_cwd(slot.cwd)
        };
        let id = self
            .add_session(slot.workspace, spec)
            .ok_or(CompanionRefusal::NoSuchSession)?;
        if let Some(right) = &copy.right {
            if let Some(session) = self.session_mut(id) {
                session.initial_split_cwd = Some(right.cwd.clone());
            }
            self.set_split_visibility(id, true, Some(copy.axis));
            self.set_split_focus(id, false);
            if let Some(ratio) = copy.ratio {
                self.apply_split_ratio(ratio, id);
            }
        }
        // Both destination pane identities now exist; surfaces spawn only after the model has finished this copy.
        let ids = self
            .session(source)
            .zip(self.session(id))
            .map(|(source, copy)| identity_rebinding(source, copy))
            .ok_or(CompanionRefusal::NoSuchSession)?;
        copy.left.rebind(&ids);
        if let Some(right) = &mut copy.right {
            right.rebind(&ids);
        }
        if let Some(session) = self.session_mut(id) {
            session.launch.command = copy.left.command;
            session.launch.argv = copy.left.argv;
            if let Some(right) = copy.right {
                session.split_launch.command = right.command;
                session.split_launch.argv = right.argv;
            }
        }
        self.save();
        Ok(id)
    }
}
