//! X11, any window manager: the keyboard's XKB groups are the layouts, named as the keymap names them (the name GPUI
//! reports), read and locked through the XKB extension on a connection of this module's own to the display GPUI runs
//! on. A lock is the server's, seen by every client of the display (GPUI hears it as a layout change).

use std::error::Error;

use x11rb::connection::{Connection as _, RequestConnection as _};
use x11rb::protocol::xkb::{self, ConnectionExt as _, ID, NameDetail};
use x11rb::protocol::xproto::{ConnectionExt as _, ModMask};
use x11rb::rust_connection::RustConnection;

use super::super::{Backend, Failure, KeyboardLayout};

/// The core keyboard's device spec.
const CORE: u16 = 256;

/// The XKB groups of an X display's core keyboard.
pub struct Xkb {
    conn: RustConnection,
}

impl Xkb {
    /// Connects to `display` (`DISPLAY` when `None`) and opens its XKB extension.
    ///
    /// # Errors
    /// When the display cannot be reached or has no XKB.
    pub fn connect(display: Option<&str>) -> Result<Self, Failure> {
        let (conn, _) = x11rb::connect(display)?;
        if conn.extension_information(xkb::X11_EXTENSION_NAME)?.is_none() {
            return Err("the display has no XKB extension".into());
        }
        if !conn.xkb_use_extension(1, 0)?.reply()?.supported {
            return Err("the display's XKB version is not supported".into());
        }
        debug_assert_eq!(u16::from(ID::USE_CORE_KBD), CORE);
        Ok(Self { conn })
    }

    /// The groups' names, in group order.
    fn names(&self) -> Result<Vec<String>, Failure> {
        let groups = self.conn.xkb_get_controls(CORE)?.reply()?.num_groups;
        let names = self.conn.xkb_get_names(CORE, NameDetail::GROUP_NAMES)?.reply()?;
        let present = u8::from(names.group_names);
        let mut atoms = names.value_list.groups.unwrap_or_default().into_iter();
        (0..groups.min(4))
            .map(|group| {
                let atom = if present & (1 << group) == 0 {
                    None
                } else {
                    atoms.next()
                };
                match atom.filter(|atom| *atom != x11rb::NONE) {
                    Some(atom) => self.atom_name(atom),
                    None => Ok(format!("Group {}", group + 1)),
                }
            })
            .collect()
    }

    fn atom_name(&self, atom: u32) -> Result<String, Failure> {
        let name = self.conn.get_atom_name(atom)?.reply()?.name;
        Ok(String::from_utf8_lossy(&name).into_owned())
    }

    /// The effective group's index as the server holds it, which a new keymap with fewer groups leaves out of range
    /// until the next state change.
    fn group(&self) -> Result<usize, Failure> {
        Ok(usize::from(u8::from(self.conn.xkb_get_state(CORE)?.reply()?.group)))
    }

    /// Locks group `index`.
    ///
    /// # Errors
    /// When the server refused it.
    pub fn lock_group(&self, index: u8) -> Result<(), Failure> {
        let none = ModMask::from(0u16);
        self.conn
            .xkb_latch_lock_state(CORE, none, none, true, xkb::Group::from(index), none, false, 0)?
            .check()?;
        self.conn.flush()?;
        Ok(())
    }
}

impl Backend for Xkb {
    fn layouts(&mut self) -> Result<Vec<KeyboardLayout>, Failure> {
        Ok(self.names()?.into_iter().map(KeyboardLayout::named).collect())
    }

    fn current(&mut self) -> Result<String, Failure> {
        let names = self.names()?;
        let group = self.group()?;
        // an out-of-range group wraps into range, as XKB's default `GroupsWrap` (and xkbcommon, so GPUI) reads it
        group
            .checked_rem(names.len())
            .and_then(|group| names.into_iter().nth(group))
            .ok_or_else(|| -> Box<dyn Error + Send + Sync> { format!("no group {group}").into() })
    }

    fn select(&mut self, id: &str) -> Result<(), Failure> {
        let index = super::index_of(&self.names()?, id)?;
        self.lock_group(u8::try_from(index)?)
    }
}
