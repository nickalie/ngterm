//! `notify` and terminal desktop notifications (OSC 9/777): the badge, the `notify` event and the banner the frontend
//! posts (upstream's `ControlDispatcher` `.notify` arm, `ControlServer.sendNotification`, `NotificationManager.notify`
//! and `send(toSession:)`, `Notifications.swift`, `notifications.md`).
//!
//! The core records the event and bumps the badge; the banner itself is the frontend's ([`Core::take_notifications`]),
//! gated by the "Show notification banners" setting. A terminal's banner takes its pane's slot, a `notify` the
//! session's control slot ([`control_identity`]), so neither replaces the other. Withdrawing delivered banners is edge-triggered, as upstream's
//! `clearDelivered`: the core queues a [`NotificationEffect::Clear`] in the same queue as the posts at each site that
//! clears them (a pane taking focus, zoomed or not, and a successful swap), and only for a session an open window
//! holds. The badge clears on its own path (a selection, `session.seen`), which leaves the banners alone.

use ngterm_proto::{ControlResult, Request};
use uuid::Uuid;

use super::{Outcome, args_of, with_id};
use crate::Core;
use crate::error::ControlError;
use crate::model::StatusPane;
use crate::pane_env::uuid_string;
use crate::resolve::Resolver;

/// `result.text` when banners are off (upstream's `ControlNotify.bannersOffNote`).
pub const BANNERS_OFF_NOTE: &str = "badge updated, but \"Show notification banners\" is off, so no banner was posted";

/// Which surface of a session fired a notification (upstream's `PaneRole`), part of the banner's identity so a click
/// reveals that pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PaneRole {
    Main,
    Split,
    Overlay,
}

impl PaneRole {
    pub const ALL: [Self; 3] = [Self::Main, Self::Split, Self::Overlay];

    #[must_use]
    pub fn wire_name(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Split => "split",
            Self::Overlay => "overlay",
        }
    }

    #[must_use]
    pub fn from_wire(raw: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|role| role.wire_name() == raw)
    }
}

/// A banner's identity, `<windowID>:<sessionID>:<paneRole>` (upstream's `TerminalNotification.identity`): repeats from
/// one pane replace each other, and a click decodes its target from it.
#[must_use]
pub fn identity(window: Uuid, session: Uuid, pane: PaneRole) -> String {
    format!("{}:{}:{}", uuid_string(window), uuid_string(session), pane.wire_name())
}

/// The suffix of the control `notify` slot's identity (upstream's `controlSuffix`).
const CONTROL_SUFFIX: &str = "notify";

/// The identity of a control `notify` banner, `<windowID>:<sessionID>:notify` (upstream's `controlIdentity`): one slot
/// per session, apart from every pane's, so a terminal banner never replaces a note posted on purpose; repeated notes
/// still replace each other.
#[must_use]
pub fn control_identity(window: Uuid, session: Uuid) -> String {
    format!("{}:{}:{CONTROL_SUFFIX}", uuid_string(window), uuid_string(session))
}

/// Every identity a session's banners can carry: each pane's, then the control slot (upstream's `sessionIdentities`).
#[must_use]
pub fn session_identities(window: Uuid, session: Uuid) -> Vec<String> {
    PaneRole::ALL
        .into_iter()
        .map(|pane| identity(window, session, pane))
        .chain(std::iter::once(control_identity(window, session)))
        .collect()
}

/// The click target of a banner identity, an [`identity`] or a [`control_identity`], whose `notify` suffix targets the
/// main pane; `None` for a malformed one (upstream's `parseIdentity`).
#[must_use]
pub fn parse_identity(identity: &str) -> Option<(Uuid, Uuid, PaneRole)> {
    let parts: Vec<&str> = identity.split(':').collect();
    let [window, session, pane] = parts.as_slice() else {
        return None;
    };
    let pane = if *pane == CONTROL_SUFFIX {
        PaneRole::Main
    } else {
        PaneRole::from_wire(pane)?
    };
    Some((Uuid::parse_str(window).ok()?, Uuid::parse_str(session).ok()?, pane))
}

