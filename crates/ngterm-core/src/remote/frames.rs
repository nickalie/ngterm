//! The presentation stream's frames (upstream's `PresentationFrames.swift` and `PresentationLayout.swift`): one JSON
//! object per line, `{"kind", "gen", "rev", <payload>}`, carried between an origin session and the viewers attached to
//! it.
//!
//! `gen` is the connection generation and `rev` a revision monotonic within it, so a receiver drops whatever is left
//! over from an earlier connection. A kind this build does not speak decodes to [`Body::Unknown`], keeping its place
//! in the order, so a newer peer does not break the stream.

use std::fmt;

use ngterm_proto::{AskButton, AskChoice};
use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::html::HtmlLoadState;
use crate::hud::{HudPosition, HudSpec, HudSpinner};
use crate::model::{
    AgentStatus, AskAlignment, AskChoiceForm, AskForm, AskInputForm, AskInputMode, AskStyle, PendingAsk, StatusShape,
};
use crate::pane_env::uuid_string;

/// The presentation protocol version this build speaks.
pub const VERSION: i64 = 1;
/// A HUD body is the largest legitimate frame; this leaves it room and still bounds a hostile line.
pub const MAX_FRAME_BYTES: usize = 256 * 1024;
/// Frames a subscriber may have queued before it counts as stalled.
pub const MAX_PENDING_FRAMES: usize = 256;

/// A UUID as Foundation reads one: the hyphenated form, either case.
pub(crate) fn parse_uuid(raw: &str) -> Option<Uuid> {
    (raw.len() == 36).then(|| Uuid::parse_str(raw).ok()).flatten()
}

/// A UUID as Foundation encodes one: hyphenated, upper case.
fn serialize_uuid<S: Serializer>(id: &Uuid, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&uuid_string(*id))
}

/// A pane of the origin session on the wire (upstream's `PresentationPane`). A main or split pane travels as its stable
/// identity, never as a left/right role, so a swap or promotion on either side cannot misroute it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresentationPane {
    Identity(Uuid),
    Scratch,
}

const SCRATCH: &str = "scratch";

impl Serialize for PresentationPane {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Identity(id) => serializer.serialize_str(&uuid_string(*id)),
            Self::Scratch => serializer.serialize_str(SCRATCH),
        }
    }
}

impl<'de> Deserialize<'de> for PresentationPane {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        if raw == SCRATCH {
            return Ok(Self::Scratch);
        }
        parse_uuid(&raw)
            .map(Self::Identity)
            .ok_or_else(|| de::Error::custom("pane is neither scratch nor a uuid"))
    }
}

/// What a viewer asks to be: a mirror beside the origin, or the session's sole presenter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PresentationMode {
    Mirror,
    Presenter,
}

/// Opens a stream in both directions and carries what each side can speak.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationHello {
    pub version: i64,
    pub kinds: Vec<String>,
    pub mode: PresentationMode,
}

/// Serde for a status by its wire spelling.
mod status_wire {
    use super::{AgentStatus, Deserialize, Deserializer, Serializer, de};

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's `with` hands the field by reference"
    )]
    pub(super) fn serialize<S: Serializer>(status: &AgentStatus, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(status.wire_name())
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<AgentStatus, D::Error> {
        let raw = String::deserialize(deserializer)?;
        AgentStatus::from_wire(&raw).ok_or_else(|| de::Error::custom(format!("unknown status: {raw}")))
    }
}

/// Serde for an optional shape by its wire spelling.
mod shape_wire {
    use super::{Deserialize, Deserializer, Serializer, StatusShape, de};

    #[expect(
        clippy::ref_option,
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's `with` hands the field by reference"
    )]
    pub(super) fn serialize<S: Serializer>(shape: &Option<StatusShape>, serializer: S) -> Result<S::Ok, S::Error> {
        match shape {
            Some(shape) => serializer.serialize_some(shape.wire_name()),
            None => serializer.serialize_none(),
        }
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<StatusShape>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|raw| StatusShape::from_wire(&raw).ok_or_else(|| de::Error::custom(format!("unknown shape: {raw}"))))
            .transpose()
    }
}

/// A session's full non-idle agent status; idle travels as an absent status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationStatus {
    #[serde(with = "status_wire")]
    pub status: AgentStatus,
    pub blink: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "shape_wire")]
    pub shape: Option<StatusShape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<PresentationPane>,
    /// Epoch seconds on the origin's clock.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub changed_at: Option<f64>,
}

/// A HUD spec as it travels (upstream's `HudSpec` coding: every field optional on decode but the message).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HudWire {
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    detail: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    spinner: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    background_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    text_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    size_percent: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    position: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    hide_after: Option<f64>,
    #[serde(default)]
    markdown: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    font_size: Option<f64>,
    #[serde(default)]
    sticky: bool,
    #[serde(default = "default_frame")]
    frame: bool,
}

fn default_frame() -> bool {
    true
}

/// Serde for a [`HudSpec`] through [`HudWire`].
mod hud_spec {
    use super::{Deserialize, Deserializer, HudPosition, HudSpec, HudSpinner, HudWire, Serialize, Serializer, de};

    pub(super) fn serialize<S: Serializer>(spec: &HudSpec, serializer: S) -> Result<S::Ok, S::Error> {
        HudWire {
            message: spec.message.clone(),
            detail: spec.detail.clone(),
            spinner: spec.spinner.map(|spinner| spinner.wire_name().to_owned()),
            background_color: spec.background_color.clone(),
            text_color: spec.text_color.clone(),
            size_percent: spec.size_percent,
            position: Some(spec.position.wire_name().to_owned()),
            hide_after: spec.hide_after,
            markdown: spec.markdown,
            font_size: spec.font_size,
            sticky: spec.sticky,
            frame: spec.frame,
        }
        .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<HudSpec, D::Error> {
        let wire = HudWire::deserialize(deserializer)?;
        let spinner = wire
            .spinner
            .map(|raw| HudSpinner::from_wire(&raw).ok_or_else(|| de::Error::custom(format!("unknown spinner: {raw}"))))
            .transpose()?;
        let position = match wire.position {
            Some(raw) => {
                HudPosition::parse(&raw).ok_or_else(|| de::Error::custom(format!("unknown hud position: {raw}")))?
            }
            None => HudPosition::DEFAULT,
        };
        Ok(HudSpec {
            message: wire.message,
            detail: wire.detail,
            spinner,
            background_color: wire.background_color,
            text_color: wire.text_color,
            size_percent: wire.size_percent,
            position,
            hide_after: wire.hide_after,
            markdown: wire.markdown,
            font_size: wire.font_size,
            sticky: wire.sticky,
            frame: wire.frame,
        })
    }
}

/// The origin's live HUD. `generation` counts its publications; a withdrawal carries none, and the frame's `rev`
/// orders it against a replacement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PresentationHud {
    #[serde(with = "hud_spec")]
    pub spec: HudSpec,
    /// The pane a pane-scoped panel sits over; `None` for a session-wide one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<PresentationPane>,
    pub generation: i64,
    /// Seconds until the origin hides the panel, sampled when the frame was built; `None` for a persistent one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
}

/// One control-origin notification: a live event, never part of a snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationNotify {
    pub title: String,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<PresentationPane>,
    pub source: String,
}

/// A status sound the origin hands its presenter to play instead of playing it itself (upstream's
/// `PresentationSound`): a one-shot event, never part of a snapshot. A `None` name is the blocked-default marker, which
/// the presenter plays as its own Settings blocked sound, so each machine's choice applies where it plays; it encodes
/// as `{}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationSound {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Serde for an ask style by its wire spelling.
mod style_wire {
    use super::{AskStyle, Deserialize, Deserializer, Serializer, de};

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's `with` hands the field by reference"
    )]
    pub(super) fn serialize<S: Serializer>(style: &AskStyle, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match style {
            AskStyle::Terminal => "terminal",
            AskStyle::Gui => "gui",
        })
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<AskStyle, D::Error> {
        let raw = String::deserialize(deserializer)?;
        AskStyle::from_wire(&raw).ok_or_else(|| de::Error::custom(format!("unknown style: {raw}")))
    }
}

