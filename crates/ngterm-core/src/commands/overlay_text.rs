//! `session.overlay.search`: a text page's search, set, stepped or closed (upstream's `dispatchSessionOverlaySearch` in
//! `ControlDispatcher+Overlay.swift`, then `searchSessionOverlay` in `agterm/Control/ControlServer+TextPage.swift`);
//! `session.overlay.changes`: a diff page stepped to its next or previous change (upstream's
//! `dispatchSessionOverlayChanges`, then `stepSessionOverlayChanges`); `session.overlay.wrap`: a text page's long lines
//! wrapped or not (upstream's dispatcher arm, then `setSessionOverlayWrap`); `session.overlay.follow`: a log page's
//! following set or toggled (upstream's dispatcher arm, then `setSessionOverlayFollow`).
//!
//! Each runs on the presenting machine when a presenter listing live text pages shows the page in the addressed slot
//! (upstream's `textPageAction`, [`crate::remote::remote_text`]): it waits for that machine's answer there, which runs
//! the same local command, so replies and refusals match.
//!
//! The query lives in the model ([`crate::model::WindowLibrary::set_text_search`]), which reaches the page whenever its
//! renderer runs, so a query set before the page loads applies when it does. A step, of a search or between a diff's
//! changes, is a one-shot request to the page's renderer through the frontend
//! ([`crate::html::HtmlHost::step_text_search`], [`crate::html::HtmlHost::step_text_changes`]), which answers the count
//! once; the command waits for it a main-loop hop at a time. The wrap lives in the model too
//! ([`crate::model::WindowLibrary::set_text_wrap`]), which the page's bridge pushes back on every change.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use ngterm_proto::{Args, ControlResult, Request};
use uuid::Uuid;

use super::overlay::parse_overlay_pane;
use super::surface::POLL_INTERVAL;
use super::{Outcome, ToggleMode, args_of, with_id};
use crate::Core;
use crate::dispatch::Dispatched;
use crate::error::ControlError;
use crate::html::text_page::{self, TextChanges};
use crate::html::{HtmlOverlay, HtmlSource, SearchCount, SearchStep};
use crate::model::{OverlayPane, TextChangesFailure, TextSearchFailure, TextWrapFailure};
use crate::pane_env::uuid_string;
use crate::remote::remote_text::{RemoteTextOutcome, RemoteTextPoll, RemoteTextWait, SearchRequest, TextPageAction};
use crate::resolve::Resolver;

/// Upstream's `OverlayHtmlError.searchRequest`.
const SEARCH_REQUEST: &str = "session.overlay.search needs text, --next, --prev or --close";
/// Upstream's `OverlayHtmlError.searchTextWithStep`.
const SEARCH_TEXT_WITH_STEP: &str = "session.overlay.search: text cannot be combined with --next, --prev or --close";
const SEARCH_EMPTY: &str = "session.overlay.search: text must not be empty";
/// Upstream's dispatcher refusal of a `session.overlay.changes` with no `to`.
const CHANGES_REQUEST: &str = "session.overlay.changes requires to next or prev";

/// How long a step waits for the page's renderer before it counts the page as not loaded: upstream's call into the
/// web view fails when the page goes, where a renderer here may never answer.
const STEP_WAIT: Duration = Duration::from_secs(5);

/// Exactly one of a query and a `to` step (upstream's dispatcher arm), checked before anything is looked up.
fn parse(args: &Args) -> Result<SearchRequest, ControlError> {
    match (args.text.as_deref(), args.to.as_deref()) {
        (None, None) => Err(ControlError::rejected(SEARCH_REQUEST)),
        (Some(_), Some(_)) => Err(ControlError::rejected(SEARCH_TEXT_WITH_STEP)),
        (Some(""), None) => Err(ControlError::rejected(SEARCH_EMPTY)),
        (Some(text), None) => Ok(SearchRequest::Query(text.to_owned())),
        (None, Some("next")) => Ok(SearchRequest::Next),
        (None, Some("prev")) => Ok(SearchRequest::Previous),
        (None, Some("close")) => Ok(SearchRequest::Close),
        (None, Some(other)) => Err(ControlError::rejected(format!(
            "invalid overlay search direction: {other}"
        ))),
    }
}

