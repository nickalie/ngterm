//! `session.overlay.search`: a text page's search, set, stepped or closed (upstream's `dispatchSessionOverlaySearch` in
//! `ControlDispatcher+Overlay.swift`, then `searchSessionOverlay` in `agterm/Control/ControlServer+TextPage.swift`).
//!
//! The query lives in the model ([`crate::model::WindowLibrary::set_text_search`]), which reaches the page whenever its
//! renderer runs, so a query set before the page loads applies when it does. A step is a one-shot request to the
//! page's renderer through the frontend ([`crate::html::HtmlHost::step_text_search`]), which answers the count once;
//! the command waits for it a main-loop hop at a time.

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
use crate::html::{HtmlOverlay, HtmlSource, SearchCount, SearchStep};
use crate::model::{OverlayPane, TextSearchFailure};
use crate::resolve::Resolver;

/// Upstream's `OverlayHtmlError.searchRequest`.
const SEARCH_REQUEST: &str = "session.overlay.search needs text, --next, --prev or --close";
/// Upstream's `OverlayHtmlError.searchTextWithStep`.
const SEARCH_TEXT_WITH_STEP: &str = "session.overlay.search: text cannot be combined with --next, --prev or --close";
const SEARCH_EMPTY: &str = "session.overlay.search: text must not be empty";

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
        Ok(Started::Step { session, answer }) => wait(session, answer, 0),
        Err(error) => Dispatched::done(Err(error)),
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

/// Answers the step once the page's renderer counted, `page not loaded` when it never does.
fn wait(session: Uuid, answer: Receiver<SearchCount>, attempt: u32) -> Dispatched {
    match answer.try_recv() {
        Ok(count) => Dispatched::done(Ok(ControlResult {
            text: Some(format!("{} of {}", count.current, count.matches)),
            ..with_id(session)
        })),
        Err(TryRecvError::Empty) if POLL_INTERVAL * attempt < STEP_WAIT => Dispatched::Pending {
            after: POLL_INTERVAL,
            resume: Box::new(move |_: &mut Core| wait(session, answer, attempt + 1)),
        },
        Err(_) => Dispatched::done(Err(refused(TextSearchFailure::NotLoaded))),
    }
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
}