/// Whether a terminal notification is delivered: suppressed only while the firing pane holds the keyboard in the
/// active app (upstream's `shouldDeliver`).
#[must_use]
pub fn should_deliver(firing_is_focused: bool, app_active: bool) -> bool {
    !(firing_is_focused && app_active)
}

/// The slot of a session's banners a post takes: the firing pane's (a terminal's OSC 9/777), or the control slot (a
/// `notify`, or one a remote origin sent), which a pane's banner never replaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BannerSlot {
    Pane(PaneRole),
    Control,
}

/// A banner for the frontend to post.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotificationPost {
    pub window: Uuid,
    pub session: Uuid,
    pub slot: BannerSlot,
    /// The effective title: the caller's, else the session's name.
    pub title: String,
    pub body: String,
}

impl NotificationPost {
    /// The banner's identity: its pane's [`identity`], or the session's [`control_identity`].
    #[must_use]
    pub fn identity(&self) -> String {
        match self.slot {
            BannerSlot::Pane(pane) => identity(self.window, self.session, pane),
            BannerSlot::Control => control_identity(self.window, self.session),
        }
    }
}

/// What the frontend does with banners, queued in order so a post and a withdrawal are never reordered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotificationEffect {
    /// Post a banner.
    Post(NotificationPost),
    /// Withdraw every delivered banner of session `session` in open window `window`, each pane's and the control slot's
    /// (upstream's `clearDelivered`).
    Clear { window: Uuid, session: Uuid },
    /// Post an app-level banner tied to no pane (a keymap or hooks problem, a failed custom command or hook), under
    /// `identity`, which coalesces repeats.
    App {
        identity: String,
        title: String,
        body: String,
    },
}

impl NotificationEffect {
    /// The identities the effect withdraws: the session's [`session_identities`] for a clear, none for a post.
    #[must_use]
    pub fn withdrawn_identities(&self) -> Vec<String> {
        match self {
            Self::Post(_) | Self::App { .. } => Vec::new(),
            Self::Clear { window, session } => session_identities(*window, *session),
        }
    }
}

/// `notify`: a body is required, checked before the target; the badge always counts it and the banner, posted in the
/// session's control slot, obeys the setting, whose being off the result notes.
pub(crate) fn dispatch(core: &mut Core, request: &Request) -> Outcome {
    let args = args_of(request);
    let body = args
        .body
        .as_deref()
        .filter(|body| !body.is_empty())
        .ok_or_else(|| ControlError::rejected("notify requires a body"))?;
    let resolved = Resolver::new(core.library()).session(request.target.as_deref(), args.window.as_deref())?;
    let title = args.title.as_deref().unwrap_or("");
    let Some(title) = core.deliver_notification(resolved.window, resolved.session, BannerSlot::Control, title, body)
    else {
        return Err(ControlError::rejected("session's window is not open"));
    };
    // only a control-origin notification travels to viewers: a terminal's reaches them through its live session itself
    core.publish_notify(resolved.session, title, body);
    let mut result = with_id(resolved.session);
    if !core.banners_enabled() {
        result = ControlResult {
            text: Some(BANNERS_OFF_NOTE.to_owned()),
            ..result
        };
    }
    Ok(result)
}

impl Core {
    /// Whether banners are posted (the "Show notification banners" setting, on by default).
    #[must_use]
    pub fn banners_enabled(&self) -> bool {
        self.settings().notifications_enabled.unwrap_or(true)
    }

    /// Records a notification on session `session` of open window `window` and, with banners on, queues its banner in
    /// `slot`: the `notify` event, the badge, the post. Its effective title; `None` when the window or session is gone.
    pub(crate) fn deliver_notification(
        &mut self,
        window: Uuid,
        session: Uuid,
        slot: BannerSlot,
        title: &str,
        body: &str,
    ) -> Option<String> {
        let title = self
            .library_mut()
            .store_mut(window)
            .and_then(|store| store.record_notification(session, title, body))?;
        if self.banners_enabled() {
            self.notifications.push(NotificationEffect::Post(NotificationPost {
                window,
                session,
                slot,
                title: title.clone(),
                body: body.to_owned(),
            }));
        } else {
            tracing::info!(%session, "notify: badge bumped, but banners are off in the settings");
        }
        Some(title)
    }

