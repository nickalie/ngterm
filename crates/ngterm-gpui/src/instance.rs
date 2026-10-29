//! One running ngterm per application id (plan > Solution Overview > Single instance). This build's id
//! ([`DEFAULT_APP_ID`]) is its own, not upstream's, so it never forwards to an agterm instance; instances of different
//! ids keep apart through separate state directories (`ngterm_core::paths`).
//!
//! A launch that finds a primary running forwards to it and exits before GPUI starts and before it reads or writes the
//! model state (`windows.json`, the window files, the settings), so two launches of one id never both write them; it
//! only appends its lines to the shared `ngterm.log` (logging starts first, and rotates the file only under the running
//! instance's lock). The mechanism is the platform's:
//!
//! - Linux ([`gapplication`]): `GApplication`'s own D-Bus protocol on the session bus; a launch with no session bus
//!   runs as a primary of its own, as `GApplication` does.
//! - Windows ([`windows`]): a named pipe per app id and state directory, owner-only (plan > Decisions > Single
//!   instance); the forwarding launch lets the primary take the foreground (`AllowSetForegroundWindow`).
//!
//! The activation token and the startup id are Linux's: GPUI consumes `XDG_ACTIVATION_TOKEN` for its first toplevel
//! only and removes it from the environment when its platform is created, so [`Activation::from_env`] reads the token
//! and the startup id first thing in `main`. A primary ends its own X11 startup sequence once its window exists
//! (`activation::launched`), and `main` removes `DESKTOP_STARTUP_ID` from the environment, as GDK does, so no child
//! inherits it. On Windows neither is set, and an activation carries nothing.

use std::error::Error;

use ngterm_core::paths::Env;

#[cfg(target_os = "linux")]
pub mod gapplication;
#[cfg(target_os = "linux")]
pub use gapplication::{Primary, Remote, register};
#[cfg(windows)]
pub mod windows;
#[cfg(windows)]
pub use windows::{Primary, Remote, register};

/// The application id this build registers.
pub const DEFAULT_APP_ID: &str = "com.nickalie.ngterm";
/// The variable that gives a dev or test instance an id of its own (`scripts/run-gpui-dev.sh`, the conformance
/// harness).
pub const APP_ID_ENV: &str = "NGTERM_APP_ID";
/// The xdg-activation token a launcher hands a Wayland launch.
pub const ACTIVATION_TOKEN_ENV: &str = "XDG_ACTIVATION_TOKEN";
/// The startup-notification id a launcher hands an X11 launch (its `_TIME<n>` suffix is the launching event's time).
pub const STARTUP_ID_ENV: &str = "DESKTOP_STARTUP_ID";

/// `NGTERM_APP_ID` when `GApplication` would accept it, else [`DEFAULT_APP_ID`].
#[must_use]
pub fn app_id(env: &dyn Env) -> String {
    env.var(APP_ID_ENV)
        .and_then(|id| id.into_string().ok())
        .filter(|id| id_is_valid(id))
        .unwrap_or_else(|| DEFAULT_APP_ID.to_owned())
}

/// `GApplication`'s validity rule (`g_application_id_is_valid`): at most 255 bytes, at least two `.`-separated
/// elements, each non-empty, of `[A-Za-z0-9_-]` and not starting with a digit.
#[must_use]
pub fn id_is_valid(id: &str) -> bool {
    let element_is_valid = |element: &str| {
        element.bytes().next().is_some_and(|first| !first.is_ascii_digit())
            && element
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    };
    id.len() <= 255 && id.split('.').count() >= 2 && id.split('.').all(element_is_valid)
}

/// What a launch hands the primary to raise its window with: the Wayland activation token and the X11 startup id.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Activation {
    pub token: Option<String>,
    pub startup_id: Option<String>,
}

impl Activation {
    /// This launch's token and startup id. Call it before GPUI's platform exists, which removes the token.
    #[must_use]
    pub fn from_env(env: &dyn Env) -> Self {
        let read = |name| {
            env.var(name)
                .and_then(|value| value.into_string().ok())
                .filter(|value| !value.is_empty())
        };
        Self {
            token: read(ACTIVATION_TOKEN_ENV),
            startup_id: read(STARTUP_ID_ENV),
        }
    }
}

/// What another launch asked of the primary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BusRequest {
    /// `org.gtk.Application.Activate` (or `Open`), or a forward over the instance pipe: raise the frontmost window.
    Activate(Activation),
}

/// Where the instance's handlers hand requests: drained into the main loop once the app runs.
pub type BusRequests = async_channel::Sender<BusRequest>;

/// How this launch ended up registered.
pub enum Registration {
    /// This process is the primary for the app id: it runs the app. The value must live as long as the process.
    Primary(Primary),
    /// Another process is the primary: this launch forwards to it and exits.
    Remote(Remote),
    /// No way to tell (no session bus): the app runs without single-instance (`GApplication`'s behavior).
    Unique(Box<dyn Error + Send + Sync>),
}

/// The registration failed past reaching the platform's service: the launch cannot tell whether another instance
/// runs.
#[derive(Debug, thiserror::Error)]
#[error("failed to register {app_id} {place}: {source}")]
pub struct RegisterError {
    app_id: String,
    /// Where: "on the session bus", "on its instance pipe".
    place: &'static str,
    #[source]
    source: Box<dyn Error + Send + Sync>,
}

impl RegisterError {
    fn new(app_id: &str, place: &'static str, source: impl Into<Box<dyn Error + Send + Sync>>) -> Self {
        Self {
            app_id: app_id.to_owned(),
            place,
            source: source.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use ngterm_core::paths::MapEnv;

    use super::*;

    #[test]
    fn the_app_id_honours_a_valid_override_only() {
        assert_eq!(app_id(&MapEnv::new()), "com.nickalie.ngterm");
        let env = MapEnv::new().with(APP_ID_ENV, "com.nickalie.ngterm.dev.i1");
        assert_eq!(app_id(&env), "com.nickalie.ngterm.dev.i1");
        for invalid in [
            "",
            "ngterm",
            "io..ngterm",
            "io.github.9agterm",
            ".io.ngterm",
            "io.ngterm.",
            "io.agt erm",
            "io.agtérm",
        ] {
            assert_eq!(
                app_id(&MapEnv::new().with(APP_ID_ENV, invalid)),
                DEFAULT_APP_ID,
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn validity_follows_gapplication() {
        assert!(id_is_valid("org.example.my-app"));
        assert!(id_is_valid("_a._b"));
        assert!(id_is_valid(&format!("a.{}", "b".repeat(253))));
        assert!(!id_is_valid(&format!("a.{}", "b".repeat(254))));
        assert!(!id_is_valid("a.1b"));
    }

    #[test]
    fn a_launch_reads_its_token_and_startup_id_and_skips_empty_ones() {
        let env = MapEnv::new()
            .with(ACTIVATION_TOKEN_ENV, "tok")
            .with(STARTUP_ID_ENV, "id_TIME42");
        assert_eq!(
            Activation::from_env(&env),
            Activation {
                token: Some("tok".to_owned()),
                startup_id: Some("id_TIME42".to_owned()),
            }
        );
        let empty = MapEnv::new().with(ACTIVATION_TOKEN_ENV, "").with(STARTUP_ID_ENV, "");
        assert_eq!(Activation::from_env(&empty), Activation::default());
    }
}
