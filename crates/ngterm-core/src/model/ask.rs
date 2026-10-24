//! Questions (`ask.*`): the dialog a caller opens, its keyboard navigation, and the slot a terminal ask waits in on
//! its session (upstream's `Ask.swift` and `Session`'s ask slot). A GUI ask waits in its window's modal slot instead
//! ([`crate::modal`]).
//!
//! A session's ask ends here, by an answer, a dismissal or a cancel; the ended outcome waits in the store until the
//! core retains it in its registry ([`AppStore::take_ended_asks`]), as upstream's `Session.resolveAsk` hands it to
//! `AskRegistry`.

use ngterm_proto::{AskButton, ControlAskOutcome, ControlAskResult};
use uuid::Uuid;

use super::ask_form::{AskChoiceForm, AskDraft, AskForm, AskInputForm, AskInputMode};
use super::{AppStore, OverlayPane, Session};
use crate::remote::frames::PresentationAskAnswer;

/// The most buttons one dialog takes (upstream's `ControlAskButton.maxButtons`).
pub const MAX_ASK_BUTTONS: usize = 6;

/// Who owns a dialog and how it looks (upstream's `ControlAskStyle`): a terminal ask waits on its session in the
/// terminal's colors, a GUI ask holds its window's modal slot with native buttons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AskStyle {
    #[default]
    Terminal,
    Gui,
}

impl AskStyle {
    /// The style a wire name names.
    #[must_use]
    pub fn from_wire(raw: &str) -> Option<Self> {
        match raw {
            "terminal" => Some(Self::Terminal),
            "gui" => Some(Self::Gui),
            _ => None,
        }
    }
}

/// Where the button block sits (upstream's `ControlAskAlignment`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AskAlignment {
    Left,
    Center,
    #[default]
    Right,
}

impl AskAlignment {
    /// The alignment a wire name names.
    #[must_use]
    pub fn from_wire(raw: &str) -> Option<Self> {
        match raw {
            "left" => Some(Self::Left),
            "center" => Some(Self::Center),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
}

/// The session or pane a GUI ask was opened over (upstream's `AskAnchor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskAnchor {
    pub session: Uuid,
    /// The role resolved at open; `None` for a session-wide anchor.
    pub pane: Option<OverlayPane>,
    /// The pane's stable identity, which follows it through swaps and promotion.
    pub pane_identity: Option<Uuid>,
}

/// A dialog awaiting an answer (upstream's `PendingAsk`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAsk {
    /// The globally unique dialog id (an upper-case UUID).
    pub id: String,
    pub title: String,
    pub message: Option<String>,
    /// The caller's buttons in their order, hotkeys lower-cased.
    pub buttons: Vec<AskButton>,
    /// The initially highlighted button.
    pub default_id: Option<String>,
    /// The button styled as destructive.
    pub destructive_id: Option<String>,
    pub style: AskStyle,
    pub align: AskAlignment,
    /// A fixed width in percent of the anchor; `None` sizes to the content.
    pub width: Option<i64>,
    /// A GUI ask's session or pane; a terminal ask's placement lives on its session.
    pub anchor: Option<AskAnchor>,
    /// The text field or choice list shown above the buttons; `None` for a plain button ask.
    pub form: Option<AskForm>,
    /// The button that resolves `escaped`, as Esc does.
    pub cancel_id: Option<String>,
}

impl PendingAsk {
    /// The outcome of pressing the button at `index` with `draft` as the form's input (upstream's `answer(buttonAt:
    /// draft:)`): the cancel button escapes with no payload, any other answers with the field text, or the selected
    /// ids plus the selected free-form row's text. Without a draft the form's initial values answer; `None` past the
    /// last button.
    #[must_use]
    pub fn answer(&self, index: usize, draft: Option<&AskDraft>) -> Option<ControlAskResult> {
        let button = self.buttons.get(index)?;
        if self.cancel_id.as_ref() == Some(&button.id) {
            return Some(ControlAskResult::outcome(ControlAskOutcome::Escaped));
        }
        let initial;
        let input = if let Some(draft) = draft {
            draft
        } else {
            initial = AskDraft::new(self.form.as_ref());
            &initial
        };
        let (text, selected) = match &self.form {
            Some(AskForm::Input(_)) => (Some(input.text().to_owned()), None),
            Some(AskForm::Choices(_)) => (input.other_text().map(str::to_owned), Some(input.selected())),
            None => (None, None),
        };
        Some(ControlAskResult {
            text,
            selected,
            ..self.answered(index)
        })
    }