/// `session.overlay.search` (upstream's `searchSessionOverlay`): the request, then the pane, before the session
/// resolves; then the local text page in the addressed slot. A query or a close answers the session's id at once; a
/// step waits for the page's count and answers `N of M`.
pub(crate) fn begin_search(core: &mut Core, request: &Request) -> Dispatched {
    let addressed = parse(args_of(request)).and_then(|search| Ok((search, addressed(core, request)?)));
    let (search, (session, pane)) = match addressed {
        Ok(addressed) => addressed,
        Err(error) => return Dispatched::done(Err(error)),
    };
    text_action(
        core,
        session,
        pane,
        &TextPageAction::Search(search.clone()),
        move |core| match start(core, session, pane, search) {
            Ok(Started::Done(result)) => Dispatched::done(Ok(*result)),
            Ok(Started::Step { session, answer }) => {
                wait(session, answer, |session, count| Ok(searched(session, count)), 0)
            }
            Err(error) => Dispatched::done(Err(error)),
        },
    )
}

/// The pane, then the session, a text page command addresses.
fn addressed(core: &Core, request: &Request) -> Result<(Uuid, Option<OverlayPane>), ControlError> {
    let args = args_of(request);
    let pane = parse_overlay_pane(args.pane.as_deref())?;
    let session = Resolver::new(core.library())
        .session(request.target.as_deref(), args.window.as_deref())?
        .session;
    Ok((session, pane))
}

/// Runs `action` on the presenting machine when it shows the page in `pane`'s slot of `session`, waiting for its
/// answer, else here through `local` (upstream's `textPageAction`).
fn text_action(
    core: &mut Core,
    session: Uuid,
    pane: Option<OverlayPane>,
    action: &TextPageAction,
    local: impl FnOnce(&mut Core) -> Dispatched,
) -> Dispatched {
    match core.remote_text_action(session, pane, action) {
        RemoteTextOutcome::Local => local(core),
        RemoteTextOutcome::Failed(error) => Dispatched::done(Err(ControlError::rejected(error))),
        RemoteTextOutcome::Waiting(wait) => remote_wait(session, wait, 0),
    }
}

/// Polls for the presenter's answer to `wait` a main-loop hop at a time, answering with its text or its error.
fn remote_wait(session: Uuid, wait: RemoteTextWait, attempt: u32) -> Dispatched {
    Dispatched::Pending {
        after: POLL_INTERVAL,
        resume: Box::new(move |core: &mut Core| match core.remote_text_poll(&wait, attempt) {
            RemoteTextPoll::Answered(Ok(text)) => Dispatched::done(Ok(ControlResult {
                text,
                ..with_id(session)
            })),
            RemoteTextPoll::Answered(Err(error)) => Dispatched::done(Err(ControlError::rejected(error))),
            RemoteTextPoll::Failed(error) => Dispatched::done(Err(ControlError::rejected(error))),
            RemoteTextPoll::Pending => remote_wait(session, wait, attempt + 1),
        }),
    }
}

/// A search step's answer: `N of M` for the match it reached.
fn searched(session: Uuid, count: SearchCount) -> ControlResult {
    ControlResult {
        text: Some(format!("{} of {}", count.current, count.matches)),
        ..with_id(session)
    }
}

/// What a search request left to do.
enum Started {
    Done(Box<ControlResult>),
    Step {
        session: Uuid,
        answer: Receiver<SearchCount>,
    },
}

