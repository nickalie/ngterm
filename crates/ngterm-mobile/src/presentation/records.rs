//! What an attached session's presentation stream tells the app, as `UniFFI` records: the origin's frames
//! ([`ngterm_core::remote::frames`]) flattened for Kotlin. Enumerations the origin names on the wire (a status, a shape,
//! a spinner, a position, an ask's style) travel as those wire names, and a pane as its identity (an upper-case UUID,
//! or `scratch`), so the app reads them as the protocol documents them.

use ngterm_core::model::{AskAlignment, AskChoiceForm, AskInputForm, AskInputMode, AskStyle, RemoteConnection};
use ngterm_core::pane_env::uuid_string;
use ngterm_core::remote::frames::{
    PresentationAsk, PresentationHud, PresentationLayout, PresentationMode, PresentationNotify, PresentationPane,
    PresentationStatus,
};

/// The state of an attached session's presentation stream.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum StreamState {
    /// Opening, or reopening after a failure.
    Connecting,
    /// The origin's snapshot arrived: what follows is live.
    Connected,
    /// The machine's ngterm predates the stream; final, nothing is ever opened.
    Unsupported,
    /// The stream ended or went quiet, and is retried on its own backoff.
    Failed { reason: String },
}

impl From<RemoteConnection> for StreamState {
    fn from(connection: RemoteConnection) -> Self {
        match connection {
            RemoteConnection::Connecting => Self::Connecting,
            RemoteConnection::Connected => Self::Connected,
            RemoteConnection::Unsupported => Self::Unsupported,
            RemoteConnection::Failed(reason) => Self::Failed { reason },
        }
    }
}

/// This app's role on an attached session: the presenter shows what the origin hands over (asks); a mirror only
/// watches, as another viewer holds the role or the origin offers none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, uniffi::Enum)]
pub enum PresentationRole {
    Presenter,
    Mirror,
}

impl From<PresentationMode> for PresentationRole {
    fn from(mode: PresentationMode) -> Self {
        match mode {
            PresentationMode::Presenter => Self::Presenter,
            PresentationMode::Mirror => Self::Mirror,
        }
    }
}

/// The origin's agent status of a session.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RemoteStatus {
    /// `idle`, `active`, `completed` or `blocked`.
    pub status: String,
    pub blink: bool,
    /// `#rrggbb`, when the agent set one.
    pub color: Option<String>,
    /// The glyph's shape (`circle`, `square`, …), when the agent set one.
    pub shape: Option<String>,
    /// The pane the status belongs to.
    pub pane: Option<String>,
    /// When it changed, in epoch seconds on the origin's clock.
    pub changed_at: Option<f64>,
}

impl From<PresentationStatus> for RemoteStatus {
    fn from(status: PresentationStatus) -> Self {
        Self {
            status: status.status.wire_name().to_owned(),
            blink: status.blink,
            color: status.color,
            shape: status.shape.map(|shape| shape.wire_name().to_owned()),
            pane: status.pane.map(pane_name),
            changed_at: status.changed_at,
        }
    }
}

/// A HUD panel the origin shows over a session.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct RemoteHud {
    pub message: String,
    pub detail: Option<String>,
    /// The spinner's style (`bar`, `braille`, …); `None` for a static panel.
    pub spinner: Option<String>,
    pub background_color: Option<String>,
    pub text_color: Option<String>,
    /// The panel's share of the pane's width; `None` measures it from the message.
    pub size_percent: Option<i64>,
    /// Where it sits (`top-left`, `center`, …).
    pub position: String,
    pub markdown: bool,
    pub font_size: Option<f64>,
    pub sticky: bool,
    pub frame: bool,
    /// The pane a pane-scoped panel sits over; `None` for a session-wide one.
    pub pane: Option<String>,
    /// The origin's count of the panel's publications: a later one replaces an earlier one.
    pub generation: i64,
    /// Seconds until the origin hides it, as of the frame; `None` for a persistent panel.
    pub remaining: Option<f64>,
}

impl From<PresentationHud> for RemoteHud {
    fn from(hud: PresentationHud) -> Self {
        let spec = hud.spec;
        Self {
            message: spec.message,
            detail: spec.detail,
            spinner: spec.spinner.map(|spinner| spinner.wire_name().to_owned()),
            background_color: spec.background_color,
            text_color: spec.text_color,
            size_percent: spec.size_percent,
            position: spec.position.wire_name().to_owned(),
            markdown: spec.markdown,
            font_size: spec.font_size,
            sticky: spec.sticky,
            frame: spec.frame,
            pane: hud.pane.map(pane_name),
            generation: hud.generation,
            remaining: hud.remaining,
        }
    }
}

/// A notification the origin sent, delivered as the app's own.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteNotice {
    pub title: String,
    pub body: String,
    pub pane: Option<String>,
    /// What raised it on the origin.
    pub source: String,
}

impl From<PresentationNotify> for RemoteNotice {
    fn from(notify: PresentationNotify) -> Self {
        Self {
            title: notify.title,
            body: notify.body,
            pane: notify.pane.map(pane_name),
            source: notify.source,
        }
    }
}

