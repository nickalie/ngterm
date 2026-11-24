//! Text page actions on a page the presenting machine shows, on the origin (upstream's `AppStore+RemoteTextPage.swift`
//! and the `textPageAction` arm of `ControlServer+TextPage.swift`).
//!
//! `session.overlay.follow|search|wrap|changes` aimed at a slot whose page a presenter listing
//! [`super::hub::PAGE_TEXT_KIND`] shows travel there as a numbered `page.command` and wait for its `page.result`, at
//! most [`PresentationPageResult::DEADLINE`]. The presenter runs each as the local command and answers with the page's
//! text state, which is mirrored into the page's record here before the caller gets the reply, so a `tree` read right
//! after it already shows the change. A presenter that does not list the kind is sent nothing, and the local path's
//! refusals answer. A page that ends, parks or goes to another presenter while an action waits answers
//! [`super::page::VIEWER_GONE`].
//!
//! The presenter's `page.state` reports carry the same text state ([`mirror`]): a change of a log's following, the
//! search query or the wrap emits `tree.changed`, the counts alone never do.

use std::collections::HashMap;
use std::time::Instant;

use uuid::Uuid;

use super::frames::{
    Body, PresentationPageAction, PresentationPageCommand, PresentationPageResult, PresentationTextState,
    PresentationTextStep, PresentationToggle,
};
use super::hub::{PAGE_TEXT_KIND, Surface};
use super::page::VIEWER_GONE;
use crate::Core;
use crate::commands::ToggleMode;
use crate::html::text_page::{self, TextSearch};
use crate::html::{HtmlOverlay, HtmlSource, ReplicaContent};
use crate::model::OverlayPane;

/// Upstream's `OverlayResultError.presenterSilent`.
pub(crate) const PRESENTER_SILENT: &str = "presenter did not answer";

/// One `session.overlay.search` request (upstream's `TextSearchRequest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SearchRequest {
    Query(String),
    Next,
    Previous,
    Close,
}

/// One control action on a text page (upstream's `TextPageAction`): it travels to the presenting machine as a page
/// command and runs there as the local command does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum TextPageAction {
    Follow(ToggleMode),
    Wrap(ToggleMode),
    Search(SearchRequest),
    Changes { forward: bool },
}

impl TextPageAction {
    /// The action a page command carries; `None` for one that is not a whole text action (upstream's `init?`).
    pub(crate) fn of(command: &PresentationPageCommand) -> Option<Self> {
        use PresentationPageAction as Action;
        use PresentationTextStep as Step;
        let toggle = |toggle: PresentationToggle| match toggle {
            PresentationToggle::On => ToggleMode::On,
            PresentationToggle::Off => ToggleMode::Off,
            PresentationToggle::Toggle => ToggleMode::Toggle,
        };
        Some(
            match (command.action, command.toggle, command.query.as_ref(), command.step) {
                (Action::Follow, Some(mode), None, None) => Self::Follow(toggle(mode)),
                (Action::Wrap, Some(mode), None, None) => Self::Wrap(toggle(mode)),
                (Action::Search, None, Some(query), None) => Self::Search(SearchRequest::Query(query.clone())),
                (Action::Search, None, None, Some(Step::Next)) => Self::Search(SearchRequest::Next),
                (Action::Search, None, None, Some(Step::Prev)) => Self::Search(SearchRequest::Previous),
                (Action::Search, None, None, Some(Step::Close)) => Self::Search(SearchRequest::Close),
                (Action::Changes, None, None, Some(Step::Next)) => Self::Changes { forward: true },
                (Action::Changes, None, None, Some(Step::Prev)) => Self::Changes { forward: false },
                _ => return None,
            },
        )
    }