fn start(
    core: &mut Core,
    session: Uuid,
    pane: Option<OverlayPane>,
    search: SearchRequest,
) -> Result<Started, ControlError> {
    let shown = text_search_page(core, session, pane).map_err(refused)?;
    let (id, revision, query) = (
        shown.id,
        shown.search_revision,
        shown.text_search.as_ref().map(|search| search.query.clone()),
    );
    let forward = match search {
        SearchRequest::Query(query) => {
            core.library_mut().set_text_search(id, &query);
            return Ok(Started::Done(Box::new(with_id(session))));
        }
        SearchRequest::Close => {
            core.library_mut().set_text_search(id, "");
            return Ok(Started::Done(Box::new(with_id(session))));
        }
        SearchRequest::Next => true,
        SearchRequest::Previous => false,
    };
    let query = query.ok_or_else(|| refused(TextSearchFailure::NoQuery))?;
    let step = SearchStep {
        forward,
        query,
        revision,
    };
    let answer = core
        .html_host()
        .and_then(|host| host.step_text_search(id, step))
        .ok_or_else(|| refused(TextSearchFailure::NotLoaded))?;
    Ok(Started::Step { session, answer })
}

/// The local text page in the slot `pane` of `session` addresses (upstream's `textSearchPage`): a slot the placement
/// rule hands to the presenting machine is refused, since presentation state carries no search.
fn text_search_page(core: &Core, session: Uuid, pane: Option<OverlayPane>) -> Result<&HtmlOverlay, TextSearchFailure> {
    if core.page_belongs_remote(session, pane) {
        return Err(TextSearchFailure::Remote);
    }
    let store = core
        .library()
        .store_for_session(session)
        .ok_or(TextSearchFailure::Page(
            crate::model::HtmlOverlayCommandFailure::UnknownSession,
        ))?;
    let found = store.html_overlay(session, pane).map_err(TextSearchFailure::Page)?;
    match found.source {
        HtmlSource::Text { .. } => Ok(found),
        _ => Err(TextSearchFailure::NotText),
    }
}

fn refused(failure: TextSearchFailure) -> ControlError {
    ControlError::rejected(failure.message())
}

/// Answers the step through `answered` once the page's renderer counted, `page not loaded` when it never does.
fn wait<T: Send + 'static>(
    session: Uuid,
    answer: Receiver<T>,
    answered: fn(Uuid, T) -> Result<ControlResult, ControlError>,
    attempt: u32,
) -> Dispatched {
    match answer.try_recv() {
        Ok(count) => Dispatched::done(answered(session, count)),
        Err(TryRecvError::Empty) if POLL_INTERVAL * attempt < STEP_WAIT => Dispatched::Pending {
            after: POLL_INTERVAL,
            resume: Box::new(move |_: &mut Core| wait(session, answer, answered, attempt + 1)),
        },
        // both failures say the same
        Err(_) => Dispatched::done(Err(refused(TextSearchFailure::NotLoaded))),
    }
}

/// `session.overlay.changes` (upstream's `dispatchSessionOverlayChanges`, then `stepSessionOverlayChanges`): the step,
/// then the pane, before the session resolves; then the local plain diff page in the addressed slot, whose renderer
/// steps and answers `N of M`.
pub(crate) fn begin_changes(core: &mut Core, request: &Request) -> Dispatched {
    let addressed = parse_changes(args_of(request)).and_then(|forward| Ok((forward, addressed(core, request)?)));
    let (forward, (session, pane)) = match addressed {
        Ok(addressed) => addressed,
        Err(error) => return Dispatched::done(Err(error)),
    };
    text_action(
        core,
        session,
        pane,
        &TextPageAction::Changes { forward },
        move |core| match start_changes(core, session, pane, forward) {
            Ok(answer) => wait(session, answer, stepped, 0),
            Err(error) => Dispatched::done(Err(error)),
        },
    )
}

