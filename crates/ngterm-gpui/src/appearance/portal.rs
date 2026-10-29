//! The XDG settings portal's `org.freedesktop.appearance` `color-scheme` over `zbus`, Linux's appearance source (the
//! parent module's docs say how it is read and followed).

use std::time::Duration;

use zbus::blocking::{Connection, Proxy};
use zbus::proxy::CacheProperties;
use zbus::zvariant::{OwnedValue, Value};

/// The portal's bus name, object path and settings interface.
pub const PORTAL: &str = "org.freedesktop.portal.Desktop";
pub const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
pub const SETTINGS_INTERFACE: &str = "org.freedesktop.portal.Settings";
/// The setting: its namespace and key.
pub const NAMESPACE: &str = "org.freedesktop.appearance";
pub const KEY: &str = "color-scheme";

/// How long a portal call may take: the launch waits for the first read, which may start the portal.
const CALL_TIMEOUT: Duration = Duration::from_secs(3);

/// The portal's `color-scheme` value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ColorScheme {
    /// 0, and any value the specification does not define.
    #[default]
    NoPreference,
    /// 1.
    PreferDark,
    /// 2.
    PreferLight,
}

impl ColorScheme {
    /// The scheme of the portal's value `value`.
    #[must_use]
    pub fn from_value(value: u32) -> Self {
        match value {
            1 => Self::PreferDark,
            2 => Self::PreferLight,
            _ => Self::NoPreference,
        }
    }

    /// The scheme a setting's `value` carries: a `u32`, possibly wrapped in variants (the deprecated `Read` wraps it
    /// twice); `None` for any other type.
    #[must_use]
    pub fn from_variant(value: &Value<'_>) -> Option<Self> {
        match value {
            Value::U32(raw) => Some(Self::from_value(*raw)),
            Value::Value(inner) => Self::from_variant(inner),
            _ => None,
        }
    }

    /// Whether the desktop prefers dark: no preference is light, as libadwaita reads it.
    #[must_use]
    pub fn is_dark(self) -> bool {
        self == Self::PreferDark
    }
}

/// A connection to the desktop's settings portal.
pub struct Portal {
    connection: Connection,
}

impl Portal {
    /// The portal on the session bus; `None` (logged) without one.
    #[must_use]
    pub fn session() -> Option<Self> {
        let connected = zbus::blocking::connection::Builder::session()
            .map(|builder| builder.method_timeout(CALL_TIMEOUT))
            .and_then(zbus::blocking::connection::Builder::build);
        match connected {
            Ok(connection) => Some(Self::over(connection)),
            Err(error) => {
                tracing::info!(%error, "appearance: no session bus; the desktop reads as light");
                None
            }
        }
    }

    /// The portal reached over `connection`.
    #[must_use]
    pub fn over(connection: Connection) -> Self {
        Self { connection }
    }

    fn proxy(&self) -> zbus::Result<Proxy<'static>> {
        zbus::blocking::proxy::Builder::new(&self.connection)
            .destination(PORTAL)?
            .path(PORTAL_PATH)?
            .interface(SETTINGS_INTERFACE)?
            .cache_properties(CacheProperties::No)
            .build()
    }

    /// The desktop's color scheme now: `ReadOne`, else the deprecated `Read` of portals before version 2.
    ///
    /// # Errors
    ///
    /// The portal is not there, does not have the setting, or answered with another type.
    pub fn color_scheme(&self) -> zbus::Result<ColorScheme> {
        let proxy = self.proxy()?;
        let value: OwnedValue = match proxy.call("ReadOne", &(NAMESPACE, KEY)) {
            Ok(value) => value,
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" =>
            {
                proxy.call("Read", &(NAMESPACE, KEY))?
            }
            Err(error) => return Err(error),
        };
        ColorScheme::from_variant(&value).ok_or_else(|| zbus::Error::Failure(format!("{NAMESPACE} {KEY}: {value:?}")))
    }

    /// Whether the desktop prefers dark now; light when the portal cannot say (logged).
    #[must_use]
    pub fn is_dark(&self) -> bool {
        match self.color_scheme() {
            Ok(scheme) => {
                tracing::info!(?scheme, "appearance: the desktop's color scheme");
                scheme.is_dark()
            }
            Err(error) => {
                tracing::info!(%error, "appearance: the settings portal has no color scheme; the desktop reads as light");
                false
            }
        }
    }

    /// Reports every color scheme the portal announces from now on to `report`, on a thread of its own, starting with
    /// one read made after subscribing. The thread ends with the connection.
    ///
    /// # Errors
    ///
    /// The thread could not start.
    pub fn follow(self, report: impl Fn(ColorScheme) + Send + 'static) -> std::io::Result<std::thread::JoinHandle<()>> {
        std::thread::Builder::new()
            .name("ngterm-appearance".to_owned())
            .spawn(move || self.listen(&report))
    }

    fn listen(&self, report: &dyn Fn(ColorScheme)) {
        let changes = self
            .proxy()
            .and_then(|proxy| proxy.receive_signal_with_args("SettingChanged", &[(0, NAMESPACE), (1, KEY)]));
        let changes = match changes {
            Ok(changes) => changes,
            Err(error) => {
                tracing::warn!(%error, "appearance: cannot follow the settings portal");
                return;
            }
        };
        if let Ok(scheme) = self.color_scheme() {
            report(scheme);
        }
        for message in changes {
            let body = message.body();
            match body.deserialize::<(String, String, OwnedValue)>() {
                Ok((_, _, value)) => {
                    if let Some(scheme) = ColorScheme::from_variant(&value) {
                        report(scheme);
                    } else {
                        tracing::debug!(?value, "appearance: a color scheme of another type");
                    }
                }
                Err(error) => tracing::debug!(%error, "appearance: an unreadable setting change"),
            }
        }
        tracing::debug!("appearance: the settings portal's signals ended");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_portals_values_and_their_wrapping() {
        assert_eq!(ColorScheme::from_value(0), ColorScheme::NoPreference);
        assert_eq!(ColorScheme::from_value(1), ColorScheme::PreferDark);
        assert_eq!(ColorScheme::from_value(2), ColorScheme::PreferLight);
        assert_eq!(
            ColorScheme::from_value(7),
            ColorScheme::NoPreference,
            "an undefined value"
        );
        assert!(ColorScheme::PreferDark.is_dark());
        assert!(!ColorScheme::PreferLight.is_dark());
        assert!(
            !ColorScheme::NoPreference.is_dark(),
            "no preference is light, as libadwaita reads it"
        );
        assert_eq!(ColorScheme::from_variant(&Value::U32(1)), Some(ColorScheme::PreferDark));
        // the deprecated `Read` wraps the value in one more variant
        let wrapped = Value::Value(Box::new(Value::Value(Box::new(Value::U32(2)))));
        assert_eq!(ColorScheme::from_variant(&wrapped), Some(ColorScheme::PreferLight));
        assert_eq!(ColorScheme::from_variant(&Value::from("dark")), None);
    }
}