/// The origin's panes of a session and how they are arranged: one pane, or two side by side.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteLayout {
    /// The panes' identities, in order.
    pub panes: Vec<String>,
    /// The pane that has focus there.
    pub primary: String,
    /// The divider's direction of a split.
    pub axis: Option<String>,
    /// Whether the split shows; a hidden one shows the primary pane alone.
    pub shown: bool,
}

impl From<PresentationLayout> for RemoteLayout {
    fn from(layout: PresentationLayout) -> Self {
        Self {
            panes: layout.panes.into_iter().map(uuid_string).collect(),
            primary: uuid_string(layout.primary),
            axis: layout.axis,
            shown: layout.shown,
        }
    }
}

/// An ask the origin handed over: a dialog when its session is on screen, otherwise a notification.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteAsk {
    /// What [`crate::MobileCore::answer_ask`] names it by.
    pub id: String,
    pub title: String,
    pub message: Option<String>,
    pub buttons: Vec<RemoteAskButton>,
    pub default_id: Option<String>,
    pub destructive_id: Option<String>,
    /// The button that answers a dismissal.
    pub cancel_id: Option<String>,
    /// `terminal` or `gui`.
    pub style: String,
    /// Where the buttons sit: `left`, `center` or `right`.
    pub align: String,
    pub width: Option<i64>,
    /// The pane it covers; `None` for the whole session.
    pub pane: Option<String>,
    pub input: Option<RemoteAskInput>,
    pub choices: Option<RemoteAskChoices>,
}

impl From<PresentationAsk> for RemoteAsk {
    fn from(ask: PresentationAsk) -> Self {
        Self {
            id: ask.id,
            title: ask.title,
            message: ask.message,
            buttons: ask
                .buttons
                .into_iter()
                .map(|button| RemoteAskButton {
                    id: button.id,
                    label: button.label,
                    hotkey: button.hotkey,
                })
                .collect(),
            default_id: ask.default_id,
            destructive_id: ask.destructive_id,
            cancel_id: ask.cancel_id,
            style: style_name(ask.style).to_owned(),
            align: align_name(ask.align).to_owned(),
            width: ask.width,
            pane: ask.pane.map(pane_name),
            input: ask.input.map(RemoteAskInput::from),
            choices: ask.choices.map(RemoteAskChoices::from),
        }
    }
}

/// One button of an ask.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteAskButton {
    pub id: String,
    pub label: String,
    /// An ASCII letter that presses it.
    pub hotkey: Option<String>,
}

/// An ask's text field.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteAskInput {
    /// An editing area rather than one line.
    pub multiline: bool,
    /// The field's initial text.
    pub value: String,
    pub placeholder: Option<String>,
}

impl From<AskInputForm> for RemoteAskInput {
    fn from(input: AskInputForm) -> Self {
        Self {
            multiline: input.mode == AskInputMode::Multiline,
            value: input.value,
            placeholder: input.placeholder,
        }
    }
}

/// An ask's choice list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteAskChoices {
    pub choices: Vec<RemoteAskChoice>,
    /// Checkboxes; radio buttons otherwise.
    pub multiple: bool,
    /// The ids checked at open.
    pub checked: Vec<String>,
    /// The placeholder of a free-form row after the choices; `None` for no such row.
    pub other: Option<String>,
}

impl From<AskChoiceForm> for RemoteAskChoices {
    fn from(form: AskChoiceForm) -> Self {
        Self {
            choices: form
                .choices
                .into_iter()
                .map(|choice| RemoteAskChoice {
                    id: choice.id,
                    label: choice.label,
                    subtitle: choice.subtitle,
                })
                .collect(),
            multiple: form.multiple,
            checked: form.checked,
            other: form.other,
        }
    }
}

/// One item of an ask's choice list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct RemoteAskChoice {
    pub id: String,
    pub label: String,
    pub subtitle: Option<String>,
}

/// The user's answer to an ask: the button pressed, the field's text, the choices selected.
#[derive(Debug, Clone, Default, PartialEq, Eq, uniffi::Record)]
pub struct AskReply {
    pub button: Option<String>,
    /// The text field's content, or the free-form row's text.
    pub text: Option<String>,
    /// A choice list's selected ids.
    pub selected: Option<Vec<String>>,
}

/// A pane as the origin names it: its identity, or `scratch`.
fn pane_name(pane: PresentationPane) -> String {
    match pane {
        PresentationPane::Identity(id) => uuid_string(id),
        PresentationPane::Scratch => "scratch".to_owned(),
    }
}

fn style_name(style: AskStyle) -> &'static str {
    match style {
        AskStyle::Terminal => "terminal",
        AskStyle::Gui => "gui",
    }
}

fn align_name(align: AskAlignment) -> &'static str {
    match align {
        AskAlignment::Left => "left",
        AskAlignment::Center => "center",
        AskAlignment::Right => "right",
    }
}