/// Serde for an ask alignment by its wire spelling.
mod align_wire {
    use super::{AskAlignment, Deserialize, Deserializer, Serializer, de};

    #[expect(
        clippy::trivially_copy_pass_by_ref,
        reason = "serde's `with` hands the field by reference"
    )]
    pub(super) fn serialize<S: Serializer>(align: &AskAlignment, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(match align {
            AskAlignment::Left => "left",
            AskAlignment::Center => "center",
            AskAlignment::Right => "right",
        })
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<AskAlignment, D::Error> {
        let raw = String::deserialize(deserializer)?;
        AskAlignment::from_wire(&raw).ok_or_else(|| de::Error::custom(format!("unknown align: {raw}")))
    }
}

/// Serde for an ask's text field (upstream's `AskInputForm` coding): `{mode, value, placeholder?}`.
mod input_wire {
    use super::{AskInputForm, AskInputMode, Deserialize, Deserializer, Serialize, Serializer, de};

    #[derive(Serialize, Deserialize)]
    struct Wire {
        mode: String,
        value: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        placeholder: Option<String>,
    }

    #[expect(clippy::ref_option, reason = "serde's `with` hands the field by reference")]
    pub(super) fn serialize<S: Serializer>(input: &Option<AskInputForm>, serializer: S) -> Result<S::Ok, S::Error> {
        input
            .as_ref()
            .map(|input| Wire {
                mode: input.mode.wire_name().to_owned(),
                value: input.value.clone(),
                placeholder: input.placeholder.clone(),
            })
            .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<AskInputForm>, D::Error> {
        Option::<Wire>::deserialize(deserializer)?
            .map(|wire| {
                let mode = AskInputMode::from_wire(&wire.mode)
                    .ok_or_else(|| de::Error::custom(format!("unknown input mode: {}", wire.mode)))?;
                Ok(AskInputForm {
                    mode,
                    value: wire.value,
                    placeholder: wire.placeholder,
                })
            })
            .transpose()
    }
}

/// Serde for an ask's choice list (upstream's `AskChoiceForm` coding): `{choices, multiple, checked, other?}`. A list
/// from an origin without the free-form row carries no `other`, which reads as none.
mod choices_wire {
    use super::{AskChoice, AskChoiceForm, Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Serialize, Deserialize)]
    struct Wire {
        choices: Vec<AskChoice>,
        multiple: bool,
        checked: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        other: Option<String>,
    }

    #[expect(clippy::ref_option, reason = "serde's `with` hands the field by reference")]
    pub(super) fn serialize<S: Serializer>(list: &Option<AskChoiceForm>, serializer: S) -> Result<S::Ok, S::Error> {
        list.as_ref()
            .map(|list| Wire {
                choices: list.choices.clone(),
                multiple: list.multiple,
                checked: list.checked.clone(),
                other: list.other.clone(),
            })
            .serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<AskChoiceForm>, D::Error> {
        Ok(Option::<Wire>::deserialize(deserializer)?.map(|wire| AskChoiceForm {
            choices: wire.choices,
            multiple: wire.multiple,
            checked: wire.checked,
            other: wire.other,
        }))
    }
}

/// An ask the origin hands its presenter to draw. `owner` is the presenter generation it was handed under, and every
/// answer echoes it, so one arriving after a handback is recognisably stale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationAsk {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub buttons: Vec<AskButton>,
    #[serde(default, rename = "defaultID", skip_serializing_if = "Option::is_none")]
    pub default_id: Option<String>,
    #[serde(default, rename = "destructiveID", skip_serializing_if = "Option::is_none")]
    pub destructive_id: Option<String>,
    #[serde(with = "style_wire")]
    pub style: AskStyle,
    #[serde(with = "align_wire")]
    pub align: AskAlignment,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<i64>,
    /// The origin's pane the dialog covers; `None` for the whole session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<PresentationPane>,
    pub owner: i64,
    /// The ask's text field; a presenter without `ask.form` never receives one.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "input_wire")]
    pub input: Option<AskInputForm>,
    /// The ask's choice list; a presenter without `ask.form` never receives one.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "choices_wire")]
    pub choices: Option<AskChoiceForm>,
    /// The button that resolves `escaped`.
    #[serde(default, rename = "cancelID", skip_serializing_if = "Option::is_none")]
    pub cancel_id: Option<String>,
}

impl PresentationAsk {
    /// `ask` as handed to the presenter of generation `owner`.
    #[must_use]
    pub fn new(ask: &PendingAsk, pane: Option<PresentationPane>, owner: i64) -> Self {
        let (input, choices) = match &ask.form {
            Some(AskForm::Input(input)) => (Some(input.clone()), None),
            Some(AskForm::Choices(list)) => (None, Some(list.clone())),
            None => (None, None),
        };
        Self {
            id: ask.id.clone(),
            title: ask.title.clone(),
            message: ask.message.clone(),
            buttons: ask.buttons.clone(),
            default_id: ask.default_id.clone(),
            destructive_id: ask.destructive_id.clone(),
            style: ask.style,
            align: ask.align,
            width: ask.width,
            pane,
            owner,
            input,
            choices,
            cancel_id: ask.cancel_id.clone(),
        }
    }

    /// The form the request carries, the text field first should a malformed frame carry both.
    #[must_use]
    pub fn form(&self) -> Option<AskForm> {
        self.input
            .clone()
            .map(AskForm::Input)
            .or_else(|| self.choices.clone().map(AskForm::Choices))
    }

    /// Whether the presenter must list `ask.form` to show this ask as the origin means it (upstream's
    /// `needsFormSupport`).
    #[must_use]
    pub fn needs_form_support(&self) -> bool {
        self.input.is_some() || self.choices.is_some() || self.cancel_id.is_some()
    }

    /// Whether the presenter must also list `ask.other` to show the choice list's free-form row (upstream's
    /// `needsOtherSupport`).
    #[must_use]
    pub fn needs_other_support(&self) -> bool {
        self.choices.as_ref().is_some_and(|list| list.other.is_some())
    }

    /// Whether the request, and the largest answer it can produce, each fit one stream frame, measured through
    /// [`encode`] with the widest counters (upstream's `fitsFrames`). A text answer is tried with every button and the
    /// text cap of `"` characters, which escape to twice their bytes, the worst the allowed characters reach; a
    /// selection alone names only ids the request already carried, but a checkbox list's free-form text rides beside
    /// every id.
    #[must_use]
    pub fn fits_frames(&self) -> bool {
        if !fits(Body::AskRequest(self.clone())) {
            return false;
        }
        let text = "\"".repeat(AskInputForm::MAX_TEXT_BYTES);
        let answer = |button: &AskButton, selected: Option<Vec<String>>| {
            Body::AskResolve(PresentationAskAnswer {
                id: self.id.clone(),
                owner: self.owner,
                button: Some(button.id.clone()),
                text: Some(text.clone()),
                selected,
            })
        };
        if self.input.is_some() {
            return self.buttons.iter().all(|button| fits(answer(button, None)));
        }
        let Some(list) = self.choices.as_ref().filter(|list| list.other.is_some()) else {
            return true;
        };
        let selected: Vec<String> = if list.multiple {
            list.choices.iter().map(|choice| choice.id.clone()).collect()
        } else {
            Vec::new()
        };
        self.buttons
            .iter()
            .all(|button| fits(answer(button, Some(selected.clone()))))
    }
}

/// Whether `body` encodes within [`MAX_FRAME_BYTES`] with the widest counters.
fn fits(body: Body) -> bool {
    encode(&PresentationFrame {
        generation: i64::MAX,
        rev: i64::MAX,
        body,
    })
    .is_ok()
}

/// The presenter's answer: a button id, or `None` for a dismissal. Only the id travels for the button; the origin
/// derives the label and index from the buttons it stored and checks `text` and `selected` against the form it stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationAskAnswer {
    pub id: String,
    pub owner: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<String>,
    /// The text field's content, or the selected free-form row's text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A choice list's selected ids, in the caller's order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected: Option<Vec<String>>,
}

/// Names one handed-over ask, for a refusal or a dismissal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationAskRef {
    pub id: String,
    pub owner: i64,
}

