//! `session.overlay.search`: a text page's search, set, stepped or closed (upstream's `dispatchSessionOverlaySearch` in
//! `ControlDispatcher+Overlay.swift`, then `searchSessionOverlay` in `agterm/Control/ControlServer+TextPage.swift`);
//! `session.overlay.changes`: a diff page stepped to its next or previous change (upstream's
//! `dispatchSessionOverlayChanges`, then `stepSessionOverlayChanges`).
//!
//! The query lives in the model ([`crate::model::WindowLibrary::set_text_search`]), which reaches the page whenever its
//! renderer runs, so a query set before the page loads applies when it does. A step, of a search or between a diff's
//! changes, is a one-shot request to the page's renderer through the frontend
//! ([`crate::html::HtmlHost::step_text_search`], [`crate::html::HtmlHost::step_text_changes`]), which answers the count
//! once; the command waits for it a main-loop hop at a time.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;

use ngterm_proto::{Args, ControlResult, Request};
use uuid::Uuid;

use super::overlay::parse_overlay_pane;
use super::surface::POLL_INTERVAL;
use super::{args_of, with_id};
use crate::Core;
use crate::dispatch::Dispatched;
use crate::error::ControlError;
use crate::html::text_page::{self, TextChanges};
use crate::html::{HtmlOverlay, HtmlSource, SearchCount, SearchStep};
use crate::model::{OverlayPane, TextChangesFailure, TextSearchFailure};
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

/// One `session.overlay.search` request (upstream's `TextSearchRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum SearchRequest {
    Query(String),
    Next,
    Previous,
    Close,
}

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
    match start(core, request) {
        Ok(Started::Done(result)) => Dispatched::done(Ok(*result)),
        Ok(Started::Step { session, answer }) => {
            wait(session, answer, |session, count| Ok(searched(session, count)), 0)
        }
        Err(error) => Dispatched::done(Err(error)),
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

fn start(core: &mut Core, request: &Request) -> Result<Started, ControlError> {
    let args = args_of(request);
    let search = parse(args)?;
    let pane = parse_overlay_pane(args.pane.as_deref())?;
    let session = Resolver::new(core.library())
        .session(request.target.as_deref(), args.window.as_deref())?
        .session;
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
    match start_changes(core, request) {
        Ok((session, answer)) => wait(session, answer, stepped, 0),
        Err(error) => Dispatched::done(Err(error)),
    }
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

fn start_changes(core: &mut Core, request: &Request) -> Result<(Uuid, Receiver<TextChanges>), ControlError> {
    let args = args_of(request);
    let forward = parse_changes(args)?;
    let pane = parse_overlay_pane(args.pane.as_deref())?;
    let session = Resolver::new(core.library())
        .session(request.target.as_deref(), args.window.as_deref())?
        .session;
    let id = text_changes_page(core, session, pane).map_err(changes_refused)?.id;
    let answer = core
        .html_host()
        .and_then(|host| host.step_text_changes(id, forward))
        .ok_or_else(|| changes_refused(TextChangesFailure::NotLoaded))?;
    Ok((session, answer))
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
