//! KDE Plasma on Wayland: `KWin`'s keyboard layouts through the `org.kde.keyboard` service on the session bus
//! (`/Layouts`, `org.kde.KeyboardLayouts`): `getLayoutsList` (short name, display name, long name; the long name is the
//! XKB layout name, the display name the user's own label), `getLayout` for the current index, `setLayout` to select.

use std::time::Duration;

use zbus::blocking::{Connection, Proxy};
use zbus::proxy::CacheProperties;

use super::super::{Backend, Failure, KeyboardLayout};

const SERVICE: &str = "org.kde.keyboard";
const PATH: &str = "/Layouts";
const INTERFACE: &str = "org.kde.KeyboardLayouts";
/// How long one call may take: they run on the main thread.
const CALL_TIMEOUT: Duration = Duration::from_millis(500);

/// `KWin`'s layouts service.
pub struct Kde {
    connection: Connection,
}

impl Kde {
    /// The service on the session bus at `address` (the session bus the environment names when `None`), checked to
    /// answer.
    ///
    /// # Errors
    /// Without a session bus or the service.
    pub fn connect(address: Option<&str>) -> Result<Self, Failure> {
        let builder = match address {
            Some(address) => zbus::blocking::connection::Builder::address(address)?,
            None => zbus::blocking::connection::Builder::session()?,
        };
        let connection = builder.method_timeout(CALL_TIMEOUT).build()?;
        Self::over(connection)
    }

    /// The service over `connection`, checked to answer.
    ///
    /// # Errors
    /// Without the service.
    pub fn over(connection: Connection) -> Result<Self, Failure> {
        let kde = Self { connection };
        kde.names()?;
        Ok(kde)
    }

    fn proxy(&self) -> zbus::Result<Proxy<'static>> {
        zbus::blocking::proxy::Builder::new(&self.connection)
            .destination(SERVICE)?
            .path(PATH)?
            .interface(INTERFACE)?
            .cache_properties(CacheProperties::No)
            .build()
    }

    /// The layouts' names, in index order.
    fn names(&self) -> Result<Vec<String>, Failure> {
        let list: Vec<(String, String, String)> = self.proxy()?.call("getLayoutsList", &())?;
        Ok(list
            .into_iter()
            .map(|(short, _, long)| if long.is_empty() { short } else { long })
            .collect())
    }
}

impl Backend for Kde {
    fn layouts(&mut self) -> Result<Vec<KeyboardLayout>, Failure> {
        Ok(self.names()?.into_iter().map(KeyboardLayout::named).collect())
    }

    fn current(&mut self) -> Result<String, Failure> {
        let index: u32 = self.proxy()?.call("getLayout", &())?;
        let names = self.names()?;
        names
            .into_iter()
            .nth(usize::try_from(index)?)
            .ok_or_else(|| format!("no layout {index}").into())
    }

    fn select(&mut self, id: &str) -> Result<(), Failure> {
        let index = u32::try_from(super::index_of(&self.names()?, id)?)?;
        let selected: bool = self.proxy()?.call("setLayout", &(index,))?;
        if selected {
            Ok(())
        } else {
            Err(format!("KDE refused layout {index}").into())
        }
    }
}
