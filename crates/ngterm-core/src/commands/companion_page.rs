//! Page companions: `session.companion.page`, `session.companion.page.attach`, and the refusal of the terminal
//! commands a page companion cannot take. Validation and its messages follow upstream's `dispatchCompanionPage` in
//! `ControlDispatcher+Companions.swift`; the effects follow `createPageCompanion` and `pageCompanionRefusal` in
//! `agterm/Control/ControlServer+Companions.swift` and `attachCompanionPage` in `ControlServer+CompanionPage.swift`.
//!
//! A page companion is a row whose page is its session-wide overlay, full size, with no shell under it: never
//! persisted, never archived, closing with its page. A remote parent answers upstream's local-path refusal until remote
//! companions are ported; the loopback refusal ahead of it is already upstream's. The web views are the frontend's
//! ([`crate::html::HtmlHost`]): a build or a host without them refuses the page as `session.overlay.open` does.

use super::overlay::{OverlayContent, overlay_content};
use super::overlay_page::PageOpening;
use super::{Outcome, session_store, with_id};
use crate::Core;
use crate::error::ControlError;
use crate::html::{self, HtmlOverlay, HtmlSource};
use crate::model::{
    CompanionRefusal, OverlayPointSize, PageCompanionOptions, Session, SessionContent, is_valid_color_hex,
};
use crate::pane_env::uuid_string;
use crate::resolve::{ACTIVE, Resolver};
use ngterm_proto::command::{AskCmd, SessionCmd, SurfaceCmd};
use ngterm_proto::{Args, Command, ControlResult, Request};

/// Upstream's refusals of `session.companion.page`.
pub(crate) const NEEDS_PAGE: &str = "a page companion needs --html or --url";
pub(crate) const FULL_SIZE: &str =
    "a page companion is full size: it takes no --pane, --size-percent, --width or --height";
const NOT_CREATED: &str = "could not create page companion";
const REMOTE_PARENT: &str = "companions of a remote session are not supported yet";
const REMOTE_LOOPBACK: &str = "a loopback page cannot be a companion of a remote session";
/// The page refusals of a build or host without web views, under this command's name.
const PAGES_UNAVAILABLE: &str = "session.companion.page: html overlays are not available in this build";
const PAGES_UNAVAILABLE_HERE: &str = "session.companion.page: html overlays are not available here";

/// The session commands a page companion refuses (upstream's `pageRefusedCommands`): they act on a terminal, or would
/// cover or float the page.
const PAGE_REFUSED: [SessionCmd; 18] = [
    SessionCmd::Type,
    SessionCmd::Text,
    SessionCmd::Copy,
    SessionCmd::Paste,
    SessionCmd::SelectAll,
    SessionCmd::Search,
    SessionCmd::Background,
    SessionCmd::Lead,
    SessionCmd::Swap,
    SessionCmd::Focus,
    SessionCmd::Resize,
    SessionCmd::Split,
    SessionCmd::SplitClose,
    SessionCmd::Scratch,
    SessionCmd::Restart,
    SessionCmd::Restore,
    SessionCmd::OverlayOpen,
    SessionCmd::HudOpen,
];

/// `session.companion.page` (upstream's `dispatchCompanionPage`): the page's own checks as `session.overlay.open`'s, a
/// program refused, then anything but full size and a malformed color, before the target resolves.
pub(crate) fn page(core: &mut Core, target: Option<&str>, args: &Args) -> Outcome {
    let command = args.command.as_deref().unwrap_or_default();
    let source = match overlay_content(command, args, core.library().home())? {
        OverlayContent::Program => return Err(ControlError::rejected(NEEDS_PAGE)),
        OverlayContent::Page(source) => source,
    };
    if args.pane.is_some() || args.size_percent.is_some() || args.width.is_some() || args.height.is_some() {
        return Err(ControlError::rejected(FULL_SIZE));
    }
    if let Some(color) = args.color.as_deref()
        && !is_valid_color_hex(color)
    {
        return Err(ControlError::rejected(format!("invalid color: {color} (#rrggbb)")));
    }
    create(core, target, args, source)
}

