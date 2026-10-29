//! The single instance on Linux: `GApplication`'s own D-Bus protocol on the session bus.
//!
//! The primary owns the app id's well-known name on the session bus and serves, at `GApplication`'s object path,
//! `org.gtk.Application` (`Activate` with platform data: the only call a default-flags GTK launch makes; `Open` the
//! same) and `org.gtk.Actions` (`DescribeAll`, which a non-primary `GApplication` calls while it registers and fails
//! without, and `Activate`). A launch that finds the name owned sends `Activate` with its own platform data and exits
//! (the parent module's docs say when). As `GApplication` does, a launch with no session bus runs as a primary of its
//! own.
//!
//! The activation token travels in the platform data: a plain `GApplication` forwards no token (`GtkApplication`
//! does), so it is read from the environment first thing in `main` ([`Activation::from_env`]).

use std::collections::HashMap;
use std::path::Path;

use zbus::blocking::Connection;
use zbus::fdo::{RequestNameFlags, RequestNameReply};
use zbus::message::{Flags, Message};
use zbus::zvariant::{OwnedValue, Signature, Value};

use super::{Activation, BusRequest, BusRequests, RegisterError, Registration};

/// The primary's registration: its session bus connection, which holds the name.
pub type Primary = Connection;

/// Where a failed registration happened, for [`RegisterError`].
const PLACE: &str = "on the session bus";

/// The platform data keys `GtkApplication` sends and reads.
const TOKEN_KEY: &str = "activation-token";
const STARTUP_ID_KEY: &str = "desktop-startup-id";
const CWD_KEY: &str = "cwd";

/// `GApplication`'s object path for `app_id` (`application_path_from_appid`): `/` + the id, `.` → `/`, `-` → `_`.
#[must_use]
pub fn object_path(app_id: &str) -> String {
    format!("/{}", app_id.replace('.', "/").replace('-', "_"))
}

impl Activation {
    /// What a primary was handed in `Activate`'s platform data; values of another type count as absent.
    #[must_use]
    pub fn from_platform_data(data: &HashMap<String, OwnedValue>) -> Self {
        let string = |key: &str| {
            data.get(key)
                .and_then(|value| <&str>::try_from(&**value).ok())
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        Self {
            token: string(TOKEN_KEY),
            startup_id: string(STARTUP_ID_KEY),
        }
    }

    /// The platform data a forwarding launch sends: `GApplication`'s `cwd` (a NUL-terminated bytestring) and
    /// `GtkApplication`'s two keys. Unlike `GtkApplication`, which sends one value under both keys, each key carries what
    /// this launch was given.
    #[must_use]
    pub fn platform_data(&self, cwd: Option<&Path>) -> HashMap<&'static str, Value<'static>> {
        let mut data = HashMap::new();
        if let Some(cwd) = cwd {
            let mut bytes = std::os::unix::ffi::OsStrExt::as_bytes(cwd.as_os_str()).to_vec();
            bytes.push(0);
            data.insert(CWD_KEY, Value::from(bytes));
        }
        if let Some(token) = &self.token {
            data.insert(TOKEN_KEY, Value::from(token.clone()));
        }
        if let Some(startup_id) = &self.startup_id {
            data.insert(STARTUP_ID_KEY, Value::from(startup_id.clone()));
        }
        data
    }
}

/// `GApplication`'s `org.gtk.Application` on the primary.
struct ApplicationIface {
    requests: BusRequests,
}

#[zbus::interface(name = "org.gtk.Application")]
impl ApplicationIface {
    #[expect(clippy::needless_pass_by_value, reason = "zbus hands a method its arguments owned")]
    fn activate(&self, platform_data: HashMap<String, OwnedValue>) {
        let activation = Activation::from_platform_data(&platform_data);
        let mut keys: Vec<&str> = platform_data.keys().map(String::as_str).collect();
        keys.sort_unstable();
        tracing::info!(
            ?keys,
            token = activation.token.as_deref(),
            startup_id = activation.startup_id.as_deref(),
            "instance: activation requested over the bus"
        );
        if self.requests.try_send(BusRequest::Activate(activation)).is_err() {
            tracing::debug!("instance: activation dropped, the app is gone");
        }
    }

    /// Files are not ngterm's: an `Open` raises the window as an `Activate` does.
    #[expect(clippy::needless_pass_by_value, reason = "zbus hands a method its arguments owned")]
    fn open(&self, uris: Vec<String>, hint: String, platform_data: HashMap<String, OwnedValue>) {
        tracing::info!(uris = uris.len(), hint, "instance: open requested over the bus");
        self.activate(platform_data);
    }
}

/// `GApplication`'s `org.gtk.Actions` on the primary: no exported actions yet.
struct ActionsIface;

#[zbus::interface(name = "org.gtk.Actions")]
impl ActionsIface {
    #[expect(clippy::unused_self, reason = "a zbus method takes the interface")]
    fn list(&self) -> Vec<String> {
        Vec::new()
    }

