//! A session's pages as its snapshot writes them and as a restore puts them back (an extension beyond upstream, whose
//! overlays are ephemeral). A page lives until it is closed, not until the app showing it quits: each page this machine
//! opened in an overlay slot ([`HtmlOverlay::persisted`]) is written with its source, flags and slot, and a restore
//! opens it again in that slot under a fresh id, its outcome pending (the core's page sync registers it).
//!
//! A restore checks every entry again, as `session.overlay.open` checked it: a file page's grant against the home, a
//! web address, an absolute image, Markdown or text path of a kind it shows, and the flags the source may take. An entry
//! whose file is gone, whose slot is taken or whose split pane no longer exists is dropped, so no page outlives what
//! it showed.
//!
//! A page handed to the viewer presenting the session, shown there or parked waiting for one ([`RemotePage`]), is
//! written too, marked `viewer`; a restore parks it again, its slot reserved for the next presenter rather than opened
//! here. A presenter-only origin (headless), which has no page surface of its own, parks every page a restore opened
//! locally the same way and closes the ones no presenter can show ([`park_local_pages`]).

use std::path::Path;
use std::time::Instant;

use uuid::Uuid;

use super::overlay::OverlayPane;
use super::overlay_points::{OverlayPointSize, OverlaySize};
use super::remote_overlay::{RemoteOverlaySlot, RemotePage};
use super::session::Session;
use super::status::is_valid_color_hex;
use crate::html::{self, HtmlOverlay, HtmlPageFlags, HtmlSource};
use crate::link_policy::{IMAGE_EXTENSIONS, MARKDOWN_EXTENSIONS};
use crate::pane_env::uuid_string;
use crate::persist::{PageSlotSnapshot, PageSnapshot, PageSourceSnapshot};

/// The pages `session`'s snapshot writes, session-wide first, then left and right, then the viewer's in the order their
/// slots were reserved, the order a restore reserves them again and the next presenter is handed them, so the one
/// opened last stays on top; `None` while it has none.
pub(super) fn session_pages(session: &Session) -> Option<Vec<PageSnapshot>> {
    let pages: Vec<PageSnapshot> = persisted_pages(session).into_iter().map(|(_, page)| page).collect();
    (!pages.is_empty()).then_some(pages)
}

/// [`session_pages`] by page id, in the same order: what the core's page sync compares to tell a persisted page that
/// changed what its snapshot writes.
pub(super) fn persisted_pages(session: &Session) -> Vec<(Uuid, PageSnapshot)> {
    let local = session
        .html_overlays()
        .into_iter()
        .filter(|(_, page)| page.persisted())
        .filter_map(|(pane, page)| Some((page.id, page_snapshot(session, pane, page)?)));
    let viewer = session.remote_overlays.slots().iter().filter_map(|slot| {
        let page = slot.page.as_ref()?;
        Some((page.overlay.id, viewer_page_snapshot(slot, page)?))
    });
    local.chain(viewer).collect()
}

/// What a page's source is written as; a replica is never written.
fn source_snapshot(source: &HtmlSource) -> Option<PageSourceSnapshot> {
    Some(match source {
        HtmlSource::File { path, grant_root } => PageSourceSnapshot::File {
            path: path.clone(),
            grant_root: grant_root.clone(),
        },
        HtmlSource::Url(url) => PageSourceSnapshot::Url {
            url: url.as_str().to_owned(),
        },
        HtmlSource::Image { path } => PageSourceSnapshot::Image { path: path.clone() },
        HtmlSource::Markdown { path } => PageSourceSnapshot::Markdown { path: path.clone() },
        HtmlSource::Text { path, line } => PageSourceSnapshot::Text {
            path: path.clone(),
            line: *line,
        },
        HtmlSource::Document { path } => PageSourceSnapshot::Document { path: path.clone() },
        HtmlSource::Replica { .. } => return None,
    })
}

fn slot_snapshot(pane: Option<OverlayPane>) -> PageSlotSnapshot {
    match pane {
        None => PageSlotSnapshot::Session,
        Some(OverlayPane::Left) => PageSlotSnapshot::Left,
        Some(OverlayPane::Right) => PageSlotSnapshot::Right,
    }
}

/// A page the session's viewer holds, at the size its slot keeps and the visibility it was last asked for.
fn viewer_page_snapshot(slot: &RemoteOverlaySlot, page: &RemotePage) -> Option<PageSnapshot> {
    if !page.overlay.persisted() {
        return None;
    }
    let flags = page.overlay.flags;
    Some(PageSnapshot {
        slot: slot_snapshot(slot.pane),
        source: source_snapshot(&page.overlay.source)?,
        navigation: flags.navigation,
        javascript: flags.javascript,
        chromeless: flags.chromeless,
        persistent: flags.persistent,
        browse: flags.browse,
        size_percent: slot.size_percent,
        width: slot.points.width,
        height: slot.points.height,
        background_color: page.background_color.clone(),
        hidden: page.effective_hidden(),
        holds_cover: false,
        viewer: true,
    })
}