/// Creates the page companion (upstream's `createPageCompanion`): the row lands after the parent's group showing the
/// page full size in its own slot, the parent's slot untouched; `--follow` selects it. `{id, pageID, parent}`.
fn create(core: &mut Core, target: Option<&str>, args: &Args, source: HtmlSource) -> Outcome {
    let window = args.window.as_deref();
    let opening = PageOpening::from_args(source, args, None, OverlayPointSize::default());
    let remote = {
        let (store, id) = session_store(core, target, window)?;
        store.session(id).is_some_and(|session| session.remote_host().is_some())
    };
    if remote {
        // the page would load the origin's loopback there, where nothing answers for it
        if let HtmlSource::Url(url) = &opening.source
            && html::is_loopback(url.as_str())
        {
            return Err(ControlError::rejected(REMOTE_LOOPBACK));
        }
        return Err(ControlError::rejected(REMOTE_PARENT));
    }
    let host = core
        .html_host()
        .ok_or_else(|| ControlError::rejected(PAGES_UNAVAILABLE))?;
    if let Some(reason) = host.web_views_unavailable() {
        return Err(ControlError::rejected(format!("{PAGES_UNAVAILABLE_HERE}: {reason}")));
    }
    if opening.flags.persistent
        && let Some(failure) = host.persistent_store_failure()
    {
        return Err(ControlError::rejected(format!("session.companion.page: {failure}")));
    }
    let overlay = HtmlOverlay::new(opening.source, opening.flags);
    let page_id = overlay.id;
    let home = core.library().home().to_owned();
    let options = PageCompanionOptions {
        name: None,
        select: opening.follow,
        background_color: opening.background_color,
    };
    let (store, id) = session_store(core, target, window)?;
    let companion = store
        .add_page_companion(id, overlay, options, &home)
        .map_err(|refusal| match refusal {
            CompanionRefusal::RemoteParent => ControlError::rejected(REMOTE_PARENT),
            CompanionRefusal::NoSuchSession | CompanionRefusal::PageNotOpened(_) => ControlError::rejected(NOT_CREATED),
        })?;
    let parent = store.session(companion).and_then(Session::parent_id);
    core.register_page_outcome(page_id);
    core.track_page(page_id);
    Ok(ControlResult {
        page_id: Some(uuid_string(page_id)),
        parent: parent.map(uuid_string),
        ..with_id(companion)
    })
}

/// `session.companion.page.attach` (upstream's `attachCompanionPage`): one attachment of a page companion for another
/// machine, `{id, parent, companionPage}`.
pub(crate) fn attach(core: &mut Core, target: Option<&str>, window: Option<&str>) -> Outcome {
    let (store, id) = session_store(core, target, window)?;
    let parent = store.session(id).and_then(Session::parent_id);
    let page = core
        .attach_companion_page(id, std::time::Instant::now())
        .map_err(ControlError::rejected)?;
    let page = serde_json::to_value(page).map_err(|_| ControlError::Encode)?;
    Ok(ControlResult {
        parent: parent.map(uuid_string),
        companion_page: Some(page),
        ..with_id(id)
    })
}

/// The refusal of `request` when it names a page companion and is a command a page cannot take (upstream's
/// `pageCompanionRefusal`), checked before any command runs so a socket client and a page's bridge meet one refusal.
/// `None` lets the request through, to answer as it would otherwise, a target that does not resolve included.
pub(crate) fn refusal(core: &Core, request: &Request) -> Option<ControlError> {
    let args = request.args.as_ref();
    let refused = match request.cmd {
        Command::Session(SessionCmd::OverlayResize) => args.and_then(|args| args.full) != Some(true),
        Command::Session(cmd) => PAGE_REFUSED.contains(&cmd),
        Command::Ask(AskCmd::Open) => args.and_then(|args| args.style.as_deref()) != Some("gui"),
        // `active` for these means the window's zoom target, never the selection, and a hide is idempotent
        Command::Surface(SurfaceCmd::Cursor) => names_a_surface(request),
        Command::Surface(SurfaceCmd::Zoom) => {
            args.and_then(|args| args.mode.as_deref()) != Some("hide") && names_a_surface(request)
        }
        _ => false,
    };
    if !refused {
        return None;
    }
    let target = request.target.as_deref().map(session_part);
    let window = args.and_then(|args| args.window.as_deref());
    let resolved = Resolver::new(core.library()).session(target, window).ok()?;
    let page = core
        .library()
        .store(resolved.window)
        .and_then(|store| store.session(resolved.session))
        .is_some_and(|session| session.content() == SessionContent::Page);
    page.then(|| ControlError::rejected(format!("{}: not available on a page companion", request.cmd)))
}

/// Whether a surface command names its target rather than the window's zoom target.
fn names_a_surface(request: &Request) -> bool {
    request.target.as_deref().is_some_and(|target| target != ACTIVE)
}

/// The session a target names: a `surface:<session>:<slot>` address names it in the middle.
fn session_part(target: &str) -> &str {
    let parts: Vec<&str> = target.split(':').collect();
    match parts.as_slice() {
        ["surface", session, _] => session,
        _ => target,
    }
}
