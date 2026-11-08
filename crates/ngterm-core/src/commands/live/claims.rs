//! Every pane expecting a live-session daemon (upstream's `WindowLibrary.paneClaims`), read without writing anything:
//! the open windows' sessions and the sessions their pending closes hold, then each closed window's file, then the
//! window files the index does not list. A pane with no identity to claim makes the walk incomplete rather than being
//! repaired here, since a read must not rewrite window files.

use std::collections::HashSet;

use uuid::Uuid;

use crate::Core;
use crate::live::inventory::{ClaimWalk, OwnerWindowState, PaneClaim, PaneRole};
use crate::model::Session;
use crate::persist::Snapshot;

/// Which window a claim came from.
struct Origin {
    id: Uuid,
    /// `None` for an unindexed window, whose name lives only in the index it is missing from.
    name: Option<String>,
    state: OwnerWindowState,
}

/// One session's place in the tree.
struct Site<'a> {
    origin: &'a Origin,
    workspace_id: Uuid,
    workspace_name: String,
    session_id: Uuid,
    session_name: String,
    /// A soft-closed session waiting out its grace: hidden from the tree, its daemons still owned.
    pending_close: bool,
}

impl Site<'_> {
    fn claim(&self, pane: PaneRole, identity: Uuid) -> PaneClaim {
        PaneClaim {
            pane_identity: identity,
            pane,
            pending_close: self.pending_close,
            window_id: self.origin.id,
            window_name: self.origin.name.clone(),
            window_state: self.origin.state,
            workspace_id: Some(self.workspace_id),
            workspace_name: Some(self.workspace_name.clone()),
            session_id: self.session_id,
            session_name: Some(self.session_name.clone()),
        }
    }
}

impl Core {
    /// Every pane claim of every window, and whether the walk could account for all of them.
    pub(crate) fn pane_claims(&self) -> ClaimWalk {
        let mut walk = ClaimWalk {
            claims: Vec::new(),
            complete: true,
        };
        let mut indexed = HashSet::new();
        for info in self.library().windows() {
            indexed.insert(info.id);
            let open = self.library().is_open(info.id);
            let origin = Origin {
                id: info.id,
                name: Some(info.name.clone()),
                state: if open {
                    OwnerWindowState::Open
                } else {
                    OwnerWindowState::Closed
                },
            };
            if open {
                self.live_claims(&origin, &mut walk);
            } else {
                self.persisted_claims(&origin, &mut walk);
            }
        }
        let Some(persistence) = self.persistence.as_ref() else {
            return walk;
        };
        // an unenumerable directory hides unindexed panes entirely, so the silence must not read as "none"
        let Some(strays) = persistence.stray_window_ids(&indexed) else {
            tracing::warn!("the live-session claim walk could not enumerate the windows directory");
            walk.complete = false;
            return walk;
        };
        for id in strays {
            let origin = Origin {
                id,
                name: None,
                state: OwnerWindowState::Unindexed,
            };
            self.persisted_claims(&origin, &mut walk);
        }
        walk
    }

    fn live_claims(&self, origin: &Origin, walk: &mut ClaimWalk) {
        let Some(store) = self.library().store(origin.id) else {
            return;
        };
        let site = |workspace_id, workspace_name: &str, session: &Session, pending_close| Site {
            origin,
            workspace_id,
            workspace_name: workspace_name.to_owned(),
            session_id: session.id(),
            session_name: session.display_name(),
            pending_close,
        };
        for workspace in store.workspaces() {
            for session in &workspace.sessions {
                claim_live(&site(workspace.id(), &workspace.name, session, false), session, walk);
            }
        }
        // a soft close removes the session from the tree for the grace while its terminals, and so its daemons, stay
        // alive; omitting them would report a claimed pane as an orphan
        for member in store.pending_close_members() {
            let site = site(member.workspace_id, member.workspace_name, member.session, true);
            claim_live(&site, member.session, walk);
        }
    }

    fn persisted_claims(&self, origin: &Origin, walk: &mut ClaimWalk) {
        let snapshot = self
            .persistence
            .as_ref()
            .and_then(|persistence| persistence.read_window_checked(origin.id).ok());
        let Some(snapshot) = snapshot else {
            tracing::warn!(window = %origin.id, "the live-session claim walk could not read the window");
            walk.complete = false;
            return;
        };
        claim_snapshot(origin, &snapshot, walk);
    }
}

/// A live session's claims, through its ownership projection: an empty list is a session whose daemons are not this
/// instance's to claim.
fn claim_live(site: &Site<'_>, session: &Session, walk: &mut ClaimWalk) {
    let mut owned = session.locally_managed_pane_identities().into_iter();
    let Some(primary) = owned.next() else {
        return;
    };
    walk.claims.push(site.claim(PaneRole::Left, primary));
    if !session.split.exists {
        return;
    }
    match owned.next() {
        Some(split) => walk.claims.push(site.claim(PaneRole::Right, split)),
        None => walk.complete = false,
    }
}

/// A closed or unindexed window's claims from its file: each pane is judged on its own, so a session missing its
/// primary identity can still own a good split.
fn claim_snapshot(origin: &Origin, snapshot: &Snapshot, walk: &mut ClaimWalk) {
    for workspace in &snapshot.workspaces {
        for session in &workspace.sessions {
            let site = Site {
                origin,
                workspace_id: workspace.id,
                workspace_name: workspace.name.clone(),
                session_id: session.id,
                session_name: persisted_name(session.custom_name.as_deref(), &session.cwd),
                pending_close: false,
            };
            match session.pane_identity {
                Some(identity) => walk.claims.push(site.claim(PaneRole::Left, identity)),
                None => walk.complete = false,
            }
            if !session.has_split.unwrap_or(false) && !session.is_split.unwrap_or(false) {
                continue;
            }
            match session.split_pane_identity {
                Some(split) => walk.claims.push(site.claim(PaneRole::Right, split)),
                None => walk.complete = false,
            }
        }
    }
}