/// Asks the presenter to show an overlay whose program runs on the origin: the viewer opens a surface running the
/// job's helper over ssh. Nothing of the command or its environment travels; the helper gets those when it claims the
/// job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationOverlay {
    pub job: String,
    /// The origin's pane the overlay covers; `None` for the session-wide slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<PresentationPane>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_percent: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
    pub follow: bool,
    pub wait: bool,
}

/// Names one remote overlay job, with a size for a resize.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationOverlayChange {
    pub job: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_percent: Option<i64>,
}

impl PresentationOverlayChange {
    #[must_use]
    pub fn job(job: impl Into<String>) -> Self {
        Self {
            job: job.into(),
            size_percent: None,
        }
    }
}

/// A handed-over page's full source on the origin (upstream's `PresentationPageSource`): a file the presenter fetches
/// over the page channel, with the grant bounding which files it may ask for, or a complete URL it loads itself. The
/// wire is Swift's synthesized enum coding: `{"file": {"path", "grantRoot"?}}` or `{"url": {"_0": <address>}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresentationPageSource {
    File {
        path: String,
        #[serde(default, rename = "grantRoot", skip_serializing_if = "Option::is_none")]
        grant_root: Option<String>,
    },
    Url {
        #[serde(rename = "_0")]
        address: String,
    },
}

/// Asks the presenter to show a page the origin opened (upstream's `PresentationPage`). `page` is the origin's page id,
/// which the presenter never reuses as its own; `token` claims the page channel once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[expect(
    clippy::struct_excessive_bools,
    reason = "upstream's frame: the page's five independent flags and follow, each its own wire boolean"
)]
pub struct PresentationPage {
    pub page: String,
    pub token: String,
    pub source: PresentationPageSource,
    /// The display text the identity strip shows, never used to load anything.
    pub identity: String,
    pub chromeless: bool,
    pub javascript: bool,
    pub navigation: bool,
    pub browse: bool,
    pub persistent: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_color: Option<String>,
    /// The origin's pane the page covers; `None` for the session-wide slot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pane: Option<PresentationPane>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_percent: Option<i64>,
    pub follow: bool,
}

/// Names one handed-over page by the origin's id, with the presenter's reason when it refuses the page (upstream's
/// `PresentationPageChange`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PresentationPageChange {
    pub page: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl PresentationPageChange {
    #[must_use]
    pub fn page(page: impl Into<String>) -> Self {
        Self {
            page: page.into(),
            reason: None,
        }
    }
}

/// What the origin asks of a handed-over page (upstream's `PresentationPageAction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PresentationPageAction {
    Hide,
    Show,
    ReloadOriginal,
    ReloadCurrent,
    Back,
    Forward,
    Browser,
    Resize,
}

/// One origin request for a handed-over page (upstream's `PresentationPageCommand`). `seq` grows per page, and the
/// presenter echoes the last one it applied in [`PresentationPageState::applied_seq`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationPageCommand {
    pub page: String,
    pub seq: i64,
    pub action: PresentationPageAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_percent: Option<i64>,
}

/// What a handed-over page shows on the presenter (upstream's `PresentationPageState`). `address` is the origin's
/// logical address, never the presenter's own scheme URL. [`PresentationPageState::new`] bounds the strings the page
/// writes, so a long title cannot cost a frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PresentationPageState {
    pub page: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    pub load: HtmlLoadState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub hidden: bool,
    pub applied_seq: i64,
}

/// What a page shows and where its history goes, as a [`PresentationPageState`] carries it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PageShown {
    pub title: Option<String>,
    pub address: Option<String>,
    pub load: HtmlLoadState,
    pub error: Option<String>,
    pub can_go_back: bool,
    pub can_go_forward: bool,
}

impl PresentationPageState {
    pub const MAX_TITLE: usize = 1024;
    pub const MAX_ADDRESS: usize = 8192;
    pub const MAX_ERROR: usize = 1024;

    /// The state of page `page`, each string cut to its bound in characters (upstream's `init`).
    #[must_use]
    pub fn new(page: impl Into<String>, shown: PageShown, hidden: bool, applied_seq: i64) -> Self {
        let bounded = |text: Option<String>, limit: usize| text.map(|text| text.chars().take(limit).collect());
        Self {
            page: page.into(),
            title: bounded(shown.title, Self::MAX_TITLE),
            address: bounded(shown.address, Self::MAX_ADDRESS),
            load: shown.load,
            error: bounded(shown.error, Self::MAX_ERROR),
            can_go_back: shown.can_go_back,
            can_go_forward: shown.can_go_forward,
            hidden,
            applied_seq,
        }
    }
}

/// The origin's complete pane membership and arrangement, independent of surface realization (upstream's
/// `PresentationLayout`). It decodes lossily: a bad layout reads as [`PresentationLayout::invalid`] rather than
/// discarding the rest of a snapshot or disconnecting its stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PresentationLayout {
    #[serde(serialize_with = "serialize_uuids")]
    pub panes: Vec<Uuid>,
    #[serde(serialize_with = "serialize_uuid")]
    pub primary: Uuid,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub axis: Option<String>,
    pub shown: bool,
}

fn serialize_uuids<S: Serializer>(ids: &[Uuid], serializer: S) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(ids.iter().map(|id| uuid_string(*id)))
}

impl PresentationLayout {
    /// What an undecodable layout reads as: no panes, so it is never valid.
    #[must_use]
    pub fn invalid() -> Self {
        Self {
            panes: Vec::new(),
            primary: Uuid::nil(),
            axis: None,
            shown: false,
        }
    }

    /// One or two distinct panes including the primary; a single pane is never shown split, two need a known axis.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let distinct = self.panes.len() != 2 || self.panes[0] != self.panes[1];
        (1..=2).contains(&self.panes.len())
            && distinct
            && self.panes.contains(&self.primary)
            && if self.panes.len() == 1 {
                !self.shown
            } else {
                self.axis
                    .as_deref()
                    .is_some_and(|axis| crate::model::SplitAxis::from_wire(axis).is_some())
            }
    }

    /// The layout `value` holds, or [`PresentationLayout::invalid`].
    fn lossy(value: &Value) -> Self {
        Self::decoded(value).unwrap_or_else(Self::invalid)
    }

    fn decoded(value: &Value) -> Option<Self> {
        let object = value.as_object()?;
        let panes = object
            .get("panes")?
            .as_array()?
            .iter()
            .map(|pane| pane.as_str().and_then(parse_uuid))
            .collect::<Option<Vec<_>>>()?;
        let primary = object.get("primary")?.as_str().and_then(parse_uuid)?;
        let axis = match object.get("axis") {
            None | Some(Value::Null) => None,
            Some(axis) => Some(axis.as_str()?.to_owned()),
        };
        let shown = object.get("shown")?.as_bool()?;
        Some(Self {
            panes,
            primary,
            axis,
            shown,
        })
    }
}

impl<'de> Deserialize<'de> for PresentationLayout {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Self::lossy(&Value::deserialize(deserializer)?))
    }
}

/// How often the origin respawned each of a session's panes, by daemon name, in one origin app lifetime (upstream's
/// `PresentationRespawns`): `epoch` changes on every launch, so a counter that restarted is never read as older.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PresentationRespawns {
    pub epoch: String,
    pub generations: std::collections::BTreeMap<String, i64>,
}

/// The replaceable state a subscriber starts from.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct PresentationSnapshot {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<PresentationStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hud: Option<PresentationHud>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<PresentationLayout>,
    /// `None` from an origin that predates parent respawn.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub respawns: Option<PresentationRespawns>,
}

