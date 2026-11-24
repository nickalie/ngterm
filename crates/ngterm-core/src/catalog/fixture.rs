//! Upstream's `CatalogFixture`: catalogs to diff, stream and encode.

use ngterm_proto::{CatalogVersions, ControlLiveEndpoint};
use uuid::Uuid;

use super::frames::{
    CatalogPage, CatalogPane, CatalogSession, CatalogSessionFields, CatalogSnapshot, CatalogWorkspace, Lifecycle,
};

pub(crate) fn endpoint() -> ControlLiveEndpoint {
    ControlLiveEndpoint {
        attach_bridge: None,
        executable: "/srv/ngterm-session".to_owned(),
        socket_directory: "/run/live".to_owned(),
        peer: None,
    }
}

pub(crate) fn versions() -> CatalogVersions {
    CatalogVersions {
        server: "1.2.3".to_owned(),
        protocol: 1,
        catalog: 1,
    }
}

pub(crate) const CATALOG: Uuid = Uuid::from_u128(0xC0FF_EE00_0000_4000_8000_0000_0000_0001);

pub(crate) fn pane_with(identity: Uuid, daemon: Option<&str>) -> CatalogPane {
    CatalogPane {
        identity,
        daemon: daemon.map_or_else(|| crate::live::daemon_name(identity), str::to_owned),
        endpoint: endpoint(),
    }
}

pub(crate) fn pane() -> CatalogPane {
    pane_with(Uuid::new_v4(), None)
}

pub(crate) fn fields(id: Uuid, name: &str, primary: Uuid, split: bool) -> CatalogSessionFields {
    CatalogSessionFields {
        id,
        name: name.to_owned(),
        parent: None,
        cwd: "/home".to_owned(),
        primary,
        axis: split.then(|| "vertical".to_owned()),
        shown: split,
        lifecycle: Lifecycle::Live,
        scratch: None,
        page: None,
        custom_name: None,
    }
}

pub(crate) fn session_with(id: Uuid, name: &str, panes: Vec<CatalogPane>) -> CatalogSession {
    let primary = panes.first().map_or_else(Uuid::new_v4, |pane| pane.identity);
    let split = panes.len() > 1;
    CatalogSession::new(fields(id, name, primary, split), panes)
}

pub(crate) fn session(name: &str) -> CatalogSession {
    session_with(Uuid::new_v4(), name, vec![pane()])
}

pub(crate) fn companion(name: &str, parent: Uuid) -> CatalogSession {
    let mut session = session(name);
    session.fields.parent = Some(parent);
    session
}

pub(crate) fn page_row() -> CatalogSession {
    let id = Uuid::new_v4();
    let mut fields = fields(id, "page", id, false);
    fields.page = Some(CatalogPage {
        kind: "url".to_owned(),
        identity: "example.com".to_owned(),
        endpoint: endpoint(),
    });
    CatalogSession::new(fields, Vec::new())
}

pub(crate) fn workspace(id: Uuid, name: &str, sessions: Vec<CatalogSession>) -> CatalogWorkspace {
    CatalogWorkspace {
        id,
        name: name.to_owned(),
        sessions,
    }
}

pub(crate) fn snapshot(workspaces: Vec<CatalogWorkspace>) -> CatalogSnapshot {
    CatalogSnapshot {
        epoch: "e1".to_owned(),
        rev: 0,
        catalog_id: CATALOG,
        versions: versions(),
        workspaces,
    }
}