    /// Checks an answer that arrived from elsewhere against this ask (upstream's `validatedAnswer`): a known button,
    /// the cancel role applied here, and a payload matching the form: text within the cap, or known ids in the
    /// caller's order plus the free-form row's non-blank text, exactly one of them for radio. Text follows its
    /// field's control-character rule. `None` refuses the answer.
    #[must_use]
    pub fn validated_answer(
        &self,
        button_id: &str,
        text: Option<&str>,
        selected: Option<&[String]>,
    ) -> Option<ControlAskResult> {
        let index = self.buttons.iter().position(|button| button.id == button_id)?;
        if self.cancel_id.as_deref() == Some(button_id) {
            return (text.is_none() && selected.is_none())
                .then(|| ControlAskResult::outcome(ControlAskOutcome::Escaped));
        }
        let valid = match &self.form {
            Some(AskForm::Input(input)) => selected.is_none() && text.is_some_and(|text| fits_field(text, input.mode)),
            Some(AskForm::Choices(list)) => selected.is_some_and(|selected| valid_selection(list, text, selected)),
            None => text.is_none() && selected.is_none(),
        };
        valid.then(|| ControlAskResult {
            text: text.map(str::to_owned),
            selected: selected.map(<[String]>::to_vec),
            ..self.answered(index)
        })
    }

    /// The same dialog anchored at `anchor`, everything else kept (upstream's `anchored(at:)`).
    #[must_use]
    pub fn anchored(&self, anchor: Option<AskAnchor>) -> Self {
        Self { anchor, ..self.clone() }
    }

    /// An answer with the button at `index`, which exists, and no payload.
    fn answered(&self, index: usize) -> ControlAskResult {
        let button = &self.buttons[index];
        ControlAskResult {
            id: Some(button.id.clone()),
            label: Some(button.label.clone()),
            index: i64::try_from(index).ok(),
            ..ControlAskResult::outcome(ControlAskOutcome::Answered)
        }
    }
}

/// Text a field of `mode` could hold: within the cap and its control rule.
fn fits_field(text: &str, mode: AskInputMode) -> bool {
    text.len() <= AskInputForm::MAX_TEXT_BYTES && !AskInputForm::has_forbidden_controls(text, mode)
}

/// Whether `selected` and the free-form `text` are an answer `list` could produce.
fn valid_selection(list: &AskChoiceForm, text: Option<&str>, selected: &[String]) -> bool {
    if let Some(text) = text
        && (list.other.is_none() || AskChoiceForm::is_blank(text) || !fits_field(text, AskInputMode::Line))
    {
        return false;
    }
    let in_caller_order = list
        .choices
        .iter()
        .map(|choice| &choice.id)
        .filter(|id| selected.contains(id))
        .eq(selected.iter());
    let picks = selected.len() + usize::from(text.is_some());
    in_caller_order && (list.multiple || picks == 1)
}

/// A terminal ask waiting in its session's slot, and the pane it covers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionAsk {
    pub ask: PendingAsk,
    /// The covered pane's stable identity; `None` covers the whole session.
    pub pane_identity: Option<Uuid>,
    /// The presenter generation the ask was handed to, `None` while this instance draws it (upstream's
    /// `askRemoteOwner`). A remotely presented ask keeps the slot and its result here but is neither drawn nor
    /// answered here.
    pub remote_owner: Option<i64>,
    /// On a viewer, the origin's presenter generation of an ask the origin handed over (upstream's `askReplica`):
    /// drawn and answered here, but owned and resolved on the origin, which its outcome is sent to.
    pub replica: Option<i64>,
    /// The user's unsubmitted input (upstream's `Session.askDraft`): made from the form at open, kept while the dialog
    /// is hidden, gone with the slot.
    pub draft: AskDraft,
}

/// A session's ask that ended, waiting for the core's registry to retain its outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedAsk {
    pub session: Uuid,
    pub id: String,
    pub result: ControlAskResult,
    /// The presenter generation it was handed to, which is told to take its dialog down.
    pub remote_owner: Option<i64>,
    /// The origin's owner generation of a replica, whose outcome goes to the origin instead of the registry.
    pub replica: Option<i64>,
}

impl Session {
    /// The pending terminal ask.
    #[must_use]
    pub fn ask(&self) -> Option<&SessionAsk> {
        self.ask.as_ref()
    }

    /// The pane the pending ask covers now (upstream's `askTargetPane`); `None` for session-wide placement, no ask, or
    /// a pane that is gone.
    #[must_use]
    pub fn ask_target_pane(&self) -> Option<OverlayPane> {
        self.ask
            .as_ref()
            .and_then(|ask| ask.pane_identity)
            .and_then(|identity| self.pane_role_for_identity(identity))
    }
}