/// One page as persisted, with what its slot holds beside it: the size only in the session-wide slot.
fn page_snapshot(session: &Session, pane: Option<OverlayPane>, shown: &HtmlOverlay) -> Option<PageSnapshot> {
    let source = source_snapshot(&shown.source)?;
    let (slot, size_percent, points, background_color, hidden, holds_cover) = match pane {
        None => {
            let overlay = &session.overlay;
            (
                PageSlotSnapshot::Session,
                overlay.size_percent,
                overlay.points,
                overlay.background_color.clone(),
                overlay.hidden,
                false,
            )
        }
        Some(pane) => {
            let overlay = session.pane_overlay(pane)?;
            let slot = slot_snapshot(Some(pane));
            // the hold is written only over a session-wide page the snapshot writes beside it, the one cover a
            // restore brings back
            let cover_written = session.overlay.html.as_ref().is_some_and(HtmlOverlay::persisted);
            let holds_cover =
                cover_written && overlay.holds_cover.is_some() && overlay.holds_cover == session.cover_hold();
            (
                slot,
                None,
                OverlayPointSize::default(),
                overlay.background_color.clone(),
                overlay.hidden,
                holds_cover,
            )
        }
    };
    let flags = shown.flags;
    Some(PageSnapshot {
        slot,
        source,
        navigation: flags.navigation,
        javascript: flags.javascript,
        chromeless: flags.chromeless,
        persistent: flags.persistent,
        browse: flags.browse,
        size_percent,
        width: points.width,
        height: points.height,
        background_color,
        hidden,
        holds_cover,
        viewer: false,
    })
}

/// Opens `pages` again in `session`'s slots, each under a fresh id and, in a pane slot, a fresh generation; the entries
/// that no longer check out are dropped (the module docs). A pane page holds the restored session-wide page only when
/// its entry says it held the one written beside it, so the two keep the order they were drawn in. A `viewer` entry is
/// parked in its slot instead, for the next presenter. `home` is what a file page's grant is checked against.
pub(super) fn restore_pages(session: &mut Session, pages: &[PageSnapshot], home: &str) {
    for snapshot in pages {
        let pane = match snapshot.slot {
            PageSlotSnapshot::Session => None,
            PageSlotSnapshot::Left => Some(OverlayPane::Left),
            PageSlotSnapshot::Right => Some(OverlayPane::Right),
        };
        let free_here = match pane {
            None => !session.overlay.active,
            Some(OverlayPane::Right) if !session.split.exists => false,
            Some(pane) => session.pane_overlay(pane).is_none(),
        };
        let free = free_here && session.remote_overlays.slot(pane).is_none();
        let flags = HtmlPageFlags {
            navigation: snapshot.navigation,
            javascript: snapshot.javascript,
            chromeless: snapshot.chromeless,
            persistent: snapshot.persistent,
            browse: snapshot.browse,
        };
        let Some(source) = restored_source(&snapshot.source, flags, home).filter(|_| free) else {
            continue;
        };
        let size = OverlaySize {
            percent: snapshot.size_percent,
            points: OverlayPointSize {
                width: snapshot.width.filter(|points| *points > 0),
                height: snapshot.height.filter(|points| *points > 0),
            },
        };
        let background_color = snapshot
            .background_color
            .clone()
            .filter(|color| is_valid_color_hex(color));
        if snapshot.viewer {
            let parked = ParkedPage {
                pane,
                size,
                background_color,
                hidden: snapshot.hidden,
            };
            park_page(session, HtmlOverlay::new(source, flags), parked);
            continue;
        }
        session.install_html_overlay(pane, HtmlOverlay::new(source, flags), size, background_color);
        match pane {
            None => session.overlay.hidden = snapshot.hidden,
            Some(pane) => {
                let holds_cover = session.cover_hold().filter(|_| snapshot.holds_cover);
                if let Some(overlay) = session.pane_overlays.slot_mut(pane).overlay.as_mut() {
                    overlay.hidden = snapshot.hidden;
                    overlay.holds_cover = holds_cover;
                }
            }
        }
    }
}

/// Where and how a parked page waits for its presenter.
struct ParkedPage {
    pane: Option<OverlayPane>,
    size: OverlaySize,
    background_color: Option<String>,
    hidden: bool,
}

/// Reserves `parked.pane`'s slot of `session` for `overlay`, parked: no presenter shows it until one that speaks pages
/// is handed it. A pane page is full; point axes only apply beside a session-wide page's share.
fn park_page(session: &mut Session, overlay: HtmlOverlay, parked: ParkedPage) {
    let percent = parked
        .size
        .percent
        .filter(|_| parked.pane.is_none())
        .map(|percent| percent.clamp(1, 100));
    let points = if percent.is_some() {
        parked.size.points
    } else {
        OverlayPointSize::default()
    };
    let job = uuid_string(overlay.id);
    let mut page = RemotePage::new(overlay, uuid_string(Uuid::new_v4()), Instant::now());
    page.background_color = parked.background_color;
    page.hidden = parked.hidden;
    page.parked = true;
    session.remote_overlays.reserve(RemoteOverlaySlot {
        job,
        pane: parked.pane,
        owner: 0,
        size_percent: percent,
        points,
        wait: false,
        ended: false,
        page: Some(page),
    });
}