    #[expect(clippy::unused_self, reason = "a zbus method takes the interface")]
    fn describe_all(&self) -> HashMap<String, (bool, Signature, Vec<OwnedValue>)> {
        HashMap::new()
    }

    #[expect(
        clippy::unused_self,
        clippy::needless_pass_by_value,
        reason = "a zbus method takes the interface and its arguments owned"
    )]
    fn activate(&self, action_name: String, parameter: Vec<OwnedValue>, platform_data: HashMap<String, OwnedValue>) {
        tracing::info!(
            action_name,
            parameters = parameter.len(),
            platform_data = platform_data.len(),
            "instance: unknown action activated over the bus"
        );
    }
}

/// Registers `app_id` on the session bus the way `GApplication` does: the interfaces are exported first, so a
/// racing launch never meets the name without them, then the name is requested without queueing. The state directory
/// plays no part: the bus name is the app id's alone, and instances of different ids keep apart through their state
/// directories.
///
/// # Errors
///
/// The bus refused the name request or the export; a missing bus is [`Registration::Unique`] instead.
pub fn register(app_id: &str, state_dir: &Path, requests: BusRequests) -> Result<Registration, RegisterError> {
    let _ = state_dir;
    let failed = |source: zbus::Error| RegisterError::new(app_id, PLACE, source);
    let connection = match connect(app_id, requests) {
        Ok(connection) => connection,
        Err(Connect::NoBus(error)) => return Ok(Registration::Unique(Box::new(error))),
        Err(Connect::Export(error)) => return Err(failed(error)),
    };
    let remote = |connection| {
        Registration::Remote(Remote {
            connection,
            app_id: app_id.to_owned(),
        })
    };
    match connection.request_name_with_flags(app_id, RequestNameFlags::DoNotQueue.into()) {
        Ok(RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner) => Ok(Registration::Primary(connection)),
        // zbus reports the bus's `Exists` reply as this error
        Ok(RequestNameReply::Exists | RequestNameReply::InQueue) | Err(zbus::Error::NameTaken) => {
            Ok(remote(connection))
        }
        Err(error) => Err(failed(error)),
    }
}

enum Connect {
    NoBus(zbus::Error),
    Export(zbus::Error),
}

/// A session bus connection serving both interfaces at `app_id`'s object path, no name requested yet. A connection
/// that cannot be made at all means no bus; a failure after it is a registration failure.
fn connect(app_id: &str, requests: BusRequests) -> Result<Connection, Connect> {
    let connection = zbus::blocking::connection::Builder::session()
        .and_then(zbus::blocking::connection::Builder::build)
        .map_err(Connect::NoBus)?;
    let path = object_path(app_id);
    let server = connection.object_server();
    server
        .at(path.as_str(), ApplicationIface { requests })
        .and_then(|_| server.at(path.as_str(), ActionsIface))
        .map_err(Connect::Export)?;
    drop(server);
    Ok(connection)
}

/// The primary another process runs, as a forwarding launch reaches it.
pub struct Remote {
    connection: Connection,
    app_id: String,
}

impl Remote {
    /// Sends `org.gtk.Application.Activate` with this launch's platform data, expecting no reply (`GApplication`'s
    /// remote activation is fire-and-forget: the primary's answer never holds the launch up).
    ///
    /// # Errors
    ///
    /// The message could not be built or written to the bus.
    pub fn activate(&self, activation: &Activation, cwd: Option<&Path>) -> zbus::Result<()> {
        let path = object_path(&self.app_id);
        let message = Message::method_call(path.as_str(), "Activate")?
            .destination(self.app_id.as_str())?
            .interface("org.gtk.Application")?
            .with_flags(Flags::NoReplyExpected)?
            .build(&(activation.platform_data(cwd),))?;
        self.connection.send(&message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_object_path_is_gapplications() {
        assert_eq!(object_path("com.nickalie.ngterm"), "/com/nickalie/ngterm");
        assert_eq!(object_path("org.example.my-app"), "/org/example/my_app");
    }

    #[test]
    fn platform_data_round_trips_and_carries_the_cwd_as_a_bytestring() {
        let activation = Activation {
            token: Some("tok".to_owned()),
            startup_id: Some("sid".to_owned()),
        };
        let data = activation.platform_data(Some(Path::new("/srv")));
        assert_eq!(data[CWD_KEY], Value::from(b"/srv\0".to_vec()));
        let received: HashMap<String, OwnedValue> = data
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value.try_into().unwrap()))
            .collect();
        assert_eq!(Activation::from_platform_data(&received), activation);
        assert!(Activation::default().platform_data(None).is_empty());
    }

    #[test]
    fn values_of_another_type_count_as_absent() {
        let mut data: HashMap<String, OwnedValue> = HashMap::new();
        data.insert(TOKEN_KEY.to_owned(), Value::from(7_u32).try_into().unwrap());
        data.insert(STARTUP_ID_KEY.to_owned(), Value::from("").try_into().unwrap());
        assert_eq!(Activation::from_platform_data(&data), Activation::default());
    }
}
