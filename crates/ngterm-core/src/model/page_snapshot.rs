//! A session's pages as its snapshot writes them and as a restore puts them back (an extension beyond upstream, whose
//! overlays are ephemeral). A page lives until it is closed, not until the app showing it quits: each page this machine
//! opened in an overlay slot ([`HtmlOverlay::persisted`]) is written with its source, flags and slot, and a restore
//! opens it again in that slot under a fresh id, its outcome pending (the core's page sync registers it).
//!
//! A restore checks every entry again, as `session.overlay.open` checked it: a file page's grant against the home, a
//! web address, an absolute image or Markdown path of a kind it shows, and the flags the source may take. An entry
//! whose file is gone, whose slot is taken or whose split pane no longer exists is dropped, so no page outlives what
//! it showed.

use std::path::Path;

use uuid::Uuid;

use super::overlay::OverlayPane;
use super::overlay_points::{OverlayPointSize, OverlaySize};
use super::session::Session;
use super::status::is_valid_color_hex;
use crate::html::{self, HtmlOverlay, HtmlPageFlags, HtmlSource};
use crate::link_policy::{IMAGE_EXTENSIONS, MARKDOWN_EXTENSIONS};
use crate::persist::{PageSlotSnapshot, PageSnapshot, PageSourceSnapshot};

/// The pages `session`'s snapshot writes, session-wide first, then left and right; `None` while it has none.
pub(super) fn session_pages(session: &Session) -> Option<Vec<PageSnapshot>> {
    let pages: Vec<PageSnapshot> = persisted_pages(session).into_iter().map(|(_, page)| page).collect();
    (!pages.is_empty()).then_some(pages)
}

/// [`session_pages`] by page id, in the same order: what the core's page sync compares to tell a persisted page that
/// changed what its snapshot writes.
pub(super) fn persisted_pages(session: &Session) -> Vec<(Uuid, PageSnapshot)> {
    session
        .html_overlays()
        .into_iter()
        .filter(|(_, page)| page.persisted())
        .filter_map(|(pane, page)| Some((page.id, page_snapshot(session, pane, page)?)))
        .collect()
}

/// One page as persisted, with what its slot holds beside it: the size only in the session-wide slot.
fn page_snapshot(session: &Session, pane: Option<OverlayPane>, shown: &HtmlOverlay) -> Option<PageSnapshot> {
    let source = match &shown.source {
        HtmlSource::File { path, grant_root } => PageSourceSnapshot::File {
            path: path.clone(),
            grant_root: grant_root.clone(),
        },
        HtmlSource::Url(url) => PageSourceSnapshot::Url {
            url: url.as_str().to_owned(),
        },
        HtmlSource::Image { path } => PageSourceSnapshot::Image { path: path.clone() },
        HtmlSource::Markdown { path } => PageSourceSnapshot::Markdown { path: path.clone() },
        HtmlSource::Replica { .. } => return None,
    };
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
            let slot = match pane {
                OverlayPane::Left => PageSlotSnapshot::Left,
                OverlayPane::Right => PageSlotSnapshot::Right,
            };
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
    })
}

/// Opens `pages` again in `session`'s slots, each under a fresh id and, in a pane slot, a fresh generation; the entries
/// that no longer check out are dropped (the module docs). A pane page holds the restored session-wide page only when
/// its entry says it held the one written beside it, so the two keep the order they were drawn in. `home` is what a
/// file page's grant is checked against.
pub(super) fn restore_pages(session: &mut Session, pages: &[PageSnapshot], home: &str) {
    for snapshot in pages {
        let pane = match snapshot.slot {
            PageSlotSnapshot::Session => None,
            PageSlotSnapshot::Left => Some(OverlayPane::Left),
            PageSlotSnapshot::Right => Some(OverlayPane::Right),
        };
        let free = match pane {
            None => !session.overlay.active,
            Some(OverlayPane::Right) if !session.split.exists => false,
            Some(pane) => session.pane_overlay(pane).is_none(),
        };
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