/// What one frame carries (upstream's `PresentationFrame.Body`).
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    Hello(PresentationHello),
    Ping,
    Ack,
    Snapshot(PresentationSnapshot),
    Status(Option<PresentationStatus>),
    Context(Option<String>),
    Layout(PresentationLayout),
    Hud(Option<PresentationHud>),
    Notify(PresentationNotify),
    Sound(PresentationSound),
    /// A viewer asking to be the session's sole presenter; sent only after the origin's hello offered it.
    PresenterAcquire,
    PresenterGranted,
    /// The role is held by another viewer; this one stays a mirror until it reconnects.
    PresenterRefused,
    AskRequest(PresentationAsk),
    AskResolve(PresentationAskAnswer),
    /// The presenter cannot show the ask, so the origin takes it back.
    AskRejected(PresentationAskRef),
    /// The ask ended on the origin; the presenter takes its dialog down without answering.
    AskDismiss(PresentationAskRef),
    OverlayRequest(PresentationOverlay),
    /// The presenter cannot show the overlay; the origin fails the job without launching anything.
    OverlayRejected(PresentationOverlayChange),
    OverlayClose(PresentationOverlayChange),
    OverlayResize(PresentationOverlayChange),
    /// The viewer closed its surface, which frees a slot held only for a `--wait` surface.
    OverlayClosed(PresentationOverlayChange),
    PageRequest(PresentationPage),
    /// The presenter cannot show the page, so the origin ends it without waiting for a claim.
    PageRejected(PresentationPageChange),
    PageClose(PresentationPageChange),
    /// The presenter took the page down, so the origin ends it, keeping an answer it recorded.
    PageClosed(PresentationPageChange),
    PageCommand(PresentationPageCommand),
    PageState(PresentationPageState),
    /// The origin respawned a pane's shell under the same daemon name; carries that pane's new count alone.
    Respawn(PresentationRespawns),
    /// A kind this build does not speak, kept with its place in the order.
    Unknown(String),
}

impl Body {
    /// The frame's `kind`.
    #[must_use]
    pub fn kind(&self) -> &str {
        match self {
            Self::Hello(_) => "hello",
            Self::Ping => "ping",
            Self::Ack => "ack",
            Self::Snapshot(_) => "snapshot",
            Self::Status(_) => "status",
            Self::Context(_) => "context",
            Self::Layout(_) => "layout",
            Self::Hud(_) => "hud",
            Self::Notify(_) => "notify",
            Self::Sound(_) => "sound",
            Self::PresenterAcquire => "presenter.acquire",
            Self::PresenterGranted => "presenter.granted",
            Self::PresenterRefused => "presenter.refused",
            Self::AskRequest(_) => "ask.request",
            Self::AskResolve(_) => "ask.resolve",
            Self::AskRejected(_) => "ask.rejected",
            Self::AskDismiss(_) => "ask.dismiss",
            Self::OverlayRequest(_) => "overlay.request",
            Self::OverlayRejected(_) => "overlay.rejected",
            Self::OverlayClose(_) => "overlay.close",
            Self::OverlayResize(_) => "overlay.resize",
            Self::OverlayClosed(_) => "overlay.closed",
            Self::PageRequest(_) => "page.request",
            Self::PageRejected(_) => "page.rejected",
            Self::PageClose(_) => "page.close",
            Self::PageClosed(_) => "page.closed",
            Self::PageCommand(_) => "page.command",
            Self::PageState(_) => "page.state",
            Self::Respawn(_) => "pane.respawn",
            Self::Unknown(kind) => kind,
        }
    }

    /// The payload's key and value, `None` for a kind without one (or an absent optional payload).
    fn payload(&self) -> serde_json::Result<Option<(&'static str, Value)>> {
        let entry = |key: &'static str, value: serde_json::Result<Value>| value.map(|value| Some((key, value)));
        match self {
            Self::Hello(hello) => entry("hello", serde_json::to_value(hello)),
            Self::Snapshot(snapshot) => entry("snapshot", serde_json::to_value(snapshot)),
            Self::Status(Some(status)) => entry("status", serde_json::to_value(status)),
            Self::Context(Some(context)) => entry("context", serde_json::to_value(context)),
            Self::Layout(layout) => entry("layout", serde_json::to_value(layout)),
            Self::Hud(Some(hud)) => entry("hud", serde_json::to_value(hud)),
            Self::Notify(notify) => entry("notify", serde_json::to_value(notify)),
            Self::Sound(sound) => entry("sound", serde_json::to_value(sound)),
            Self::AskRequest(ask) => entry("ask", serde_json::to_value(ask)),
            Self::AskResolve(answer) => entry("ask", serde_json::to_value(answer)),
            Self::AskRejected(reference) | Self::AskDismiss(reference) => entry("ask", serde_json::to_value(reference)),
            Self::OverlayRequest(overlay) => entry("overlay", serde_json::to_value(overlay)),
            Self::OverlayRejected(change)
            | Self::OverlayClose(change)
            | Self::OverlayResize(change)
            | Self::OverlayClosed(change) => entry("overlay", serde_json::to_value(change)),
            Self::PageRequest(page) => entry("page", serde_json::to_value(page)),
            Self::PageRejected(change) | Self::PageClose(change) | Self::PageClosed(change) => {
                entry("page", serde_json::to_value(change))
            }
            Self::PageCommand(command) => entry("page", serde_json::to_value(command)),
            Self::PageState(state) => entry("page", serde_json::to_value(state)),
            Self::Respawn(respawns) => entry("respawn", serde_json::to_value(respawns)),
            Self::Status(None)
            | Self::Context(None)
            | Self::Hud(None)
            | Self::Ping
            | Self::Ack
            | Self::PresenterAcquire
            | Self::PresenterGranted
            | Self::PresenterRefused
            | Self::Unknown(_) => Ok(None),
        }
    }
}

/// One line of the presentation stream.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationFrame {
    pub generation: i64,
    pub rev: i64,
    pub body: Body,
}

/// Why a line could not become a frame, or a frame a line (upstream's `PresentationCodec.FrameError`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FrameError {
    /// The line or the encoded frame is this many bytes, over [`MAX_FRAME_BYTES`].
    Oversize(usize),
    /// What was wrong with it.
    Malformed(String),
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Oversize(bytes) => write!(f, "frame of {bytes} bytes is over the limit"),
            Self::Malformed(detail) => write!(f, "malformed frame: {detail}"),
        }
    }
}

impl std::error::Error for FrameError {}

/// Encodes `frame` as one line, its newline included.
///
/// # Errors
///
/// The frame cannot be encoded, or its line would reach [`MAX_FRAME_BYTES`].
pub fn encode(frame: &PresentationFrame) -> Result<Vec<u8>, FrameError> {
    let malformed = |error: serde_json::Error| FrameError::Malformed(error.to_string());
    let mut object = Map::new();
    object.insert("kind".to_owned(), Value::from(frame.body.kind()));
    object.insert("gen".to_owned(), Value::from(frame.generation));
    object.insert("rev".to_owned(), Value::from(frame.rev));
    if let Some((key, value)) = frame.body.payload().map_err(malformed)? {
        object.insert(key.to_owned(), value);
    }
    let mut line = serde_json::to_vec(&Value::Object(object)).map_err(malformed)?;
    if line.len() >= MAX_FRAME_BYTES {
        return Err(FrameError::Oversize(line.len()));
    }
    line.push(b'\n');
    Ok(line)
}

/// Decodes one line, without its newline.
///
/// # Errors
///
/// The line is over [`MAX_FRAME_BYTES`], or not a frame: what was wrong rides [`FrameError::Malformed`].
pub fn decode(line: &[u8]) -> Result<PresentationFrame, FrameError> {
    if line.len() > MAX_FRAME_BYTES {
        return Err(FrameError::Oversize(line.len()));
    }
    let value: Value = serde_json::from_slice(line).map_err(|error| FrameError::Malformed(error.to_string()))?;
    let Value::Object(mut object) = value else {
        return Err(FrameError::Malformed("a frame is a JSON object".to_owned()));
    };
    let generation = integer(&object, "gen")?;
    let rev = integer(&object, "rev")?;
    let kind = match object.get("kind") {
        Some(Value::String(kind)) => kind.clone(),
        Some(_) => return Err(FrameError::Malformed("kind: expected a string".to_owned())),
        None => return Err(FrameError::Malformed("missing kind".to_owned())),
    };
    let body = body(&kind, &mut object)?;
    Ok(PresentationFrame { generation, rev, body })
}

fn integer(object: &Map<String, Value>, key: &str) -> Result<i64, FrameError> {
    match object.get(key) {
        Some(value) => value
            .as_i64()
            .ok_or_else(|| FrameError::Malformed(format!("{key}: expected an integer"))),
        None => Err(FrameError::Malformed(format!("missing {key}"))),
    }
}