/// A closed window's session label from what its file holds: no title is persisted, so a custom name else the cwd's
/// last component (`~` for none).
fn persisted_name(custom_name: Option<&str>, cwd: &str) -> String {
    if let Some(name) = custom_name.map(str::trim).filter(|name| !name.is_empty()) {
        return name.to_owned();
    }
    if cwd.is_empty() {
        return "~".to_owned();
    }
    let trimmed = cwd.trim_end_matches('/');
    if trimmed.is_empty() {
        return "/".to_owned();
    }
    trimmed.rsplit('/').next().unwrap_or(trimmed).to_owned()
}

#[cfg(test)]
mod tests {
    use super::persisted_name;

    #[test]
    fn a_persisted_session_reads_its_custom_name_else_its_directory() {
        assert_eq!(persisted_name(Some("  build  "), "/tmp/x"), "build");
        assert_eq!(persisted_name(Some("   "), "/tmp/x"), "x");
        assert_eq!(persisted_name(None, "/srv/app/"), "app");
        assert_eq!(persisted_name(None, "/"), "/");
        assert_eq!(persisted_name(None, ""), "~");
    }

    fn core(directory: &std::path::Path) -> crate::Core {
        crate::Core::new(
            crate::CoreConfig {
                app: ngterm_proto::AppIdentity::from_recorded("0.1.0", None),
                capabilities: Vec::new(),
            },
            crate::CoreState {
                library: crate::model::WindowLibrary::new("/home/u"),
                settings: crate::persist::AppSettings::default(),
                persistence: Some(crate::persist::LibraryPersistence::new(directory)),
            },
        )
    }

    fn snapshot(primary: Option<uuid::Uuid>, split: Option<uuid::Uuid>, legacy: bool) -> crate::persist::Snapshot {
        crate::persist::Snapshot {
            workspaces: vec![crate::persist::WorkspaceSnapshot {
                id: uuid::Uuid::new_v4(),
                name: "work".to_owned(),
                collapsed: None,
                tint: None,
                sessions: vec![crate::persist::SessionSnapshot {
                    id: uuid::Uuid::new_v4(),
                    cwd: "/srv/project".to_owned(),
                    pane_identity: primary,
                    split_pane_identity: split,
                    has_split: (!legacy).then_some(true),
                    is_split: legacy.then_some(true),
                    ..crate::persist::SessionSnapshot::default()
                }],
            }],
            ..crate::persist::Snapshot::default()
        }
    }

    #[test]
    fn unindexed_files_claim_each_good_pane_without_rewriting_missing_identities() {
        use crate::live::inventory::{OwnerWindowState, PaneRole};
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        let persistence = core.persistence.as_ref().unwrap();
        std::fs::create_dir_all(persistence.directory().join("windows")).unwrap();
        let primary = uuid::Uuid::new_v4();
        let split = uuid::Uuid::new_v4();
        for (left, right, legacy) in [(Some(primary), None, false), (None, Some(split), true)] {
            let id = uuid::Uuid::new_v4();
            let path = persistence.window_path(id);
            let bytes = serde_json::to_vec(&snapshot(left, right, legacy)).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            let walk = core.pane_claims();
            assert!(!walk.complete);
            assert_eq!(walk.claims.len(), 1);
            let claim = walk.claims.iter().find(|claim| claim.window_id == id).unwrap();
            assert_eq!(claim.window_state, OwnerWindowState::Unindexed);
            assert_eq!(claim.window_name, None);
            assert_eq!(claim.workspace_name.as_deref(), Some("work"));
            assert_eq!(claim.session_name.as_deref(), Some("project"));
            assert!(!claim.pending_close);
            assert_eq!(claim.pane_identity, left.or(right).unwrap());
            assert_eq!(
                claim.pane,
                if left.is_some() {
                    PaneRole::Left
                } else {
                    PaneRole::Right
                }
            );
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            std::fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn a_closed_window_reads_its_saved_claims_and_an_unreadable_file_is_incomplete() {
        use crate::live::inventory::OwnerWindowState;
        let dir = tempfile::tempdir().unwrap();
        let mut core = core(dir.path());
        let id = core.library_mut().new_window(Some("closed"));
        core.library_mut().close_window(id);
        assert!(!core.pane_claims().complete);
        let persistence = core.persistence.as_ref().unwrap();
        std::fs::create_dir_all(persistence.directory().join("windows")).unwrap();
        let primary = uuid::Uuid::new_v4();
        let split = uuid::Uuid::new_v4();
        std::fs::write(
            persistence.window_path(id),
            serde_json::to_vec(&snapshot(Some(primary), Some(split), false)).unwrap(),
        )
        .unwrap();
        let walk = core.pane_claims();
        assert!(walk.complete);
        assert_eq!(walk.claims.len(), 2);
        for claim in &walk.claims {
            assert_eq!(claim.window_id, id);
            assert_eq!(claim.window_name.as_deref(), Some("closed"));
            assert_eq!(claim.window_state, OwnerWindowState::Closed);
        }
        assert_eq!(
            walk.claims.iter().map(|claim| claim.pane_identity).collect::<Vec<_>>(),
            [primary, split]
        );
    }

    #[test]
    fn an_unenumerable_windows_directory_makes_the_claim_walk_incomplete() {
        let dir = tempfile::tempdir().unwrap();
        let core = core(dir.path());
        assert!(core.pane_claims().complete);
        let windows = core.persistence.as_ref().unwrap().directory().join("windows");
        std::fs::write(windows, "not a directory").unwrap();
        assert!(!core.pane_claims().complete);
    }
}