/// A change step's answer: `N of M` for the change it reached, `no changes` when the page has none.
fn stepped(session: Uuid, changes: TextChanges) -> Result<ControlResult, ControlError> {
    if changes.count == 0 {
        return Err(changes_refused(TextChangesFailure::NoChanges));
    }
    Ok(ControlResult {
        text: Some(format!("{} of {}", changes.current, changes.count)),
        ..with_id(session)
    })
}

/// The step's direction: `true` for next.
fn parse_changes(args: &Args) -> Result<bool, ControlError> {
    match args.to.as_deref() {
        Some("next") => Ok(true),
        Some("prev") => Ok(false),
        None => Err(ControlError::rejected(CHANGES_REQUEST)),
        Some(other) => Err(ControlError::rejected(format!(
            "invalid overlay changes direction: {other}"
        ))),
    }
}

fn start_changes(
    core: &mut Core,
    session: Uuid,
    pane: Option<OverlayPane>,
    forward: bool,
) -> Result<Receiver<TextChanges>, ControlError> {
    let id = text_changes_page(core, session, pane).map_err(changes_refused)?.id;
    core.html_host()
        .and_then(|host| host.step_text_changes(id, forward))
        .ok_or_else(|| changes_refused(TextChangesFailure::NotLoaded))
}

/// The local plain diff page in the slot `pane` of `session` addresses (upstream's `textChangesPage`), with the slot
/// refusals of [`text_search_page`]; a log page of a patch is not one.
fn text_changes_page(
    core: &Core,
    session: Uuid,
    pane: Option<OverlayPane>,
) -> Result<&HtmlOverlay, TextChangesFailure> {
    let found = text_search_page(core, session, pane).map_err(|failure| match failure {
        TextSearchFailure::Page(failure) => TextChangesFailure::Page(failure),
        TextSearchFailure::Remote => TextChangesFailure::Remote,
        _ => TextChangesFailure::NotDiff,
    })?;
    match &found.source {
        HtmlSource::Text { path, .. } if found.text_log.is_none() && text_page::is_diff(path) => Ok(found),
        _ => Err(TextChangesFailure::NotDiff),
    }
}

fn changes_refused(failure: TextChangesFailure) -> ControlError {
    ControlError::rejected(failure.message())
}

/// `session.overlay.wrap [on|off|toggle]` (upstream's dispatcher arm, then `setSessionOverlayWrap`): the mode, then the
/// pane, before the session resolves; an absent mode toggles. Then the local text page in the addressed slot, with the
/// refusals of [`text_search_page`]. `result.text` names the state left, `wrapped` or `unwrapped`.
pub(crate) fn begin_wrap(core: &mut Core, request: &Request) -> Dispatched {
    let addressed = toggle_mode(args_of(request), "wrap").and_then(|mode| Ok((mode, addressed(core, request)?)));
    let (mode, (session, pane)) = match addressed {
        Ok(addressed) => addressed,
        Err(error) => return Dispatched::done(Err(error)),
    };
    text_action(core, session, pane, &TextPageAction::Wrap(mode), move |core| {
        Dispatched::done(wrap(core, session, pane, mode))
    })
}

/// A follow or wrap request's mode, an absent one toggling.
fn toggle_mode(args: &Args, command: &str) -> Result<ToggleMode, ControlError> {
    let raw = args.mode.as_deref();
    ToggleMode::parse(raw)
        .ok_or_else(|| ControlError::rejected(format!("invalid overlay {command} mode: {}", raw.unwrap_or("toggle"))))
}

/// The local wrap of the text page in `pane`'s slot of `session`.
pub(crate) fn wrap(core: &mut Core, session: Uuid, pane: Option<OverlayPane>, mode: ToggleMode) -> Outcome {
    let shown = text_search_page(core, session, pane).map_err(|failure| {
        let failure = match failure {
            TextSearchFailure::Page(failure) => TextWrapFailure::Page(failure),
            TextSearchFailure::Remote => TextWrapFailure::Remote,
            _ => TextWrapFailure::NotText,
        };
        ControlError::rejected(failure.message())
    })?;
    let (id, wrapped) = (shown.id, mode.desired_value(shown.text_wrap));
    core.library_mut().set_text_wrap(id, wrapped);
    Ok(ControlResult {
        text: Some((if wrapped { "wrapped" } else { "unwrapped" }).to_owned()),
        ..with_id(session)
    })
}