    /// The page command carrying this action; a query is cut by [`text_page::cut_query`], whose byte budget keeps
    /// the frame inside its cap.
    pub(crate) fn command(&self, page: String, seq: i64) -> PresentationPageCommand {
        let toggle = |mode: ToggleMode| match mode {
            ToggleMode::On => PresentationToggle::On,
            ToggleMode::Off => PresentationToggle::Off,
            ToggleMode::Toggle => PresentationToggle::Toggle,
        };
        let (action, toggle, query, step) = match self {
            Self::Follow(mode) => (PresentationPageAction::Follow, Some(toggle(*mode)), None, None),
            Self::Wrap(mode) => (PresentationPageAction::Wrap, Some(toggle(*mode)), None, None),
            Self::Search(SearchRequest::Query(query)) => (
                PresentationPageAction::Search,
                None,
                Some(text_page::cut_query(query)),
                None,
            ),
            Self::Search(SearchRequest::Next) => (
                PresentationPageAction::Search,
                None,
                None,
                Some(PresentationTextStep::Next),
            ),
            Self::Search(SearchRequest::Previous) => (
                PresentationPageAction::Search,
                None,
                None,
                Some(PresentationTextStep::Prev),
            ),
            Self::Search(SearchRequest::Close) => (
                PresentationPageAction::Search,
                None,
                None,
                Some(PresentationTextStep::Close),
            ),
            Self::Changes { forward } => (
                PresentationPageAction::Changes,
                None,
                None,
                Some(if *forward {
                    PresentationTextStep::Next
                } else {
                    PresentationTextStep::Prev
                }),
            ),
        };
        PresentationPageCommand {
            page,
            seq,
            action,
            size_percent: None,
            width: None,
            height: None,
            merge: None,
            toggle,
            query,
            step,
        }
    }
}

/// How a text action addressed to a slot ended on the origin (upstream's `RemoteTextOutcome`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteTextOutcome {
    /// The slot holds no page a presenter runs text actions on: the local path answers, its refusals included.
    Local,
    /// The action went to the presenter, whose result the caller polls for ([`Core::remote_text_poll`]).
    Waiting(RemoteTextWait),
    /// Settled without the presenter.
    Failed(&'static str),
}

/// One text action waiting for its result: the page's id on the wire, the command's number, the generation of the
/// presenter it went to, and when it stops waiting.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct RemoteTextWait {
    job: String,
    seq: i64,
    session: Uuid,
    owner: i64,
    deadline: Instant,
}

/// What a polled wait found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RemoteTextPoll {
    /// The presenter's answer: `Ok` with its text, or its error.
    Answered(Result<Option<String>, String>),
    Failed(&'static str),
    Pending,
}

/// The results of the text actions in flight, `None` until the presenter answers.
#[derive(Debug, Default)]
pub(crate) struct TextWaits(HashMap<(String, i64), Option<PresentationPageResult>>);

/// The text state of `page`, a text page here or shown here for another machine (upstream's `textState`); `None` for
/// any other page.
#[must_use]
pub(crate) fn text_state(page: &HtmlOverlay) -> Option<PresentationTextState> {
    is_text(&page.source).then(|| PresentationTextState {
        following: page.log_state.map(|state| state.following),
        log: page.log_state.map(|state| state.report),
        query: page.text_search.as_ref().map(|search| search.query.clone()),
        revision: page.search_revision,
        matches: page.text_search.as_ref().map(|search| search.matches),
        current: page.text_search.as_ref().map(|search| search.current),
        wrap: page.text_wrap,
        changes: page.text_changes,
    })
}

/// Whether `source` is a text page, here or shown here for another machine (upstream's `source.kind == .text`).
#[must_use]
pub(crate) fn is_text(source: &HtmlSource) -> bool {
    matches!(
        source,
        HtmlSource::Text { .. }
            | HtmlSource::Replica {
                content: ReplicaContent::Text { .. },
                ..
            }
    )
}