    /// A terminal's OSC 9/777 desktop notification from the pane carrying `token` of session `session` (upstream's
    /// `NotificationManager.notify`). The frontend has already applied [`should_deliver`]. The title falls back to the
    /// session's name. Whether it was recorded; commit afterwards.
    pub fn terminal_notification(&mut self, session: Uuid, token: &str, title: &str, body: &str) -> bool {
        let Some(window) = self.library().window_holding_session(session) else {
            return false;
        };
        let pane = match self
            .library()
            .store(window)
            .and_then(|store| store.session(session))
            .and_then(|found| found.pane_role_for_token(token))
        {
            Some(StatusPane::Right) => PaneRole::Split,
            _ => PaneRole::Main,
        };
        self.deliver_notification(window, session, BannerSlot::Pane(pane), title, body)
            .is_some()
    }

    /// A banner was clicked (upstream's `AppActions.reveal`): the session is revealed in the open window holding it,
    /// else window `window` is opened again when it was closed since. The owning window leaves its zoom (the selection
    /// would change behind it), the session is selected and its badge cleared, the pane that fired is asked to take the
    /// keyboard (its banners are withdrawn only once the frontend gives it focus, which a cover may block)
    /// (the primary when the session no longer has a split), and the window comes to the front. Whether the split
    /// survives is the model's `split.exists` (upstream's `hasSplit`), not whether its terminal is mounted yet: a
    /// reopened window's split, or a restored hidden one, has none until the frontend lays it out, and the focus
    /// request takes it once it does. A closed window is reopened even when the session turns out gone, as upstream's.
    /// Whether anything changed; commit afterwards.
    pub fn reveal_notification(&mut self, window: Uuid, session: Uuid, pane: PaneRole) -> bool {
        let holder = self.library().window_id_for_session(session);
        let mut reopened = false;
        if holder.is_none() && !self.library().is_open(window) {
            self.open_window(window);
            reopened = self.library().is_open(window);
        }
        let window = holder.unwrap_or(window);
        if self
            .library()
            .store(window)
            .and_then(|store| store.session(session))
            .is_none()
        {
            return reopened;
        }
        self.set_zoom_target(window, None);
        self.note_window_activity(window);
        let Some(store) = self.library_mut().store_mut(window) else {
            return reopened;
        };
        let Some(found) = store.session(session) else {
            return reopened;
        };
        let split = pane == PaneRole::Split && found.split.exists;
        store.select_session(Some(session));
        store.clear_unseen(session);
        store.set_split_focus(session, split);
        self.request_pane_focus(session);
        self.take_frontmost(window);
        self.request_window(crate::windows::WindowRequest::Raise(window));
        true
    }

    /// Queues the withdrawal of session `session`'s delivered banners, every pane's and the control slot's (upstream's
    /// `clearDelivered`): a no-op when no open window holds the session, so a closed window's banners stay clickable.
    /// The badge is left as it is. Whether a withdrawal was queued; settle afterwards when it was.
    pub(crate) fn clear_delivered(&mut self, session: Uuid) -> bool {
        let Some(window) = self.library().window_id_for_session(session) else {
            tracing::debug!(%session, "notify: no open window holds the session; no banner to clear");
            return false;
        };
        self.notifications.push(NotificationEffect::Clear { window, session });
        true
    }

    /// Queues an app-level banner when banners are on (upstream's `NotificationManager` app-level posts, each guarded
    /// by `bannersEnabled`).
    pub(crate) fn post_app_banner(&mut self, identity: impl Into<String>, title: &str, body: impl Into<String>) {
        if !self.banners_enabled() {
            return;
        }
        self.notifications.push(NotificationEffect::App {
            identity: identity.into(),
            title: title.to_owned(),
            body: body.into(),
        });
    }