impl AppStore {
    /// Reserves session `session`'s ask slot for `ask` over the pane `pane_identity` names (upstream's `openAsk`);
    /// false when the session is unknown or already has an ask pending.
    pub fn open_session_ask(&mut self, session: Uuid, ask: PendingAsk, pane_identity: Option<Uuid>) -> bool {
        self.open_session_ask_owned(session, ask, pane_identity, None)
    }

    /// [`AppStore::open_session_ask`] for an ask handed to the presenter of generation `remote_owner` (`None` draws
    /// it here).
    pub fn open_session_ask_owned(
        &mut self,
        session: Uuid,
        ask: PendingAsk,
        pane_identity: Option<Uuid>,
        remote_owner: Option<i64>,
    ) -> bool {
        let Some(session) = self.session_mut(session) else {
            return false;
        };
        if session.ask.is_some() {
            return false;
        }
        let draft = AskDraft::new(ask.form.as_ref());
        session.ask = Some(SessionAsk {
            ask,
            pane_identity,
            remote_owner,
            replica: None,
            draft,
        });
        true
    }

    /// Edits session `session`'s draft of ask `id` (upstream's `updateAskDraft`); a stale id, from a dialog already
    /// resolved, changes nothing.
    pub fn update_session_ask_draft(&mut self, session: Uuid, id: &str, change: impl FnOnce(&mut AskDraft)) {
        if let Some(pending) = self
            .session_mut(session)
            .and_then(|found| found.ask.as_mut())
            .filter(|pending| pending.ask.id == id)
        {
            change(&mut pending.draft);
        }
    }

    /// Reserves session `session`'s ask slot for a replica of an ask its origin handed over under presenter generation
    /// `owner` (upstream's `openReplicaAsk`); false when the session is unknown or already has an ask pending.
    pub fn open_replica_ask(
        &mut self,
        session: Uuid,
        ask: PendingAsk,
        pane_identity: Option<Uuid>,
        owner: i64,
    ) -> bool {
        if !self.open_session_ask_owned(session, ask, pane_identity, None) {
            return false;
        }
        if let Some(pending) = self.session_mut(session).and_then(|found| found.ask.as_mut()) {
            pending.replica = Some(owner);
        }
        true
    }

    /// Takes a handed-over ask back to be drawn here, so an answer from its former presenter is stale (upstream's
    /// `takeAskBack`).
    pub fn take_session_ask_back(&mut self, session: Uuid) {
        if let Some(pending) = self.session_mut(session).and_then(|found| found.ask.as_mut()) {
            pending.remote_owner = None;
            // drawn here from the form's initial values: what the presenter typed never travelled
            pending.draft = AskDraft::new(pending.ask.form.as_ref());
        }
    }

    /// Empties session `session`'s ask slot without an outcome, for an ask that moves to another owner rather than
    /// ending (upstream's `releaseAsk`).
    pub fn release_session_ask(&mut self, session: Uuid) {
        if let Some(found) = self.session_mut(session) {
            found.ask = None;
        }
    }

    /// Whether session `session` presents ask `id` remotely under presenter generation `owner`, so a refusal from a
    /// presenter that already lost it cannot take back a newer one (upstream's `isPresentingRemotely`).
    #[must_use]
    pub fn is_presenting_remotely(&self, session: Uuid, id: &str, owner: i64) -> bool {
        self.session(session)
            .and_then(Session::ask)
            .is_some_and(|pending| pending.ask.id == id && pending.remote_owner == Some(owner))
    }

    /// Applies the presenter's answer to session `session`'s ask (upstream's `resolveRemoteAsk`). Refused unless it is
    /// for the ask this session presents under that owner: a late answer after a handback, a dismissal carrying a
    /// payload, or a button the ask does not have or a payload its form would not produce
    /// ([`PendingAsk::validated_answer`]) changes nothing, and the ask stays pending.
    pub fn resolve_remote_ask(&mut self, session: Uuid, answer: &PresentationAskAnswer) -> bool {
        if !self.is_presenting_remotely(session, &answer.id, answer.owner) {
            return false;
        }
        let Some(button) = answer.button.as_deref() else {
            if answer.text.is_some() || answer.selected.is_some() {
                return false;
            }
            return self.resolve_session_ask(
                session,
                &answer.id,
                ControlAskResult::outcome(ControlAskOutcome::Escaped),
            );
        };
        let result = self.session(session).and_then(Session::ask).and_then(|pending| {
            pending
                .ask
                .validated_answer(button, answer.text.as_deref(), answer.selected.as_deref())
        });
        result.is_some_and(|result| self.resolve_session_ask(session, &answer.id, result))
    }

