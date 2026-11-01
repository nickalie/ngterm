//! Page companions: `session.companion.page`, `session.companion.page.attach`, and the refusal of the terminal
//! commands a page companion cannot take. Validation and its messages follow upstream's `dispatchCompanionPage` in
//! `ControlDispatcher+Companions.swift`; the effects follow `createPageCompanion` and `pageCompanionRefusal` in
//! `agterm/Control/ControlServer+Companions.swift` and `attachCompanionPage` in `ControlServer+CompanionPage.swift`.
//!
//! A page companion is a row whose page is its session-wide overlay, full size, with no shell under it: never
//! persisted, never archived, closing with its page. A remote row's page companion is its origin's to create
//! ([`remote_page`], [`crate::remote::companions`]), refused here first when its URL is this machine's loopback or this
//! machine could not show it. The web views are the frontend's ([`crate::html::HtmlHost`]): a build or a host without
//! them refuses the page as `session.overlay.open` does, a remote row's page included (a Rust-only refusal).

use std::borrow::Cow;

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
use crate::remote::companions::{RemoteCompanionRequest, RemotePage};
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
    let args = on_this_drive(args);
    let source = validated_source(args.as_ref(), Some(core.library().home()))?;
    create(core, target, &args, source)
}

/// The page `session.companion.page` asks for, once its own checks passed; a file page's paths are checked against
/// `home` by this machine's rules, or left to a remote origin's with `None`.
fn validated_source(args: &Args, home: Option<&str>) -> Result<HtmlSource, ControlError> {
    let command = args.command.as_deref().unwrap_or_default();
    let source = match overlay_content(command, args, home)? {
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
    Ok(source)
}

/// A local page companion's `--html` and `--cwd` as this machine reads them. On Windows a rooted path with no drive
/// (`/srv/r.html`, which `ngtermctl` sends as typed since it may name a remote origin's file) lands on the caller's
/// drive (`callerDrive`), as the caller's shell would have put it, or on the current drive when the caller sent none;
/// elsewhere, and for any other path, the arguments are unchanged.
fn on_this_drive(args: &Args) -> Cow<'_, Args> {
    let drive = args.caller_drive.as_deref();
    let completed = |path: &Option<String>| path.as_deref().and_then(|path| drive_completed(path, drive));
    let (html, cwd) = (completed(&args.html), completed(&args.cwd));
    if html.is_none() && cwd.is_none() {
        return Cow::Borrowed(args);
    }
    Cow::Owned(Args {
        html: html.or_else(|| args.html.clone()),
        cwd: cwd.or_else(|| args.cwd.clone()),
        ..args.clone()
    })
}

/// `path` on `drive` (or the current drive, when `drive` is `None` or does not root it) when it is a rooted path with
/// no drive on Windows; `None` anywhere else.
fn drive_completed(path: &str, drive: Option<&str>) -> Option<String> {
    let driveless = cfg!(windows) && path.starts_with(['/', '\\']) && !html::is_absolute(path);
    if !driveless {
        return None;
    }
    let on_drive = drive
        .map(|drive| format!("{drive}{path}"))
        .filter(|on_drive| html::is_absolute(on_drive));
    std::path::absolute(on_drive.as_deref().unwrap_or(path))
        .ok()
        .map(|path| path.to_string_lossy().into_owned())
}

/// `session.companion.page` on a row attached from another machine (upstream's remote branch of
/// `createPageCompanion`): the page's checks, then what this machine must be able to show, before the origin is asked.
/// A file page's path and `--cwd` are the origin's, sent as they came and checked there by that machine's rules,
/// since this one's (a Windows viewer's, its home) say nothing about them. The request for the origin and whether the
/// row is selected (`--follow`).
pub(crate) fn remote_page(core: &Core, args: &Args) -> Result<(RemoteCompanionRequest, bool), ControlError> {
    let source = validated_source(args, None)?;
    let opening = PageOpening::from_args(source, args, None, OverlayPointSize::default());
    // the page would load the origin's loopback here, where nothing answers for it
    if let HtmlSource::Url(url) = &opening.source
        && html::is_loopback(url.as_str())
    {
        return Err(ControlError::rejected(REMOTE_LOOPBACK));
    }
    shows_pages(core, opening.flags.persistent)?;
    let page = RemotePage {
        source: opening.source,
        flags: opening.flags,
        background_color: opening.background_color,
    };
    Ok((RemoteCompanionRequest::Page(page), opening.follow))
}

/// Refuses a page this build or host cannot show: no web views, or a persistent page without the saved store.
fn shows_pages(core: &Core, persistent: bool) -> Result<(), ControlError> {
    let host = core
        .html_host()
        .ok_or_else(|| ControlError::rejected(PAGES_UNAVAILABLE))?;
    if let Some(reason) = host.web_views_unavailable() {
        return Err(ControlError::rejected(format!("{PAGES_UNAVAILABLE_HERE}: {reason}")));
    }
    if persistent && let Some(failure) = host.persistent_store_failure() {
        return Err(ControlError::rejected(format!("session.companion.page: {failure}")));
    }
    Ok(())
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
    shows_pages(core, opening.flags.persistent)?;
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

#[cfg(all(test, windows))]
mod tests {
    use super::drive_completed;

    #[test]
    fn a_driveless_rooted_path_lands_on_the_callers_drive() {
        assert_eq!(
            drive_completed("/reports/./r.html", Some("D:")).as_deref(),
            Some(r"D:\reports\r.html")
        );
        assert_eq!(
            drive_completed("/r.html", Some(r"\\host\share")).as_deref(),
            Some(r"\\host\share\r.html")
        );
        assert_eq!(drive_completed(r"E:\r.html", Some("D:")), None);
        assert_eq!(drive_completed("r.html", Some("D:")), None);
        let current = std::path::absolute("/r.html").unwrap().to_string_lossy().into_owned();
        assert_eq!(drive_completed("/r.html", None), Some(current.clone()));
        assert_eq!(drive_completed("/r.html", Some("junk")), Some(current));
    }
}