/// The payload under `key`, which must be there.
fn required<T: DeserializeOwned>(object: &mut Map<String, Value>, key: &str) -> Result<T, FrameError> {
    let value = object
        .remove(key)
        .ok_or_else(|| FrameError::Malformed(format!("missing {key}")))?;
    serde_json::from_value(value).map_err(|error| FrameError::Malformed(format!("{key}: {error}")))
}

/// The payload under `key`, absent or `null` reading as `None`.
fn optional<T: DeserializeOwned>(object: &mut Map<String, Value>, key: &str) -> Result<Option<T>, FrameError> {
    match object.remove(key) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => serde_json::from_value(value)
            .map(Some)
            .map_err(|error| FrameError::Malformed(format!("{key}: {error}"))),
    }
}

fn body(kind: &str, object: &mut Map<String, Value>) -> Result<Body, FrameError> {
    Ok(match kind {
        "hello" => Body::Hello(required(object, "hello")?),
        "ping" => Body::Ping,
        "ack" => Body::Ack,
        "snapshot" => Body::Snapshot(required(object, "snapshot")?),
        "status" => Body::Status(optional(object, "status")?),
        "context" => Body::Context(optional(object, "context")?),
        "layout" => Body::Layout(
            object
                .get("layout")
                .map_or_else(PresentationLayout::invalid, PresentationLayout::lossy),
        ),
        "hud" => Body::Hud(optional(object, "hud")?),
        "notify" => Body::Notify(required(object, "notify")?),
        "sound" => Body::Sound(required(object, "sound")?),
        "presenter.acquire" => Body::PresenterAcquire,
        "presenter.granted" => Body::PresenterGranted,
        "presenter.refused" => Body::PresenterRefused,
        "ask.request" => Body::AskRequest(required(object, "ask")?),
        "ask.resolve" => Body::AskResolve(required(object, "ask")?),
        "ask.rejected" => Body::AskRejected(required(object, "ask")?),
        "ask.dismiss" => Body::AskDismiss(required(object, "ask")?),
        "overlay.request" => Body::OverlayRequest(required(object, "overlay")?),
        "overlay.rejected" => Body::OverlayRejected(required(object, "overlay")?),
        "overlay.close" => Body::OverlayClose(required(object, "overlay")?),
        "overlay.resize" => Body::OverlayResize(required(object, "overlay")?),
        "overlay.closed" => Body::OverlayClosed(required(object, "overlay")?),
        "page.request" => Body::PageRequest(required(object, "page")?),
        "page.rejected" => Body::PageRejected(required(object, "page")?),
        "page.close" => Body::PageClose(required(object, "page")?),
        "page.closed" => Body::PageClosed(required(object, "page")?),
        "page.command" => Body::PageCommand(required(object, "page")?),
        "page.state" => Body::PageState(required(object, "page")?),
        "pane.respawn" => Body::Respawn(required(object, "respawn")?),
        other => Body::Unknown(other.to_owned()),
    })
}

