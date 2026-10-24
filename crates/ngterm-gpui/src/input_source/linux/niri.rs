//! niri (`NIRI_SOCKET`): one JSON request per line, one reply per line (niri-ipc): `"KeyboardLayouts"` for the XKB
//! names of the configured layouts and the current index, the `SwitchLayout` action with an `Index` target to select.

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;

use serde_json::{Value, json};

use super::super::{Backend, Failure, KeyboardLayout};

/// niri's IPC socket.
pub struct Niri {
    path: PathBuf,
}

impl Niri {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// One request, its reply's `Ok` value; an `Err` reply is the failure.
    fn request(&self, request: &Value) -> Result<Value, Failure> {
        let mut stream = super::connect(&self.path)?;
        stream.write_all(format!("{request}\n").as_bytes())?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line)?;
        let mut reply: Value = serde_json::from_str(&line)?;
        if let Some(error) = reply.get("Err") {
            return Err(format!("niri refused {request}: {error}").into());
        }
        reply
            .get_mut("Ok")
            .map(Value::take)
            .ok_or_else(|| format!("niri's reply: {line}").into())
    }

    /// The layouts' names and the current one's index.
    fn layouts_now(&self) -> Result<(Vec<String>, usize), Failure> {
        let reply = self.request(&json!("KeyboardLayouts"))?;
        let layouts = &reply["KeyboardLayouts"];
        let current = layouts["current_idx"]
            .as_u64()
            .and_then(|index| usize::try_from(index).ok())
            .ok_or("no current layout")?;
        Ok((super::names(&layouts["names"]), current))
    }
}

impl Backend for Niri {
    fn layouts(&mut self) -> Result<Vec<KeyboardLayout>, Failure> {
        Ok(self.layouts_now()?.0.into_iter().map(KeyboardLayout::named).collect())
    }

    fn current(&mut self) -> Result<String, Failure> {
        let (names, current) = self.layouts_now()?;
        names.into_iter().nth(current).ok_or_else(|| "no current layout".into())
    }

    fn select(&mut self, id: &str) -> Result<(), Failure> {
        let index = u8::try_from(super::index_of(&self.layouts_now()?.0, id)?)?;
        self.request(&json!({ "Action": { "SwitchLayout": { "layout": { "Index": index } } } }))?;
        Ok(())
    }
}