    /// Ends session `session`'s pending ask `id` with `result` (upstream's `resolveAsk`), keeping the outcome for the
    /// registry; false for a stale id or a pending result.
    pub fn resolve_session_ask(&mut self, session: Uuid, id: &str, result: ControlAskResult) -> bool {
        if result.result == ControlAskOutcome::Pending {
            return false;
        }
        let Some(target) = self.session_mut(session) else {
            return false;
        };
        if target.ask.as_ref().is_none_or(|pending| pending.ask.id != id) {
            return false;
        }
        let (remote_owner, replica) = target
            .ask
            .take()
            .map_or((None, None), |pending| (pending.remote_owner, pending.replica));
        self.ended_asks.push(EndedAsk {
            session,
            id: id.to_owned(),
            result,
            remote_owner,
            replica,
        });
        true
    }

    /// Cancels session `session`'s pending ask, if any (upstream's `cancelPendingAsk`).
    pub fn cancel_pending_ask(&mut self, session: Uuid) {
        let id = self
            .session(session)
            .and_then(|target| target.ask.as_ref())
            .map(|pending| pending.ask.id.clone());
        if let Some(id) = id {
            self.resolve_session_ask(session, &id, ControlAskResult::outcome(ControlAskOutcome::Cancelled));
        }
    }

    /// Cancels session `session`'s ask when it covers the pane `identity` names, before that identity goes or moves.
    pub(crate) fn cancel_ask_owned_by(&mut self, session: Uuid, identity: Option<Uuid>) {
        let owned = identity.is_some_and(|identity| {
            self.session(session)
                .and_then(Session::ask)
                .is_some_and(|ask| ask.pane_identity == Some(identity))
        });
        if owned {
            self.cancel_pending_ask(session);
        }
    }

    /// What a session leaving the window drops at once (upstream's `releaseLeavingSession`): its pending ask ends
    /// cancelled and the overlays it handed to a presenter close, so a soft close's undo brings back neither.
    pub(crate) fn release_leaving_session(&mut self, session: Uuid) {
        self.cancel_pending_ask(session);
        self.release_remote_overlays(session);
    }

    /// Cancels every session's pending ask (a window going away, the app quitting).
    pub fn cancel_all_session_asks(&mut self) {
        let ids: Vec<Uuid> = self.sessions().map(Session::id).collect();
        for id in ids {
            self.cancel_pending_ask(id);
        }
    }

    /// Takes the session asks that ended since the last take, in order.
    pub fn take_ended_asks(&mut self) -> Vec<EndedAsk> {
        std::mem::take(&mut self.ended_asks)
    }
}

/// The keyboard selection over a dialog's buttons, in the caller's order (upstream's `AskNavigation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskNavigation {
    highlighted: Option<usize>,
    count: usize,
    hotkeys: Vec<Option<String>>,
}

impl AskNavigation {
    /// Starts on the default button, else the first non-destructive one, else the first.
    #[must_use]
    pub fn new(ask: &PendingAsk) -> Self {
        let buttons = &ask.buttons;
        let by_id = |id: &String| buttons.iter().position(|button| &button.id == id);
        let highlighted = ask
            .default_id
            .as_ref()
            .and_then(by_id)
            .or_else(|| {
                buttons
                    .iter()
                    .position(|button| Some(&button.id) != ask.destructive_id.as_ref())
            })
            .or(if buttons.is_empty() { None } else { Some(0) });
        Self {
            highlighted,
            count: buttons.len(),
            hotkeys: buttons
                .iter()
                .map(|button| button.hotkey.as_ref().map(|key| key.to_lowercase()))
                .collect(),
        }
    }

    /// The highlighted button's index; `None` only for an empty list.
    #[must_use]
    pub fn highlighted(&self) -> Option<usize> {
        self.highlighted
    }

    /// Moves to the next button, entering at the first and wrapping after the last.
    pub fn move_forward(&mut self) {
        if self.count == 0 {
            return;
        }
        self.highlighted = Some(self.highlighted.map_or(0, |index| (index + 1) % self.count));
    }

    /// Moves to the previous button, entering at the last and wrapping before the first.
    pub fn move_backward(&mut self) {
        if self.count == 0 {
            return;
        }
        self.highlighted = Some(
            self.highlighted
                .map_or(self.count - 1, |index| (index + self.count - 1) % self.count),
        );
    }

    /// The button Return activates.
    #[must_use]
    pub fn activate(&self) -> Option<usize> {
        self.highlighted
    }

    /// The button whose hotkey is `letter`, case-insensitively.
    #[must_use]
    pub fn hotkey(&self, letter: &str) -> Option<usize> {
        let key = letter.to_lowercase();
        self.hotkeys
            .iter()
            .position(|hotkey| hotkey.as_deref() == Some(key.as_str()))
    }
}