/// The version both sides speak, or `None` when the peer's is not a version at all.
#[must_use]
pub fn negotiated_version(ours: i64, theirs: i64) -> Option<i64> {
    (ours >= 1 && theirs >= 1).then(|| ours.min(theirs))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn pane() -> PresentationPane {
        PresentationPane::Identity(Uuid::parse_str("11111111-2222-3333-4444-555555555555").unwrap())
    }

    fn status() -> PresentationStatus {
        PresentationStatus {
            status: AgentStatus::Blocked,
            blink: true,
            color: Some("#ff8800".to_owned()),
            shape: Some(StatusShape::Diamond),
            pane: Some(pane()),
            changed_at: Some(1_789_000_000.0),
        }
    }

    fn hud() -> PresentationHud {
        let mut spec = HudSpec::new("deploying");
        spec.hide_after = Some(10.0);
        PresentationHud {
            spec,
            pane: None,
            generation: 4,
            remaining: Some(7.5),
        }
    }

    fn pane_hud() -> PresentationHud {
        PresentationHud {
            spec: HudSpec::new("deploying"),
            pane: Some(pane()),
            generation: 5,
            remaining: None,
        }
    }

    fn ask() -> PendingAsk {
        PendingAsk {
            id: "a1".to_owned(),
            title: "deploy?".to_owned(),
            message: Some("to prod".to_owned()),
            buttons: vec![AskButton {
                id: "y".to_owned(),
                label: "Yes".to_owned(),
                hotkey: Some("y".to_owned()),
            }],
            default_id: Some("y".to_owned()),
            destructive_id: None,
            style: AskStyle::Gui,
            align: AskAlignment::Center,
            width: Some(40),
            anchor: None,
            form: None,
            cancel_id: None,
        }
    }

    fn change(size: Option<i64>) -> PresentationOverlayChange {
        PresentationOverlayChange {
            job: "j1".to_owned(),
            size_percent: size,
        }
    }

    fn reference() -> PresentationAskRef {
        PresentationAskRef {
            id: "a1".to_owned(),
            owner: 3,
        }
    }

    /// Upstream's `PresentationFramesTests.frames`.
    fn bodies() -> Vec<Body> {
        let mut bodies = state_bodies();
        bodies.extend(work_bodies());
        bodies
    }

    /// The hello, keepalive and presentation-state frames.
    fn state_bodies() -> Vec<Body> {
        vec![
            Body::Hello(PresentationHello {
                version: 1,
                kinds: vec!["status".to_owned(), "hud".to_owned(), "notify".to_owned()],
                mode: PresentationMode::Mirror,
            }),
            Body::Ping,
            Body::Ack,
            Body::Snapshot(PresentationSnapshot {
                status: Some(status()),
                hud: Some(hud()),
                ..PresentationSnapshot::default()
            }),
            Body::Snapshot(PresentationSnapshot::default()),
            Body::Status(Some(status())),
            Body::Status(None),
            Body::Status(Some(PresentationStatus {
                status: AgentStatus::Active,
                blink: false,
                color: None,
                shape: None,
                pane: Some(PresentationPane::Scratch),
                changed_at: None,
            })),
            Body::Hud(Some(hud())),
            Body::Hud(None),
            Body::Hud(Some(pane_hud())),
            Body::Snapshot(PresentationSnapshot {
                hud: Some(pane_hud()),
                ..PresentationSnapshot::default()
            }),
            Body::Snapshot(PresentationSnapshot {
                respawns: Some(respawns()),
                ..PresentationSnapshot::default()
            }),
            Body::Respawn(respawns()),
            Body::Notify(PresentationNotify {
                title: "build".to_owned(),
                body: "done".to_owned(),
                pane: Some(pane()),
                source: "control".to_owned(),
            }),
        ]
    }

    /// The presenter's frames and the work handed to it.
    fn work_bodies() -> Vec<Body> {
        vec![
            Body::Hello(PresentationHello {
                version: 1,
                kinds: vec!["status".to_owned()],
                mode: PresentationMode::Presenter,
            }),
            Body::PresenterAcquire,
            Body::PresenterGranted,
            Body::PresenterRefused,
            Body::AskRequest(PresentationAsk::new(&ask(), Some(pane()), 3)),
            Body::AskResolve(answer("a1", Some("y"), None, None)),
            Body::AskResolve(answer("a1", None, None, None)),
            Body::AskRejected(reference()),
            Body::AskDismiss(reference()),
            Body::OverlayRequest(PresentationOverlay {
                job: "j1".to_owned(),
                pane: Some(pane()),
                size_percent: Some(60),
                background_color: Some("#102030".to_owned()),
                follow: true,
                wait: true,
            }),
            Body::OverlayRequest(PresentationOverlay {
                job: "j1".to_owned(),
                pane: None,
                size_percent: None,
                background_color: None,
                follow: false,
                wait: false,
            }),
            Body::OverlayRejected(change(None)),
            Body::OverlayClose(change(None)),
            Body::OverlayResize(change(Some(40))),
            Body::OverlayClosed(change(None)),
            Body::Context(Some("PR #517".to_owned())),
            Body::Context(None),
            Body::Snapshot(PresentationSnapshot {
                context: Some("PR #517".to_owned()),
                ..PresentationSnapshot::default()
            }),
            Body::Layout(PresentationLayout {
                panes: vec![Uuid::new_v4(), Uuid::new_v4()],
                primary: Uuid::nil(),
                axis: Some("vertical".to_owned()),
                shown: true,
            }),
            Body::Sound(PresentationSound {
                name: Some("Glass".to_owned()),
            }),
            Body::Sound(PresentationSound { name: None }),
            Body::AskRequest(PresentationAsk::new(&form_ask(), None, 3)),
            Body::AskResolve(answer("a2", Some("ok"), Some(" mine "), Some(&["a"]))),
            Body::AskRequest(PresentationAsk::new(
                &PendingAsk {
                    form: Some(AskForm::Input(AskInputForm {
                        mode: AskInputMode::Multiline,
                        value: "v".to_owned(),
                        placeholder: Some("name".to_owned()),
                    })),
                    cancel_id: Some("y".to_owned()),
                    ..ask()
                },
                Some(pane()),
                3,
            )),
            Body::AskResolve(answer("a1", Some("y"), Some("é \"q\"\nb"), None)),
        ]
        .into_iter()
        .chain(page_bodies())
        .collect()
    }

    fn page_request(id: &str, source: PresentationPageSource, pane: Option<PresentationPane>) -> PresentationPage {
        PresentationPage {
            page: id.to_owned(),
            token: format!("t-{id}"),
            source,
            identity: "index.html".to_owned(),
            chromeless: pane.is_some(),
            javascript: true,
            navigation: pane.is_none(),
            browse: false,
            persistent: pane.is_none(),
            background_color: pane.map(|_| "#102030".to_owned()),
            pane,
            size_percent: pane.is_none().then_some(80),
            follow: pane.is_none(),
        }
    }

    fn respawns() -> PresentationRespawns {
        PresentationRespawns {
            epoch: "e1".to_owned(),
            generations: [("ngterm-a".to_owned(), 2)].into(),
        }
    }

    fn page_state(hidden: bool) -> PresentationPageState {
        let shown = PageShown {
            title: Some("Review".to_owned()),
            address: Some("/Users/u/docs/b.html".to_owned()),
            load: HtmlLoadState::Loaded,
            error: None,
            can_go_back: true,
            can_go_forward: false,
        };
        PresentationPageState::new("p1", shown, hidden, 8)
    }

    /// Upstream's page frames in `PresentationFramesTests.frames`.
    fn page_bodies() -> Vec<Body> {
        let url = PresentationPageSource::Url {
            address: "https://example.com/review/42?token=abc&x=1#top".to_owned(),
        };
        let granted = PresentationPageSource::File {
            path: "/Users/u/docs/index.html".to_owned(),
            grant_root: Some("/Users/u/docs".to_owned()),
        };
        let bare = PresentationPageSource::File {
            path: "/tmp/r.html".to_owned(),
            grant_root: None,
        };
        let command = |seq: i64, action: PresentationPageAction, size_percent: Option<i64>| {
            Body::PageCommand(PresentationPageCommand {
                page: "p1".to_owned(),
                seq,
                action,
                size_percent,
            })
        };
        vec![
            Body::PageRequest(page_request("p1", url, None)),
            Body::PageRequest(page_request("p2", granted, Some(pane()))),
            Body::PageRequest(page_request("p3", bare, None)),
            Body::PageRejected(PresentationPageChange {
                page: "p1".to_owned(),
                reason: Some("slot taken".to_owned()),
            }),
            Body::PageClose(PresentationPageChange::page("p1")),
            Body::PageClosed(PresentationPageChange::page("p1")),
            command(7, PresentationPageAction::Resize, Some(50)),
            command(8, PresentationPageAction::ReloadCurrent, None),
            Body::PageState(page_state(true)),
            Body::PageState(PresentationPageState::new("p1", PageShown::default(), false, 0)),
        ]
    }

    /// Upstream's form frame: a checkbox list with a free-form row.
    fn form_ask() -> PendingAsk {
        PendingAsk {
            id: "a2".to_owned(),
            title: "pick".to_owned(),
            message: None,
            buttons: vec![button("ok")],
            default_id: None,
            destructive_id: None,
            style: AskStyle::Terminal,
            align: AskAlignment::Right,
            width: None,
            anchor: None,
            form: Some(AskForm::Choices(AskChoiceForm {
                multiple: true,
                other: Some("Other".to_owned()),
                ..AskChoiceForm::new(vec![choice("a")])
            })),
            cancel_id: None,
        }
    }

    fn button(id: &str) -> AskButton {
        AskButton {
            id: id.to_owned(),
            label: id.to_uppercase(),
            hotkey: None,
        }
    }

    fn choice(id: &str) -> AskChoice {
        AskChoice {
            id: id.to_owned(),
            label: id.to_uppercase(),
            subtitle: None,
        }
    }

    fn answer(id: &str, button: Option<&str>, text: Option<&str>, selected: Option<&[&str]>) -> PresentationAskAnswer {
        PresentationAskAnswer {
            id: id.to_owned(),
            owner: 3,
            button: button.map(str::to_owned),
            text: text.map(str::to_owned),
            selected: selected.map(|ids| ids.iter().map(|id| (*id).to_owned()).collect()),
        }
    }

    /// An ask titled `t` answering with `buttons` and showing `form`.
    fn sized_ask(buttons: Vec<AskButton>, form: AskForm) -> PresentationAsk {
        PresentationAsk::new(
            &PendingAsk {
                id: "a".to_owned(),
                title: "t".to_owned(),
                message: None,
                buttons,
                form: Some(form),
                ..form_ask()
            },
            None,
            1,
        )
    }

    /// The encoded length of `body` under the counters `counter`.
    fn encoded_len(body: Body, counter: i64) -> usize {
        encode(&PresentationFrame {
            generation: counter,
            rev: counter,
            body,
        })
        .unwrap()
        .len()
    }

    fn widest_text() -> String {
        "\"".repeat(AskInputForm::MAX_TEXT_BYTES)
    }

    #[test]
    fn every_frame_survives_a_round_trip() {
        for (rev, body) in (0..).zip(bodies()) {
            let frame = PresentationFrame {
                generation: 1,
                rev,
                body,
            };
            let line = encode(&frame).unwrap();
            assert_eq!(line.last(), Some(&b'\n'));
            assert!(!line[..line.len() - 1].contains(&b'\n'));
            assert_eq!(decode(&line[..line.len() - 1]).unwrap(), frame, "{}", frame.body.kind());
        }
    }

    /// The page frames' keys, as Swift's synthesized coding spells them.
    #[test]
    fn the_page_frames_spell_their_keys_as_upstream() {
        let encoded = |body: Body| -> Value {
            let line = encode(&PresentationFrame {
                generation: 1,
                rev: 1,
                body,
            })
            .unwrap();
            serde_json::from_slice(&line).unwrap()
        };
        let url = PresentationPageSource::Url {
            address: "https://example.com/".to_owned(),
        };
        let request = encoded(Body::PageRequest(page_request("p1", url, None)));
        assert_eq!(request["kind"], "page.request");
        assert_eq!(
            request["page"],
            json!({"page": "p1", "token": "t-p1", "source": {"url": {"_0": "https://example.com/"}},
                "identity": "index.html", "chromeless": false, "javascript": true, "navigation": true,
                "browse": false, "persistent": true, "sizePercent": 80, "follow": true})
        );
        let file = PresentationPageSource::File {
            path: "/tmp/r.html".to_owned(),
            grant_root: None,
        };
        let request = encoded(Body::PageRequest(page_request("p2", file, Some(pane()))));
        assert_eq!(request["page"]["source"], json!({"file": {"path": "/tmp/r.html"}}));
        assert_eq!(request["page"]["pane"], "11111111-2222-3333-4444-555555555555");
        assert_eq!(request["page"]["backgroundColor"], "#102030");
        let command = encoded(Body::PageCommand(PresentationPageCommand {
            page: "p1".to_owned(),
            seq: 3,
            action: PresentationPageAction::ReloadOriginal,
            size_percent: None,
        }));
        assert_eq!(
            command["page"],
            json!({"page": "p1", "seq": 3, "action": "reloadOriginal"})
        );
        let state = encoded(Body::PageState(page_state(false)));
        assert_eq!(
            state["page"],
            json!({"page": "p1", "title": "Review", "address": "/Users/u/docs/b.html", "load": "loaded",
                "canGoBack": true, "canGoForward": false, "hidden": false, "appliedSeq": 8})
        );
        let rejected = encoded(Body::PageRejected(PresentationPageChange::page("p1")));
        assert_eq!(rejected["page"], json!({"page": "p1"}));
    }

    /// Upstream's `PresentationFramesTests.pageStateBoundsThePagesOwnStrings`.
    #[test]
    fn page_state_bounds_the_pages_own_strings() {
        let long = "x".repeat(100_000);
        let shown = PageShown {
            title: Some(long.clone()),
            address: Some(long.clone()),
            load: HtmlLoadState::Failed,
            error: Some(long),
            can_go_back: false,
            can_go_forward: false,
        };
        let state = PresentationPageState::new("p1", shown, false, 0);
        let count = |text: &Option<String>| text.as_deref().map(|text| text.chars().count());
        assert_eq!(count(&state.title), Some(PresentationPageState::MAX_TITLE));
        assert_eq!(count(&state.address), Some(PresentationPageState::MAX_ADDRESS));
        assert_eq!(count(&state.error), Some(PresentationPageState::MAX_ERROR));
        let line = encode(&PresentationFrame {
            generation: i64::MAX,
            rev: i64::MAX,
            body: Body::PageState(state),
        })
        .unwrap();
        assert!(line.len() < MAX_FRAME_BYTES);
        let wide = PresentationPageState::new(
            "p1",
            PageShown {
                title: Some("é".repeat(2000)),
                ..PageShown::default()
            },
            false,
            0,
        );
        assert_eq!(
            count(&wide.title),
            Some(PresentationPageState::MAX_TITLE),
            "bounded in characters"
        );
    }

    #[test]
    fn the_wire_spells_ids_and_keys_as_upstream() {
        let frame = PresentationFrame {
            generation: 2,
            rev: 7,
            body: Body::AskRequest(PresentationAsk::new(&ask(), Some(pane()), 3)),
        };
        let line = encode(&frame).unwrap();
        let value: Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(value["kind"], "ask.request");
        assert_eq!(value["ask"]["pane"], "11111111-2222-3333-4444-555555555555");
        assert_eq!(value["ask"]["defaultID"], "y");
        assert_eq!(value["ask"]["style"], "gui");
        assert!(value["ask"].get("destructiveID").is_none());
        let none = encode(&PresentationFrame {
            generation: 1,
            rev: 1,
            body: Body::Status(None),
        })
        .unwrap();
        assert_eq!(none, b"{\"kind\":\"status\",\"gen\":1,\"rev\":1}\n");
        let marker = encode(&PresentationFrame {
            generation: 1,
            rev: 2,
            body: Body::Sound(PresentationSound { name: None }),
        })
        .unwrap();
        assert_eq!(marker, b"{\"kind\":\"sound\",\"gen\":1,\"rev\":2,\"sound\":{}}\n");
    }

    #[test]
    fn a_form_request_spells_its_keys_as_upstream() {
        let line = encode(&PresentationFrame {
            generation: 1,
            rev: 1,
            body: Body::AskRequest(PresentationAsk {
                cancel_id: Some("ok".to_owned()),
                ..PresentationAsk::new(&form_ask(), None, 3)
            }),
        })
        .unwrap();
        let value: Value = serde_json::from_slice(&line).unwrap();
        assert_eq!(
            value["ask"]["choices"],
            json!({"choices": [{"id": "a", "label": "A"}], "multiple": true, "checked": [], "other": "Other"})
        );
        assert_eq!(value["ask"]["cancelID"], "ok");
        assert!(value["ask"].get("input").is_none());
        let input = sized_ask(
            vec![button("ok")],
            AskForm::Input(AskInputForm::new(AskInputMode::Line)),
        );
        let value = serde_json::to_value(&input).unwrap();
        assert_eq!(value["input"], json!({"mode": "line", "value": ""}));
        assert!(value.get("choices").is_none() && value.get("cancelID").is_none());
        let plain = serde_json::to_value(answer("a", Some("ok"), None, None)).unwrap();
        assert_eq!(plain, json!({"id": "a", "owner": 3, "button": "ok"}));
    }

    /// Upstream's `aChoiceRequestFromAnOriginWithoutTheOtherRowDecodesWithNone`, through a whole request.
    #[test]
    fn a_choice_request_from_an_origin_without_the_other_row_decodes_with_none() {
        let line = br#"{"kind":"ask.request","gen":1,"rev":1,"ask":{"id":"a","title":"t","buttons":[],"style":"gui","align":"right","owner":1,"choices":{"choices":[{"id":"a","label":"A"}],"multiple":false,"checked":[]}}}"#;
        let Body::AskRequest(request) = decode(line).unwrap().body else {
            panic!("not an ask request");
        };
        assert_eq!(request.choices, Some(AskChoiceForm::new(vec![choice("a")])));
        assert!(!request.needs_other_support());
        assert_eq!(
            request.form(),
            Some(AskForm::Choices(AskChoiceForm::new(vec![choice("a")])))
        );
        assert!(request.needs_form_support());
    }

    #[test]
    fn a_request_carrying_both_forms_reads_as_its_text_field() {
        let mut request = sized_ask(
            vec![button("ok")],
            AskForm::Input(AskInputForm::new(AskInputMode::Line)),
        );
        request.choices = Some(AskChoiceForm::new(vec![choice("a")]));
        assert_eq!(
            request.form(),
            Some(AskForm::Input(AskInputForm::new(AskInputMode::Line)))
        );
        let plain = PresentationAsk::new(&ask(), None, 1);
        assert!(!plain.needs_form_support() && !plain.needs_other_support());
        assert_eq!(plain.form(), None);
        let cancel = PresentationAsk {
            cancel_id: Some("y".to_owned()),
            ..plain
        };
        assert!(cancel.needs_form_support(), "a cancel role alone needs the capability");
    }

    /// Upstream's `anInputAskStaysHereWhenItsLongestButtonAndTheReservedTextCannotFitAnAnswer`, the frame half.
    #[test]
    fn an_input_ask_does_not_fit_when_its_longest_button_and_the_reserved_text_cannot_fit_an_answer() {
        let long = AskButton {
            id: "b".repeat(200 * 1024),
            label: "Long".to_owned(),
            hotkey: None,
        };
        let input = sized_ask(
            vec![long.clone()],
            AskForm::Input(AskInputForm::new(AskInputMode::Line)),
        );
        let list = sized_ask(vec![long], AskForm::Choices(AskChoiceForm::new(vec![choice("a")])));
        assert!(!input.fits_frames());
        assert!(list.fits_frames());
    }

    /// Upstream's `aFormAskWhoseFrameIsTooLargeStaysHere`, the frame half.
    #[test]
    fn a_request_over_the_limit_does_not_fit() {
        let mut request = sized_ask(
            vec![button("ok")],
            AskForm::Input(AskInputForm::new(AskInputMode::Line)),
        );
        request.message = Some("m".repeat(MAX_FRAME_BYTES));
        assert!(!request.fits_frames());
    }

    /// Upstream's `aCheckboxOtherAskReservesEveryIDPlusTheWidestText`.
    #[test]
    fn a_checkbox_other_ask_reserves_every_id_plus_the_widest_text() {
        let probe = PresentationAskAnswer {
            id: "a".to_owned(),
            owner: 1,
            button: Some("ok".to_owned()),
            text: Some(widest_text()),
            selected: Some(Vec::new()),
        };
        let room = MAX_FRAME_BYTES - encoded_len(Body::AskResolve(probe), i64::MAX);
        let ask = |id_length: usize, multiple: bool| {
            let items = (0..2)
                .map(|index| AskChoice {
                    id: format!("{index}{}", "i".repeat(id_length)),
                    label: "L".to_owned(),
                    subtitle: None,
                })
                .collect();
            sized_ask(
                vec![button("ok")],
                AskForm::Choices(AskChoiceForm {
                    multiple,
                    other: Some("Other".to_owned()),
                    ..AskChoiceForm::new(items)
                }),
            )
        };
        assert!(!ask(room / 2, true).fits_frames());
        assert!(ask(room / 2, false).fits_frames());
        assert!(ask(room / 8, true).fits_frames());
    }

    /// Upstream's `slashesInAButtonIDCountAsTheCodecWritesThem`: this encoder, like upstream's, writes `/` bare, so the
    /// request with both ids fits while the answer naming the long button cannot.
    #[test]
    fn slashes_in_a_button_id_count_as_the_codec_writes_them() {
        let slashes = AskButton {
            id: "/".repeat(90 * 1024),
            label: "Slashes".to_owned(),
            hotkey: None,
        };
        let letters = AskButton {
            id: "b".repeat(150 * 1024),
            label: "Letters".to_owned(),
            hotkey: None,
        };
        let request = sized_ask(
            vec![slashes.clone(), letters],
            AskForm::Input(AskInputForm::new(AskInputMode::Line)),
        );
        assert!(fits(Body::AskRequest(request.clone())), "a slash encodes as one byte");
        assert!(!request.fits_frames());
        let line = encode(&PresentationFrame {
            generation: 1,
            rev: 1,
            body: Body::AskResolve(answer("a", Some(&slashes.id), None, None)),
        })
        .unwrap();
        assert!(!line.windows(2).any(|pair| pair == b"\\/"));
    }

    /// Upstream's `anAnswerThatFitsOnlyWithShortCountersIsRefused`.
    #[test]
    fn an_answer_that_fits_only_with_short_counters_does_not_fit() {
        let probe = PresentationAskAnswer {
            id: "a".to_owned(),
            owner: 1,
            button: Some(String::new()),
            text: Some(widest_text()),
            selected: None,
        };
        let slack = MAX_FRAME_BYTES - encoded_len(Body::AskResolve(probe), 0) - 1;
        let id = "b".repeat(slack - 5);
        let request = sized_ask(
            vec![AskButton {
                id: id.clone(),
                label: "B".to_owned(),
                hotkey: None,
            }],
            AskForm::Input(AskInputForm::new(AskInputMode::Line)),
        );
        let answer = PresentationAskAnswer {
            id: "a".to_owned(),
            owner: 1,
            button: Some(id),
            text: Some(widest_text()),
            selected: None,
        };
        assert!(
            encode(&PresentationFrame {
                generation: 0,
                rev: 0,
                body: Body::AskResolve(answer),
            })
            .is_ok()
        );
        assert!(!request.fits_frames());
    }

    #[test]
    fn an_unknown_kind_decodes_to_unknown_and_keeps_its_ordering() {
        let line = br#"{"kind":"future.kind","gen":3,"rev":12,"job":"abc"}"#;
        assert_eq!(
            decode(line).unwrap(),
            PresentationFrame {
                generation: 3,
                rev: 12,
                body: Body::Unknown("future.kind".to_owned())
            }
        );
    }

    #[test]
    fn a_snapshot_from_an_origin_without_context_decodes_with_none() {
        let line = br#"{"kind":"snapshot","gen":1,"rev":1,"snapshot":{}}"#;
        assert_eq!(
            decode(line).unwrap().body,
            Body::Snapshot(PresentationSnapshot::default())
        );
    }

    #[test]
    fn an_oversize_line_is_refused_before_decoding() {
        let line = vec![b'x'; MAX_FRAME_BYTES + 1];
        assert_eq!(decode(&line), Err(FrameError::Oversize(line.len())));
    }

    #[test]
    fn a_markdown_hud_with_its_font_size_survives_a_frame() {
        let mut spec = HudSpec::new("# Tasks\n\n- build");
        spec.markdown = true;
        spec.font_size = Some(18.0);
        let frame = PresentationFrame {
            generation: 1,
            rev: 3,
            body: Body::Hud(Some(PresentationHud {
                spec,
                pane: None,
                generation: 2,
                remaining: None,
            })),
        };
        let line = encode(&frame).unwrap();
        assert_eq!(decode(&line[..line.len() - 1]).unwrap(), frame);
    }

    #[test]
    fn the_largest_markdown_hud_fits_a_frame() {
        let mut spec = HudSpec::new("\u{1F600}".repeat(HudSpec::MAX_MARKDOWN_LENGTH));
        spec.detail = Some("\u{1F600}".repeat(HudSpec::MAX_TEXT_LENGTH));
        spec.markdown = true;
        let line = encode(&PresentationFrame {
            generation: 1,
            rev: 1,
            body: Body::Hud(Some(PresentationHud {
                spec,
                pane: None,
                generation: 1,
                remaining: None,
            })),
        })
        .unwrap();
        assert!(line.len() <= MAX_FRAME_BYTES);
    }

    #[test]
    fn a_frame_that_would_encode_oversize_is_refused() {
        let frame = PresentationFrame {
            generation: 1,
            rev: 1,
            body: Body::Notify(PresentationNotify {
                title: "t".to_owned(),
                body: "x".repeat(MAX_FRAME_BYTES),
                pane: None,
                source: "control".to_owned(),
            }),
        };
        assert!(matches!(encode(&frame), Err(FrameError::Oversize(_))));
    }

    #[test]
    fn a_malformed_line_reports_what_was_wrong() {
        for text in [
            "not json",
            r#"{"gen":1,"rev":1}"#,
            r#"{"kind":"status","rev":1}"#,
            r#"{"kind":"hello","gen":1,"rev":1}"#,
        ] {
            match decode(text.as_bytes()) {
                Err(FrameError::Malformed(detail)) => assert!(!detail.is_empty(), "{text}"),
                other => panic!("{text}: {other:?}"),
            }
        }
    }

    #[test]
    fn negotiation_picks_the_lower_version() {
        for (ours, theirs, expected) in [(1, 1, 1), (1, 3, 1), (3, 1, 1), (2, 5, 2)] {
            assert_eq!(negotiated_version(ours, theirs), Some(expected));
        }
        for theirs in [0, -1] {
            assert_eq!(negotiated_version(1, theirs), None);
        }
    }

    /// Upstream's `PresentationLayoutTests`.
    #[test]
    fn layout_validity_and_lossy_decoding() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let single = PresentationLayout {
            panes: vec![a],
            primary: a,
            axis: None,
            shown: false,
        };
        assert!(single.is_valid());
        assert!(
            !PresentationLayout {
                shown: true,
                ..single.clone()
            }
            .is_valid()
        );
        let split = PresentationLayout {
            panes: vec![a, b],
            primary: a,
            axis: Some("horizontal".to_owned()),
            shown: false,
        };
        assert!(split.is_valid());
        for broken in [
            PresentationLayout {
                axis: None,
                ..split.clone()
            },
            PresentationLayout {
                axis: Some("diagonal".to_owned()),
                ..split.clone()
            },
            PresentationLayout {
                panes: vec![a, a],
                ..split.clone()
            },
            PresentationLayout {
                primary: Uuid::new_v4(),
                ..split.clone()
            },
            PresentationLayout::invalid(),
        ] {
            assert!(!broken.is_valid(), "{broken:?}");
        }
        let line = br#"{"kind":"snapshot","gen":1,"rev":1,"snapshot":{"context":"c","layout":{"panes":"no"}}}"#;
        let Body::Snapshot(snapshot) = decode(line).unwrap().body else {
            panic!("not a snapshot");
        };
        assert_eq!(snapshot.context.as_deref(), Some("c"), "a bad layout keeps the rest");
        assert_eq!(snapshot.layout, Some(PresentationLayout::invalid()));
        let bare = br#"{"kind":"layout","gen":1,"rev":1}"#;
        assert_eq!(decode(bare).unwrap().body, Body::Layout(PresentationLayout::invalid()));
        let older = br#"{"kind":"snapshot","gen":1,"rev":1,"snapshot":{"status":null}}"#;
        assert_eq!(
            decode(older).unwrap().body,
            Body::Snapshot(PresentationSnapshot::default())
        );
    }
}
