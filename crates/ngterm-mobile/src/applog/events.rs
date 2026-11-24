//! What the log keeps of each [`MobileEvent`]: its kind, the ids it is about and the class of its state, never its
//! payload (a title, a status text, a notification, a chat body, a clipboard's contents, a fingerprint, a failure's
//! wording). [`LoggedSink`] stands before the app's [`EventSink`], so every event the core or its transport reports
//! passes here once, and a new event has to be given its line (or none) for the crate to build.

use std::sync::Arc;

use super::tag;
use crate::api::{EventSink, MachineState, MobileEvent};
use crate::chat::{ChatBody, ChatEvent, ChatState};
use crate::note;
use crate::pane::{PaneEnd, PaneRef, PaneRole};
use crate::presentation::{PresentationRole, StreamState};

/// The app's sink, with each event logged before it is handed on.
pub(crate) struct LoggedSink {
    app: Arc<dyn EventSink>,
}

impl LoggedSink {
    pub(crate) fn wrap(app: Arc<dyn EventSink>) -> Arc<dyn EventSink> {
        Arc::new(Self { app })
    }
}

impl EventSink for LoggedSink {
    fn on_event(&self, event: MobileEvent) {
        log(&event);
        self.app.on_event(event);
    }
}

impl StreamState {
    /// The state's name, for the log.
    #[must_use]
    pub fn class(&self) -> &'static str {
        match self {
            Self::Connecting => "connecting",
            Self::Connected => "connected",
            Self::Unsupported => "unsupported",
            Self::Failed { .. } => "failed",
        }
    }
}

impl PresentationRole {
    /// The role's name, for the log.
    #[must_use]
    pub fn class(self) -> &'static str {
        match self {
            Self::Presenter => "presenter",
            Self::Mirror => "mirror",
        }
    }
}

impl PaneRole {
    /// The role's name, for the log.
    #[must_use]
    pub fn class(self) -> &'static str {
        match self {
            Self::Unowned => "unowned",
            Self::Leader => "leader",
            Self::Follower => "follower",
        }
    }
}

impl PaneEnd {
    /// The end's name, for the log.
    #[must_use]
    pub fn class(&self) -> &'static str {
        match self {
            Self::Lost { .. } => "lost",
            Self::Exited { .. } => "exited",
            Self::Gone => "gone",
        }
    }
}

impl ChatState {
    /// The state's name, for the log.
    #[must_use]
    pub fn class(self) -> &'static str {
        match self {
            Self::Reconnecting => "reconnecting",
            Self::Ended => "ended",
        }
    }
}

fn log(event: &MobileEvent) {
    match event {
        MobileEvent::MachineState { .. }
        | MobileEvent::HostKeyUnknown { .. }
        | MobileEvent::HostKeyChanged { .. }
        | MobileEvent::SignInRequested { .. }
        | MobileEvent::Tree { .. } => machine(event),
        MobileEvent::SessionConnection { .. }
        | MobileEvent::SessionLayout { .. }
        | MobileEvent::SessionMode { .. }
        | MobileEvent::Notify { .. }
        | MobileEvent::Ask { .. }
        | MobileEvent::AskDismiss { .. }
        | MobileEvent::Pick { .. }
        | MobileEvent::PickDismiss { .. }
        | MobileEvent::PaneAgent { .. } => session(event),
        MobileEvent::PaneAttached { .. }
        | MobileEvent::PaneLead { .. }
        | MobileEvent::PaneEnded { .. }
        | MobileEvent::ClipboardWrite { .. }
        | MobileEvent::ClipboardReadRequest { .. }
        | MobileEvent::ClipboardReadDismiss { .. } => pane(event),
        MobileEvent::Chat { pane, events } => chat(pane, events),
        MobileEvent::ChatState { pane, state } => note!(
            INFO,
            tag::CHAT,
            "chat state",
            state = state.class(),
            session = pane.session_id.as_str(),
            pane = pane.pane_id.as_str()
        ),
        // Too frequent, or nothing but payload: a frame, a title, a status text, a context, a name, a HUD, a sound,
        // a bell.
        MobileEvent::PaneDirty { .. }
        | MobileEvent::PaneTitle { .. }
        | MobileEvent::Bell { .. }
        | MobileEvent::SessionStatus { .. }
        | MobileEvent::SessionContext { .. }
        | MobileEvent::SessionName { .. }
        | MobileEvent::Hud { .. }
        | MobileEvent::Sound { .. } => {}
    }
}

fn machine(event: &MobileEvent) {
    match event {
        MobileEvent::MachineState { machine_id, state } => machine_state(machine_id, state),
        MobileEvent::HostKeyUnknown { machine_id, .. } => {
            note!(
                INFO,
                tag::SSH,
                "host key unknown: asking the user",
                machine = machine_id.as_str()
            );
        }
        MobileEvent::HostKeyChanged { machine_id, .. } => {
            note!(
                WARN,
                tag::SSH,
                "host key changed: connect stopped",
                machine = machine_id.as_str()
            );
        }
        MobileEvent::SignInRequested { machine_id, .. } => {
            note!(
                INFO,
                tag::SSH,
                "machine asks for a sign-in",
                machine = machine_id.as_str()
            );
        }
        MobileEvent::Tree {
            machine_id,
            sessions,
            hint,
        } => note!(
            INFO,
            tag::SESSIONS,
            "session list reported",
            machine = machine_id.as_str(),
            sessions = sessions.len(),
            hint = hint.is_some()
        ),
        _ => {}
    }
}