    /// Takes the banner posts and withdrawals queued since the last take, in order.
    pub fn take_notifications(&mut self) -> Vec<NotificationEffect> {
        std::mem::take(&mut self.notifications)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identities_round_trip_and_malformed_ones_are_refused() {
        let (window, session) = (Uuid::new_v4(), Uuid::new_v4());
        for pane in PaneRole::ALL {
            let text = identity(window, session, pane);
            assert_eq!(parse_identity(&text), Some((window, session, pane)));
        }
        assert_eq!(
            identity(window, session, PaneRole::Split),
            format!("{}:{}:split", uuid_string(window), uuid_string(session))
        );
        for bad in [
            "",
            "a:b:main",
            &format!("{}:{}", uuid_string(window), uuid_string(session)),
        ] {
            assert_eq!(parse_identity(bad), None, "{bad}");
        }
        let extra = format!("{}:{}:main:x", uuid_string(window), uuid_string(session));
        assert_eq!(parse_identity(&extra), None);
        let role = format!("{}:{}:scratch", uuid_string(window), uuid_string(session));
        assert_eq!(parse_identity(&role), None);
    }

    fn fixed_ids() -> (Uuid, Uuid) {
        (
            Uuid::parse_str("00000000-0000-0000-0000-000000000000").unwrap(),
            Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
        )
    }

    /// Upstream's `controlIdentityFormatIsWindowColonSessionColonNotify`.
    #[test]
    fn control_identity_format_is_window_colon_session_colon_notify() {
        let (window, session) = fixed_ids();
        assert_eq!(
            control_identity(window, session),
            "00000000-0000-0000-0000-000000000000:11111111-1111-1111-1111-111111111111:notify"
        );
    }

    /// Upstream's `controlIdentityIsStableAndNeverCollidesWithPaneIdentities`.
    #[test]
    fn control_identity_is_stable_and_never_collides_with_pane_identities() {
        let (window, session) = (Uuid::new_v4(), Uuid::new_v4());
        let control = control_identity(window, session);
        assert_eq!(control, control_identity(window, session));
        for pane in PaneRole::ALL {
            assert_ne!(control, identity(window, session, pane), "{pane:?}");
        }
    }

    /// Upstream's `controlIdentityParsesToTheMainPane`.
    #[test]
    fn control_identity_parses_to_the_main_pane() {
        let (window, session) = (Uuid::new_v4(), Uuid::new_v4());
        assert_eq!(
            parse_identity(&control_identity(window, session)),
            Some((window, session, PaneRole::Main))
        );
    }

    /// Upstream's `sessionIdentitiesCoverEveryPaneAndTheControlSlot`.
    #[test]
    fn session_identities_cover_every_pane_and_the_control_slot() {
        let (window, session) = (Uuid::new_v4(), Uuid::new_v4());
        let mut expected: Vec<String> = PaneRole::ALL
            .into_iter()
            .map(|pane| identity(window, session, pane))
            .collect();
        expected.push(control_identity(window, session));
        let identities = session_identities(window, session);
        assert_eq!(identities.len(), 4);
        assert_eq!(identities, expected);
        let clear = NotificationEffect::Clear { window, session };
        assert_eq!(clear.withdrawn_identities(), expected, "a clear withdraws all four");
    }

    /// Upstream's `parseRejectsMalformed`, the control slot's two cases included.
    #[test]
    fn parse_rejects_malformed_control_identities() {
        let (window, session) = (uuid_string(Uuid::new_v4()), uuid_string(Uuid::new_v4()));
        for bad in [
            format!("{window}:not-a-uuid:main"),
            format!("{window}:{session}:bogus"),
            format!("{window}:{session}:notify:1"),
            format!("{window}:not-a-uuid:notify"),
            format!("not-a-uuid:{session}:main"),
            format!("{session}:main"),
            "no-colon".to_owned(),
        ] {
            assert_eq!(parse_identity(&bad), None, "{bad}");
        }
    }

    #[test]
    fn a_post_takes_its_slots_identity() {
        let (window, session) = (Uuid::new_v4(), Uuid::new_v4());
        let post = |slot| NotificationPost {
            window,
            session,
            slot,
            title: "t".to_owned(),
            body: "b".to_owned(),
        };
        assert_eq!(post(BannerSlot::Control).identity(), control_identity(window, session));
        assert_eq!(
            post(BannerSlot::Pane(PaneRole::Main)).identity(),
            identity(window, session, PaneRole::Main)
        );
    }

    #[test]
    fn only_the_focused_pane_of_the_active_app_suppresses() {
        assert!(!should_deliver(true, true));
        assert!(should_deliver(true, false));
        assert!(should_deliver(false, true));
        assert!(should_deliver(false, false));
    }
}
