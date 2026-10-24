//! Hyprland (`HYPRLAND_INSTANCE_SIGNATURE`): one command per connection to its request socket, the reply read to the
//! end (hyprctl's protocol). `j/devices` gives the main keyboard's XKB rule names and its active layout's name;
//! `switchxkblayout <device> <index>` selects. Hyprland lists no layout names, so they come from compiling the
//! keyboard's rule names with xkbcommon, as Hyprland compiles them for its keymap.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};

use serde_json::Value;
use xkbcommon::xkb;

use super::super::{Backend, Failure, KeyboardLayout};

/// The request sockets an instance may serve: under the runtime directory (Hyprland 0.40 on), under `/tmp` before.
#[must_use]
pub fn sockets(signature: &Path, runtime_dir: Option<&Path>) -> Vec<PathBuf> {
    let under = |root: &Path| root.join("hypr").join(signature).join(".socket.sock");
    runtime_dir
        .into_iter()
        .map(under)
        .chain([under(Path::new("/tmp"))])
        .collect()
}

/// The main keyboard as `j/devices` describes it.
struct Keyboard {
    name: String,
    /// Its XKB rule names: rules, model, layout, variant, options.
    rmlvo: [String; 5],
    active: String,
}

/// Hyprland's request socket.
pub struct Hyprland {
    path: PathBuf,
    /// The last rule names compiled and their layouts' names.
    compiled: Option<([String; 5], Vec<String>)>,
}

impl Hyprland {
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self { path, compiled: None }
    }

    /// One command, its reply.
    fn request(&self, command: &str) -> Result<String, Failure> {
        let mut stream = super::connect(&self.path)?;
        stream.write_all(command.as_bytes())?;
        let mut reply = String::new();
        stream.read_to_string(&mut reply)?;
        Ok(reply)
    }

    /// The main keyboard (the first one where none is marked main).
    fn keyboard(&self) -> Result<Keyboard, Failure> {
        let devices: Value = serde_json::from_str(&self.request("j/devices")?)?;
        let keyboards = devices["keyboards"].as_array().ok_or("no keyboards")?;
        let keyboard = keyboards
            .iter()
            .find(|keyboard| keyboard["main"] == true)
            .or_else(|| keyboards.first())
            .ok_or("no keyboard")?;
        let text = |key: &str| keyboard[key].as_str().unwrap_or_default().to_owned();
        Ok(Keyboard {
            name: text("name"),
            rmlvo: ["rules", "model", "layout", "variant", "options"].map(text),
            active: text("active_keymap"),
        })
    }

    /// The names of the layouts `keyboard`'s rule names compile to.
    fn names(&mut self, keyboard: &Keyboard) -> Result<Vec<String>, Failure> {
        if let Some((rmlvo, names)) = &self.compiled
            && *rmlvo == keyboard.rmlvo
        {
            return Ok(names.clone());
        }
        let names = layout_names(&keyboard.rmlvo)?;
        self.compiled = Some((keyboard.rmlvo.clone(), names.clone()));
        Ok(names)
    }
}

/// The layout names XKB's rule names compile to.
///
/// # Errors
/// When they do not compile.
pub fn layout_names(rmlvo: &[String; 5]) -> Result<Vec<String>, Failure> {
    let [rules, model, layout, variant, options] = rmlvo;
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let options = (!options.is_empty()).then(|| options.clone());
    let keymap = xkb::Keymap::new_from_names(&context, rules, model, layout, variant, options, xkb::COMPILE_NO_FLAGS)
        .ok_or_else(|| format!("the layouts {layout:?} ({variant:?}) do not compile"))?;
    Ok((0..keymap.num_layouts())
        .map(|index| keymap.layout_get_name(index).to_owned())
        .collect())
}

impl Backend for Hyprland {
    fn layouts(&mut self) -> Result<Vec<KeyboardLayout>, Failure> {
        let keyboard = self.keyboard()?;
        Ok(self.names(&keyboard)?.into_iter().map(KeyboardLayout::named).collect())
    }

    fn current(&mut self) -> Result<String, Failure> {
        Ok(self.keyboard()?.active)
    }

    fn select(&mut self, id: &str) -> Result<(), Failure> {
        let keyboard = self.keyboard()?;
        let index = super::index_of(&self.names(&keyboard)?, id)?;
        let reply = self.request(&format!("switchxkblayout {} {index}", keyboard.name))?;
        if reply.trim() == "ok" {
            Ok(())
        } else {
            Err(format!("Hyprland refused the switch: {}", reply.trim()).into())
        }
    }
}