/// What [`park_local_pages`] did with the pages a restore opened in a session's own slots.
#[derive(Debug, Default)]
pub(crate) struct LocalPagesSwept {
    /// The pages parked for the next presenter.
    pub(crate) parked: Vec<Uuid>,
    /// Whether a page no presenter can show (an image, Markdown or text page) was closed.
    pub(crate) dropped: bool,
}

/// Parks every page open in `session`'s own slots that a presenter can show (a file or URL page), in the slot it held
/// with its size, background and visibility: what a presenter-only origin, which renders no page itself, does with
/// the pages a restore opened. They are parked in the order they were drawn, a pane page under the session-wide one
/// before it and one holding it after, so the next presenter stacks them the same way. A page no presenter can show is
/// closed, as nothing here would ever show it and it would hold its slot.
pub(crate) fn park_local_pages(session: &mut Session) -> LocalPagesSwept {
    let mut swept = LocalPagesSwept::default();
    let mut local: Vec<(Option<OverlayPane>, PageSnapshot, HtmlOverlay)> = Vec::new();
    let mut unpresentable: Vec<Option<OverlayPane>> = Vec::new();
    for (pane, page) in session.html_overlays() {
        if !page.persisted() {
            continue;
        }
        if crate::remote::presentation_source(&page.source).is_none() {
            unpresentable.push(pane);
        } else if let Some(snapshot) = page_snapshot(session, pane, page) {
            local.push((pane, snapshot, page.clone()));
        }
    }
    for pane in unpresentable {
        close_local_page(session, pane);
        swept.dropped = true;
    }
    // drawing order: a pane page the cover is drawn over, the cover, a pane page drawn over the cover
    local.sort_by_key(|(pane, snapshot, _)| match pane {
        Some(_) if !snapshot.holds_cover => 0,
        None => 1,
        Some(_) => 2,
    });
    for (pane, snapshot, overlay) in local {
        close_local_page(session, pane);
        swept.parked.push(overlay.id);
        let size = OverlaySize {
            percent: snapshot.size_percent,
            points: OverlayPointSize {
                width: snapshot.width,
                height: snapshot.height,
            },
        };
        let waiting = ParkedPage {
            pane,
            size,
            background_color: snapshot.background_color,
            hidden: snapshot.hidden,
        };
        park_page(session, overlay, waiting);
    }
    swept
}

fn close_local_page(session: &mut Session, pane: Option<OverlayPane>) {
    match pane {
        None => {
            session.overlay.close();
        }
        Some(pane) => {
            session.pane_overlays.slot_mut(pane).close();
        }
    }
}

/// The source a persisted page shows, when it still checks out with `flags` as `session.overlay.open` would check it
/// now, and the file it names is still there.
fn restored_source(source: &PageSourceSnapshot, flags: HtmlPageFlags, home: &str) -> Option<HtmlSource> {
    let document = flags == HtmlPageFlags::default();
    match source {
        PageSourceSnapshot::File { path, grant_root } => {
            let fits = !flags.persistent && !flags.browse && (!flags.chromeless || !flags.navigation);
            let granted = html::grant_error(path, grant_root.as_deref(), home).is_none();
            (fits && granted && Path::new(path).is_file()).then(|| HtmlSource::File {
                path: path.clone(),
                grant_root: grant_root.clone(),
            })
        }
        PageSourceSnapshot::Url { url } => {
            let fits = !flags.chromeless || (html::is_loopback(url) && !flags.navigation && !flags.browse);
            html::web_url(url).filter(|_| fits).map(HtmlSource::Url)
        }
        PageSourceSnapshot::Image { path } => {
            (document && shown_document(path, &IMAGE_EXTENSIONS)).then(|| HtmlSource::Image { path: path.clone() })
        }
        PageSourceSnapshot::Markdown { path } => (document && shown_document(path, &MARKDOWN_EXTENSIONS))
            .then(|| HtmlSource::Markdown { path: path.clone() }),
        PageSourceSnapshot::Text { path, line } => (document
            && html::is_absolute(path)
            && html::text_page::is_eligible(path)
            && line.is_none_or(|line| line >= 1)
            && Path::new(path).is_file())
        .then(|| HtmlSource::Text {
            path: path.clone(),
            line: *line,
        }),
        PageSourceSnapshot::Document { path } => (document && shown_document(path, &html::document_page::EXTENSIONS))
            .then(|| HtmlSource::Document { path: path.clone() }),
    }
}

/// Whether `path` is an absolute, existing file of a kind in `extensions`, as the image and Markdown pages show.
fn shown_document(path: &str, extensions: &[&str]) -> bool {
    let extension = Path::new(path)
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    html::is_absolute(path) && extensions.contains(&extension.as_str()) && Path::new(path).is_file()
}
