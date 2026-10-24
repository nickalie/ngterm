//! sway, and the wlroots compositors that speak its IPC (`SWAYSOCK`): the keyboards' layouts from `GET_INPUTS` (the
//! first keyboard with layouts: `xkb_layout_names`, `xkb_active_layout_index`), selected on each keyboard that has the
//! layout with `input "<identifier>" xkb_switch_layout <index>` at that keyboard's own index, as per-device configs may
//! list the layouts in different orders (sway-ipc(7), sway-input(5)).

use std::io::{Read as _, Write as _};
use std::path::PathBuf;

use serde_json::Value;

use super::super::{Backend, Failure, KeyboardLayout};

/// The message types (sway-ipc(7)).
const RUN_COMMAND: u32 = 0;
const GET_INPUTS: u32 = 100;
/// Every message's magic string.
const MAGIC: &[u8; 6] = b"i3-ipc";

/// sway's IPC socket.
pub struct Sway {
    path: PathBuf,
}

impl Sway {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// One message of type `kind`, its reply's payload.
    fn request(&self, kind: u32, payload: &str) -> Result<Value, Failure> {
        let mut stream = super::connect(&self.path)?;
        let mut message = MAGIC.to_vec();
        message.extend(u32::try_from(payload.len())?.to_ne_bytes());
        message.extend(kind.to_ne_bytes());
        message.extend(payload.as_bytes());
        stream.write_all(&message)?;
        let mut header = [0u8; 14];
        stream.read_exact(&mut header)?;
        if &header[..6] != MAGIC {
            return Err("not an i3-ipc reply".into());
        }
        let length = u32::from_ne_bytes([header[6], header[7], header[8], header[9]]);
        let mut body = vec![0u8; usize::try_from(length)?];
        stream.read_exact(&mut body)?;
        Ok(serde_json::from_slice(&body)?)
    }

    /// The keyboards that have layouts, in `GET_INPUTS` order: each one's identifier, layout names and the active
    /// one's index.
    fn keyboards(&self) -> Result<Vec<(String, Vec<String>, usize)>, Failure> {
        let inputs = self.request(GET_INPUTS, "")?;
        Ok(inputs
            .as_array()
            .into_iter()
            .flatten()
            .filter(|input| input["type"] == "keyboard")
            .map(|keyboard| {
                (
                    keyboard["identifier"].as_str().unwrap_or_default().to_owned(),
                    super::names(&keyboard["xkb_layout_names"]),
                    keyboard["xkb_active_layout_index"]
                        .as_u64()
                        .and_then(|index| usize::try_from(index).ok())
                        .unwrap_or(0),
                )
            })
            .filter(|(_, names, _)| !names.is_empty())
            .collect())
    }

    /// The first keyboard's layout names and the active one's index.
    fn keyboard(&self) -> Result<(Vec<String>, usize), Failure> {
        let (_, names, active) = self.keyboards()?.into_iter().next().ok_or("no keyboard with layouts")?;
        Ok((names, active))
    }
}

/// `identifier` as a quoted sway command argument.
fn quoted(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Backend for Sway {
    fn layouts(&mut self) -> Result<Vec<KeyboardLayout>, Failure> {
        Ok(self.keyboard()?.0.into_iter().map(KeyboardLayout::named).collect())
    }

    fn current(&mut self) -> Result<String, Failure> {
        let (names, active) = self.keyboard()?;
        names.into_iter().nth(active).ok_or_else(|| "no active layout".into())
    }

    fn select(&mut self, id: &str) -> Result<(), Failure> {
        let commands: Vec<String> = self
            .keyboards()?
            .iter()
            .filter_map(|(identifier, names, _)| {
                let index = names.iter().position(|name| name == id)?;
                Some(format!("input {} xkb_switch_layout {index}", quoted(identifier)))
            })
            .collect();
        if commands.is_empty() {
            return Err(format!("no layout named {id:?}").into());
        }
        let replies = self.request(RUN_COMMAND, &commands.join("; "))?;
        let failed = replies
            .as_array()
            .into_iter()
            .flatten()
            .find(|reply| reply["success"] != true);
        match failed {
            Some(reply) => Err(format!("sway refused the switch: {}", reply["error"]).into()),
            None => Ok(()),
        }
    }
}