type Transition = (&'static str, Vec<CatalogWorkspace>, Vec<CatalogWorkspace>);

/// The sessions and workspaces the transitions move around.
struct Cast {
    a: CatalogSession,
    b: CatalogSession,
    c: CatalogSession,
    left: CatalogPane,
    split: CatalogSession,
    swapped: CatalogSession,
    renamed: CatalogSession,
    ended: CatalogSession,
    one_pane: CatalogSession,
    w1: Uuid,
    w2: Uuid,
}

fn cast() -> Cast {
    let a = session("a");
    let (left, right) = (pane(), pane());
    let split = session_with(Uuid::new_v4(), "s", vec![left.clone(), right.clone()]);
    let mut swapped = split.clone();
    swapped.fields.primary = right.identity;
    swapped.normalize();
    let mut renamed = a.clone();
    renamed.fields.name = "renamed".to_owned();
    renamed.fields.cwd = "/tmp".to_owned();
    let mut ended = a.clone();
    ended.fields.lifecycle = Lifecycle::Ended;
    ended.panes.clear();
    Cast {
        one_pane: session_with(split.id(), "s", vec![left.clone()]),
        a,
        b: session("b"),
        c: session("c"),
        left,
        split,
        swapped,
        renamed,
        ended,
        w1: Uuid::new_v4(),
        w2: Uuid::new_v4(),
    }
}

fn ws(id: Uuid, name: &str, sessions: &[&CatalogSession]) -> CatalogWorkspace {
    workspace(id, name, sessions.iter().map(|&session| session.clone()).collect())
}

fn workspace_transitions(cast: &Cast) -> Vec<Transition> {
    let Cast { a, b, c, w1, w2, .. } = cast;
    let (w1, w2) = (*w1, *w2);
    vec![
        ("first workspace", vec![], vec![ws(w1, "w1", &[a])]),
        ("workspace renamed", vec![ws(w1, "w1", &[])], vec![ws(w1, "dev", &[])]),
        (
            "workspace removed with its sessions",
            vec![ws(w1, "w1", &[a]), ws(w2, "w2", &[])],
            vec![ws(w2, "w2", &[])],
        ),
        (
            "workspaces reordered",
            vec![ws(w1, "w1", &[]), ws(w2, "w2", &[])],
            vec![ws(w2, "w2", &[]), ws(w1, "w1", &[])],
        ),
        (
            "session added in the middle",
            vec![ws(w1, "w1", &[a, c])],
            vec![ws(w1, "w1", &[a, b, c])],
        ),
        (
            "sessions reordered",
            vec![ws(w1, "w1", &[a, b, c])],
            vec![ws(w1, "w1", &[c, a, b])],
        ),
        (
            "session moved to another workspace",
            vec![ws(w1, "w1", &[a, b]), ws(w2, "w2", &[c])],
            vec![ws(w1, "w1", &[b]), ws(w2, "w2", &[a, c])],
        ),
        (
            "session moved into a new workspace",
            vec![ws(w1, "w1", &[a, b])],
            vec![ws(w1, "w1", &[b]), ws(w2, "w2", &[a])],
        ),
    ]
}

fn session_transitions(cast: &Cast) -> Vec<Transition> {
    let Cast {
        a,
        b,
        c,
        left,
        split,
        swapped,
        renamed,
        ended,
        one_pane,
        w1,
        w2,
    } = cast;
    let (w1, w2) = (*w1, *w2);
    let daemon = |name| session_with(a.id(), "a", vec![pane_with(left.identity, Some(name))]);
    vec![
        (
            "session renamed and its cwd changed",
            vec![ws(w1, "w1", &[a])],
            vec![ws(w1, "w1", &[renamed])],
        ),
        (
            "split added",
            vec![ws(w1, "w1", &[one_pane])],
            vec![ws(w1, "w1", &[split])],
        ),
        (
            "split closed",
            vec![ws(w1, "w1", &[split])],
            vec![ws(w1, "w1", &[one_pane])],
        ),
        (
            "panes swapped",
            vec![ws(w1, "w1", &[split])],
            vec![ws(w1, "w1", &[swapped])],
        ),
        ("session ended", vec![ws(w1, "w1", &[a])], vec![ws(w1, "w1", &[ended])]),
        (
            "pane daemon changed",
            vec![ws(w1, "w1", &[&daemon("old")])],
            vec![ws(w1, "w1", &[&daemon("new")])],
        ),
        (
            "everything at once",
            vec![ws(w1, "w1", &[a, b, split]), ws(w2, "w2", &[c])],
            vec![
                ws(w2, "x", &[swapped, c, a]),
                ws(Uuid::new_v4(), "new", &[&session("s")]),
            ],
        ),
    ]
}

/// Upstream's `CatalogFixture.transitions`.
pub(crate) fn transitions() -> Vec<Transition> {
    let cast = cast();
    let mut all = workspace_transitions(&cast);
    all.extend(session_transitions(&cast));
    all
}