/// Takes the presenter's text state into `page`'s record (upstream's `HtmlOverlay.mirror`): true when a log's
/// following, the search query or the wrap changed; the counts alone never count.
pub(crate) fn mirror(page: &mut HtmlOverlay, text: &PresentationTextState) -> bool {
    let before = (
        page.log_state.map(|state| state.following),
        page.text_search.as_ref().map(|search| search.query.clone()),
        page.text_wrap,
    );
    if let Some(state) = page.log_state.as_mut() {
        if let Some(following) = text.following {
            state.following = following;
        }
        if let Some(log) = text.log {
            state.report = log;
        }
    }
    page.search_revision = text.revision;
    page.text_search = text
        .query
        .as_ref()
        .filter(|query| !query.is_empty())
        .map(|query| TextSearch {
            query: query.clone(),
            matches: text.matches.unwrap_or(0),
            current: text.current.unwrap_or(0),
        });
    page.text_wrap = text.wrap;
    page.text_changes = text.changes;
    before
        != (
            page.log_state.map(|state| state.following),
            page.text_search.as_ref().map(|search| search.query.clone()),
            page.text_wrap,
        )
}

impl Core {
    /// Sends `action` to the presenter showing the page in `pane`'s slot of `session` (upstream's `remoteTextAction`).
    /// A presenter that does not list [`PAGE_TEXT_KIND`] is never sent one: it cannot decode the action and would end
    /// the whole presentation.
    pub(crate) fn remote_text_action(
        &mut self,
        session: Uuid,
        pane: Option<OverlayPane>,
        action: &TextPageAction,
    ) -> RemoteTextOutcome {
        let Some(slot) = self
            .presented_session(session)
            .and_then(|found| found.remote_overlays().slot(pane))
            .filter(|slot| slot.page.is_some())
        else {
            return RemoteTextOutcome::Local;
        };
        // a headless origin holds the page with no owner until a presenter connects
        if !self.page_shown_by_owner(session, slot) {
            return RemoteTextOutcome::Failed(VIEWER_GONE);
        }
        if !self
            .remote
            .hub
            .surface_supports(Surface::Pages, PAGE_TEXT_KIND, session)
        {
            return RemoteTextOutcome::Local;
        }
        let job = slot.job.clone();
        let owner = slot.owner;
        let Some(seq) = self
            .library_mut()
            .store_for_session_mut(session)
            .and_then(|store| store.number_remote_text_command(session, &job))
        else {
            return RemoteTextOutcome::Local;
        };
        // registered before the send, which can drop a stalled presenter and end the page at once
        self.remote.text_waits.0.insert((job.clone(), seq), None);
        let wait = RemoteTextWait {
            job,
            seq,
            session,
            owner,
            deadline: (self.remote.clock)() + PresentationPageResult::DEADLINE,
        };
        let command = action.command(wait.job.clone(), seq);
        if !self
            .remote
            .hub
            .send_to_surface(Surface::Pages, Body::PageCommand(command), session)
        {
            self.remote.text_waits.0.remove(&(wait.job, wait.seq));
            return RemoteTextOutcome::Failed(VIEWER_GONE);
        }
        RemoteTextOutcome::Waiting(wait)
    }

    /// What `wait` found: the presenter's answer, the page gone (ended, parked or handed to another presenter), its
    /// wall-time deadline passed, or nothing yet. Every answer but the last drops the wait.
    pub(crate) fn remote_text_poll(&mut self, wait: &RemoteTextWait) -> RemoteTextPoll {
        let key = (wait.job.clone(), wait.seq);
        if let Some(Some(result)) = self.remote.text_waits.0.get(&key) {
            let answer = if result.ok {
                Ok(result.text.clone())
            } else {
                Err(result.error.clone().unwrap_or_default())
            };
            self.remote.text_waits.0.remove(&key);
            return RemoteTextPoll::Answered(answer);
        }
        let failure = if !self.text_wait_held(wait) {
            VIEWER_GONE
        } else if (self.remote.clock)() >= wait.deadline {
            PRESENTER_SILENT
        } else {
            return RemoteTextPoll::Pending;
        };
        self.remote.text_waits.0.remove(&key);
        RemoteTextPoll::Failed(failure)
    }