fn machine_state(machine_id: &str, state: &MachineState) {
    match state {
        MachineState::Connected => note!(INFO, tag::LINK, "machine connected", machine = machine_id),
        MachineState::Reconnecting { next_in_ms } => note!(
            WARN,
            tag::LINK,
            "machine reconnecting",
            machine = machine_id,
            next_in_ms = *next_in_ms
        ),
        MachineState::Unreachable { .. } => note!(WARN, tag::LINK, "machine unreachable", machine = machine_id),
    }
}

fn session(event: &MobileEvent) {
    match event {
        MobileEvent::SessionConnection { session_id, state, .. } => note!(
            INFO,
            tag::STREAM,
            "stream state",
            state = state.class(),
            session = session_id.as_str()
        ),
        MobileEvent::SessionLayout { session_id, layout, .. } => note!(
            INFO,
            tag::STREAM,
            "layout",
            session = session_id.as_str(),
            panes = layout.panes.len(),
            split_shown = layout.shown
        ),
        MobileEvent::SessionMode { session_id, role, .. } => note!(
            INFO,
            tag::STREAM,
            "presentation role",
            role = role.class(),
            session = session_id.as_str()
        ),
        MobileEvent::Notify { session_id, .. } => {
            note!(INFO, tag::STREAM, "notification", session = session_id.as_str());
        }
        MobileEvent::Ask { session_id, .. } => note!(INFO, tag::STREAM, "ask opened", session = session_id.as_str()),
        MobileEvent::AskDismiss { session_id, .. } => {
            note!(INFO, tag::STREAM, "ask dismissed", session = session_id.as_str());
        }
        MobileEvent::Pick { session_id, .. } => note!(INFO, tag::STREAM, "pick opened", session = session_id.as_str()),
        MobileEvent::PickDismiss { session_id, .. } => {
            note!(INFO, tag::STREAM, "pick dismissed", session = session_id.as_str());
        }
        MobileEvent::PaneAgent {
            session_id,
            pane_id,
            agent,
            conversation_id,
            ..
        } => note!(
            INFO,
            tag::CHAT,
            "pane agent announced",
            agent = agent.as_deref().unwrap_or("none"),
            conversation = conversation_id.is_some(),
            session = session_id.as_str(),
            pane = pane_id.as_str()
        ),
        _ => {}
    }
}

fn pane(event: &MobileEvent) {
    match event {
        MobileEvent::PaneAttached { pane } => note!(
            INFO,
            tag::PANE,
            "attached",
            session = pane.session_id.as_str(),
            pane = pane.pane_id.as_str()
        ),
        MobileEvent::PaneLead { pane, role, covered } => note!(
            INFO,
            tag::PANE,
            "role",
            role = role.map_or("none", PaneRole::class),
            covered = *covered,
            session = pane.session_id.as_str(),
            pane = pane.pane_id.as_str()
        ),
        MobileEvent::PaneEnded { pane, end } => pane_ended(pane, end),
        MobileEvent::ClipboardWrite { pane, .. } => {
            note!(INFO, tag::PANE, "clipboard write asked", pane = pane.pane_id.as_str());
        }
        MobileEvent::ClipboardReadRequest { pane, granted, .. } => note!(
            INFO,
            tag::PANE,
            "clipboard read asked",
            granted = *granted,
            pane = pane.pane_id.as_str()
        ),
        MobileEvent::ClipboardReadDismiss { pane, .. } => {
            note!(
                INFO,
                tag::PANE,
                "clipboard read dismissed",
                pane = pane.pane_id.as_str()
            );
        }
        _ => {}
    }
}

fn pane_ended(pane: &PaneRef, end: &PaneEnd) {
    let code = match end {
        PaneEnd::Exited { code, .. } => code.map(u64::from),
        _ => None,
    };
    note!(
        WARN,
        tag::PANE,
        "ended",
        end = end.class(),
        code = code,
        session = pane.session_id.as_str(),
        pane = pane.pane_id.as_str()
    );
}

/// A batch of chat events: how many, and how many of them are the user's prompts, never what they say.
fn chat(pane: &PaneRef, events: &[ChatEvent]) {
    let count = |wanted: fn(&ChatBody) -> bool| events.iter().filter(|event| wanted(&event.body)).count();
    note!(
        INFO,
        tag::CHAT,
        "chat events",
        count = events.len(),
        prompts = count(|body| matches!(body, ChatBody::User { .. })),
        live = count(|body| matches!(body, ChatBody::Live)) > 0,
        reset = count(|body| matches!(body, ChatBody::Reset)) > 0,
        session = pane.session_id.as_str(),
        pane = pane.pane_id.as_str()
    );
}