/// `session.overlay.follow [on|off|toggle]` (upstream's dispatcher arm, then `setSessionOverlayFollow`): the mode, then
/// the pane, before the session resolves; an absent mode toggles. `result.text` names the state left, `following` or
/// `paused`.
pub(crate) fn begin_follow(core: &mut Core, request: &Request) -> Dispatched {
    let addressed = toggle_mode(args_of(request), "follow").and_then(|mode| Ok((mode, addressed(core, request)?)));
    let (mode, (session, pane)) = match addressed {
        Ok(addressed) => addressed,
        Err(error) => return Dispatched::done(Err(error)),
    };
    text_action(core, session, pane, &TextPageAction::Follow(mode), move |core| {
        Dispatched::done(follow(core, session, pane, mode))
    })
}

/// The local follow change of the log page in `pane`'s slot of `session`.
pub(crate) fn follow(core: &mut Core, session: Uuid, pane: Option<OverlayPane>, mode: ToggleMode) -> Outcome {
    let (store, _) = super::session_store(core, Some(&uuid_string(session)), None)?;
    let following = store
        .set_text_log_following(session, pane, |current| mode.desired_value(current))
        .map_err(|failure| ControlError::rejected(failure.message()))?;
    Ok(ControlResult {
        text: Some((if following { "following" } else { "paused" }).to_owned()),
        ..with_id(session)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(text: Option<&str>, to: Option<&str>) -> Args {
        Args {
            text: text.map(str::to_owned),
            to: to.map(str::to_owned),
            ..Args::default()
        }
    }

    /// Upstream's `overlaySearchRoutesItsRequestAndPane`, the request half.
    #[test]
    fn a_search_takes_exactly_one_of_a_query_and_a_step() {
        assert_eq!(
            parse(&args(Some("main()"), None)),
            Ok(SearchRequest::Query("main()".to_owned()))
        );
        assert_eq!(parse(&args(None, Some("next"))), Ok(SearchRequest::Next));
        assert_eq!(parse(&args(None, Some("prev"))), Ok(SearchRequest::Previous));
        assert_eq!(parse(&args(None, Some("close"))), Ok(SearchRequest::Close));
    }

    /// Upstream's `overlaySearchRejectsBadInputBeforeCallingActions`, the request half.
    #[test]
    fn a_missing_mixed_empty_or_unknown_request_is_refused() {
        for (text, to, error) in [
            (None, None, SEARCH_REQUEST.to_owned()),
            (Some("a"), Some("next"), SEARCH_TEXT_WITH_STEP.to_owned()),
            (Some(""), None, SEARCH_EMPTY.to_owned()),
            (None, Some("up"), "invalid overlay search direction: up".to_owned()),
        ] {
            assert_eq!(parse(&args(text, to)), Err(ControlError::rejected(error)));
        }
    }

    /// Upstream's `overlayChangesRoutesItsDirectionAndPane` and `overlayChangesRejectsBadInputBeforeCallingActions`,
    /// the request half.
    #[test]
    fn a_changes_step_takes_next_or_prev() {
        assert_eq!(parse_changes(&args(None, Some("next"))), Ok(true));
        assert_eq!(parse_changes(&args(None, Some("prev"))), Ok(false));
        assert_eq!(
            parse_changes(&args(None, None)),
            Err(ControlError::rejected(CHANGES_REQUEST))
        );
        assert_eq!(
            parse_changes(&args(None, Some("close"))),
            Err(ControlError::rejected("invalid overlay changes direction: close"))
        );
    }
}