    /// Whether the page `wait` acts on is still shown by the presenter the action went to; a takeover or reattach hands
    /// the page to a new generation that never got the command.
    fn text_wait_held(&self, wait: &RemoteTextWait) -> bool {
        self.presented_session(wait.session)
            .and_then(|found| found.remote_overlays().slot_of_job(&wait.job))
            .is_some_and(|slot| slot.owner == wait.owner && self.page_shown_by_owner(wait.session, slot))
    }

    /// Applies the state a presenter's result carries, then hands the result to the action waiting for it (upstream's
    /// `receiveRemoteTextResult`). A result from a presenter that no longer holds the page is dropped.
    pub(super) fn receive_remote_text_result(&mut self, session: Uuid, result: &PresentationPageResult) {
        let held = self
            .presented_session(session)
            .and_then(|found| found.remote_overlays().slot_of_job(&result.page))
            .is_some_and(|slot| self.page_shown_by_owner(session, slot));
        if !held {
            return;
        }
        if let Some(state) = &result.state
            && let Some(store) = self.library_mut().store_for_session_mut(session)
        {
            store.mirror_remote_text(session, &result.page, state);
        }
        if let Some(waiting) = self.remote.text_waits.0.get_mut(&(result.page.clone(), result.seq)) {
            *waiting = Some(result.clone());
        }
    }
}

#[cfg(test)]
impl super::PresentationRuntime {
    /// Whether no text action waits for a result.
    pub(crate) fn text_waits_empty(&self) -> bool {
        self.text_waits.0.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::frames::{MAX_FRAME_BYTES, fits};

    /// Upstream's `TextPageAction` round trip: every action reads back from the command it travels in, a query cut to
    /// its limit, and a command missing its field or mixing two is no action.
    #[test]
    fn every_action_reads_back_from_its_command() {
        let actions = [
            TextPageAction::Follow(ToggleMode::On),
            TextPageAction::Follow(ToggleMode::Toggle),
            TextPageAction::Wrap(ToggleMode::Off),
            TextPageAction::Search(SearchRequest::Query("main".to_owned())),
            TextPageAction::Search(SearchRequest::Next),
            TextPageAction::Search(SearchRequest::Previous),
            TextPageAction::Search(SearchRequest::Close),
            TextPageAction::Changes { forward: true },
            TextPageAction::Changes { forward: false },
        ];
        for action in actions {
            let command = action.command("p".to_owned(), 3);
            assert_eq!((command.page.as_str(), command.seq), ("p", 3));
            assert_eq!(TextPageAction::of(&command), Some(action));
        }
        let long = "x".repeat(text_page::MAX_QUERY + 1);
        let command = TextPageAction::Search(SearchRequest::Query(long)).command("p".to_owned(), 1);
        assert_eq!(command.query.map(|query| query.len()), Some(text_page::MAX_QUERY));
        let marks = |count: usize| format!("\"{}", "\u{301}".repeat(count));
        let combining = marks((text_page::MAX_QUERY_BYTES - 1) / 2) + &marks(MAX_FRAME_BYTES);
        for query in [combining, "\u{1}".repeat(text_page::MAX_QUERY_BYTES)] {
            let command = TextPageAction::Search(SearchRequest::Query(query)).command("p".to_owned(), 1);
            assert!(fits(Body::PageCommand(command)));
        }
        let mut mixed = TextPageAction::Search(SearchRequest::Next).command("p".to_owned(), 1);
        mixed.query = Some("a".to_owned());
        assert_eq!(TextPageAction::of(&mixed), None);
        let mut bare = TextPageAction::Wrap(ToggleMode::On).command("p".to_owned(), 1);
        bare.toggle = None;
        assert_eq!(TextPageAction::of(&bare), None);
        bare.action = PresentationPageAction::Hide;
        assert_eq!(TextPageAction::of(&bare), None);
    }
}
