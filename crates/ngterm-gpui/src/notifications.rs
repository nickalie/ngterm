//! Desktop notifications (upstream's `Notifications/NotificationManager.swift`, `notifications.md`; port of the GTK
//! build's `notifications.rs`, which posted through `GNotification`) over the freedesktop notification server
//! (`org.freedesktop.Notifications` on the session bus) through `zbus`.
//!
//! The core records each notification (the badge and the `notify` event) and queues its banner; this posts the queued
//! banners, each under its identity: `<windowID>:<sessionID>:<pane>` for a terminal's, `<windowID>:<sessionID>:notify`
//! for a `notify` (the session's control slot, upstream #772). A repeat in one slot replaces the last one
//! (`replaces_id`), so a pane's banner never replaces a note and a newer note does; a click on a banner (its `default`
//! action, `ActionInvoked`) reveals the pane that fired (the main pane for a note) and raises its window with the
//! activation token the server sent just before (`ActivationToken`), as `GNotification`'s `activate-action` carries
//! it. Withdrawals are the core's explicit clears, queued with the posts and applied in that order
//! (`CloseNotification`): when a pane of the session takes focus and after a successful swap, as upstream's
//! `clearDelivered` call sites. A clear withdraws every pane's identity of that window and session and the control
//! slot's; a session no open window holds gets none, so a closed window's banners stay clickable and reopen it. A banner the server closes (it
//! expired, the user dismissed it) is forgotten. The badge clears on its own path and leaves the banners.
//!
//! The server sees no sound: a batch that posts banners sounds the configured notification sound as the display's
//! alert (Linux has none of upstream's named sounds), through the status sounds' throttled bell
//! ([`crate::status_sound`]); the badge and the event never depend on it.
//!
//! The banners go out in order on one task ([`run`]) over one session-bus connection of its own, which is also where
//! the server's signals arrive (a server may send `ActivationToken` to the connection that posted the banner alone);
//! the task routes them to their banners by server id. Those ids are the server's own: before one is reused (a repeat's
//! `replaces_id`, a withdrawal) the task asks the bus who owns the server's name now, and a reply from another server
//! than the last one tells the same, so the ids a server that restarted handed out are forgotten, never sent to its
//! successor (which numbers its banners afresh).

use crate::instance::Activation;
use crate::state::{AppState, Poster, post};
use ngterm_core::{NotificationEffect, parse_notification_identity};

#[cfg(target_os = "linux")]
mod freedesktop;
#[cfg(target_os = "linux")]
pub use freedesktop::run;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::run;

/// The application name banners carry (the server's grouping; GTK's `g_get_application_name`).
pub const APP_NAME: &str = "ngterm";

/// Where banners go: the notification server, or a recorder in tests.
pub trait BannerSink {
    /// Posts a banner under `identity`, replacing one posted under it before.
    fn send(&self, identity: &str, title: &str, body: &str);
    /// Withdraws the banner posted under `identity`, if any.
    fn withdraw(&self, identity: &str);
}

/// Applies the core's banner posts and withdrawals on `sink`, in the order the core queued them.
pub fn deliver(sink: &impl BannerSink, effects: &[NotificationEffect]) {
    for effect in effects {
        match effect {
            NotificationEffect::Post(post) => {
                tracing::info!(session = %post.session, "notify: posting banner");
                sink.send(&post.identity(), &post.title, &post.body);
            }
            NotificationEffect::App { identity, title, body } => {
                tracing::info!(%identity, "notify: posting app banner");
                sink.send(identity, title, body);
            }
            NotificationEffect::Clear { session, .. } => {
                tracing::debug!(%session, "notify: withdrawing the session's delivered banners");
                for identity in effect.withdrawn_identities() {
                    sink.withdraw(&identity);
                }
            }
        }
    }
}

/// The configured notification sound when `effects` post a banner (upstream attaches it to every banner it posts):
/// one alert per batch, however many banners it carries.
#[must_use]
pub fn banner_sound<'a>(
    effects: &[NotificationEffect],
    settings: &'a ngterm_core::persist::AppSettings,
) -> Option<&'a str> {
    let posts = effects
        .iter()
        .any(|effect| matches!(effect, NotificationEffect::Post(_) | NotificationEffect::App { .. }));
    settings
        .notification_sound_name
        .as_deref()
        .filter(|name| posts && !name.is_empty())
}

/// What the banner task does next, in the order the core asked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Banner {
    Send {
        identity: String,
        title: String,
        body: String,
    },
    Withdraw {
        identity: String,
    },
}

/// The notification server's side of [`BannerSink`]: the banners queue for [`run`].
pub struct Desktop {
    banners: async_channel::Sender<Banner>,
}

impl Desktop {
    /// A sink whose banners [`run`] posts, and the queue to hand it.
    #[must_use]
    pub fn new() -> (Self, async_channel::Receiver<Banner>) {
        let (banners, queue) = async_channel::unbounded();
        (Self { banners }, queue)
    }

    /// Posts the banners on `executor`, a click revealing its pane through `poster`, under the desktop entry `app_id`.
    #[must_use]
    pub fn start(app_id: String, poster: Poster, executor: &gpui_kit::BackgroundExecutor) -> Self {
        let (sink, queue) = Self::new();
        let clicked = move |click: Click| {
            post(&poster, move |state: &mut AppState| {
                state.reveal_notification(&click.identity, click.token);
            });
        };
        executor.spawn(run(queue, app_id, clicked)).detach();
        sink
    }

    fn queue(&self, banner: Banner) {
        if self.banners.try_send(banner).is_err() {
            tracing::debug!("notify: the banner task is gone");
        }
    }
}

impl BannerSink for Desktop {
    fn send(&self, identity: &str, title: &str, body: &str) {
        self.queue(Banner::Send {
            identity: identity.to_owned(),
            title: title.to_owned(),
            body: body.to_owned(),
        });
    }

    fn withdraw(&self, identity: &str) {
        self.queue(Banner::Withdraw {
            identity: identity.to_owned(),
        });
    }
}

/// A click on a banner: the identity it was posted under, and the activation token the server sent with the click
/// (`ActivationToken`: an xdg-activation token on Wayland, a startup id on X11), which raises the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Click {
    pub identity: String,
    pub token: Option<String>,
}

impl AppState {
    /// A banner was clicked: its pane is revealed (its closed window reopened, even when the session turned out gone)
    /// and its window raised with the server's activation `token` when it sent one (a compositor that prevents focus
    /// stealing may refuse a raise without one); a malformed identity only leaves the app up.
    pub fn reveal_notification(&mut self, identity: &str, token: Option<String>) {
        let Some((window, session, pane)) = parse_notification_identity(identity) else {
            return;
        };
        if self.core.reveal_notification(window, session, pane) {
            if let Some(token) = token {
                self.activation_requested(Activation {
                    token: Some(token),
                    startup_id: None,
                });
            }
            self.settle();
        }
    }
}

#[cfg(test)]
mod tests;
