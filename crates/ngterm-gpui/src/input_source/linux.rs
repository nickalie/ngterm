//! The keyboard layouts on Linux, per display server: the stored id is the XKB layout name, the name GPUI reports
//! for the layout (`keyboard_layout()`), so a layout reads the same whichever backend switched it.
//!
//! - X11, any window manager: XKB's group lock on the display GPUI runs on ([`x11`]).
//! - Wayland has no common protocol for it, so each compositor's own control: sway's IPC (`SWAYSOCK`, [`sway`]),
//!   Hyprland's (`HYPRLAND_INSTANCE_SIGNATURE`, [`hyprland`]), niri's (`NIRI_SOCKET`, [`niri`]), and on KDE Plasma
//!   the `org.kde.keyboard` service on the session bus ([`kde`]), when `KWin` (or the wrapper Plasma starts it
//!   through) serves GPUI's Wayland display. A compositor socket is used only when the process serving it is the one
//!   serving GPUI's Wayland display (their peer credentials), so a variable inherited from another session never
//!   switches that session's layout. GNOME (no supported API since `Shell.Eval` was
//!   restricted), weston and any other compositor: the setting is disabled with the reason shown.

pub mod hyprland;
pub mod kde;
pub mod niri;
pub mod sway;
pub mod x11;

use std::io;
use std::os::fd::AsRawFd as _;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{Backend, Probe};

/// Why GNOME has no lock.
pub const GNOME_UNAVAILABLE: &str =
    "Not available on GNOME: it offers applications no way to switch the keyboard layout.";
/// Why any other desktop without a backend has none.
pub const DESKTOP_UNAVAILABLE: &str =
    "Not available on this desktop: it offers applications no way to switch the keyboard layout.";

/// How long one request to a compositor may take: they run on the main thread.
const IPC_TIMEOUT: Duration = Duration::from_millis(500);

/// The backend for the display server GPUI runs on (`compositor`, GPUI's name for it).
///
/// # Errors
/// Why the setting is not available there, for the picker to show.
pub fn probe(compositor: &str) -> Probe {
    match compositor {
        "X11" => x11::Xkb::connect(None)
            .map(|backend| Box::new(backend) as Box<dyn Backend>)
            .map_err(|error| format!("Not available: the X server's keyboard extension did not answer ({error}).")),
        "Wayland" => wayland(&|name| std::env::var_os(name).map(PathBuf::from)),
        _ => Err(DESKTOP_UNAVAILABLE.to_owned()),
    }
}

/// The Wayland compositor's backend, from the environment `var` reads.
///
/// # Errors
/// Why the setting is not available on that compositor, for the picker to show.
pub fn wayland(var: &dyn Fn(&str) -> Option<PathBuf>) -> Probe {
    let compositor = wayland_socket(var).and_then(|path| socket_peer(&path));
    let serves = |path: &Path| compositor.is_some() && socket_peer(path) == compositor;
    if let Some(path) = var("SWAYSOCK").filter(|path| serves(path)) {
        return Ok(Box::new(sway::Sway::new(path)));
    }
    if let Some(signature) = var("HYPRLAND_INSTANCE_SIGNATURE")
        && let Some(path) = hyprland::sockets(&signature, var("XDG_RUNTIME_DIR").as_deref())
            .into_iter()
            .find(|path| serves(path))
    {
        return Ok(Box::new(hyprland::Hyprland::new(path)));
    }
    if let Some(path) = var("NIRI_SOCKET").filter(|path| serves(path)) {
        return Ok(Box::new(niri::Niri::new(path)));
    }
    if compositor.and_then(process_name).is_some_and(|name| is_kwin(&name)) {
        let bus = var("DBUS_SESSION_BUS_ADDRESS");
        match kde::Kde::connect(bus.as_deref().and_then(Path::to_str)) {
            Ok(kde) => return Ok(Box::new(kde)),
            Err(error) => tracing::info!(%error, "keyboard layout lock: KDE's keyboard service did not answer"),
        }
    }
    let desktop = var("XDG_CURRENT_DESKTOP").map(|desktop| desktop.to_string_lossy().to_ascii_uppercase());
    if desktop.is_some_and(|desktop| desktop.split(':').any(|name| name == "GNOME")) {
        return Err(GNOME_UNAVAILABLE.to_owned());
    }
    Err(DESKTOP_UNAVAILABLE.to_owned())
}

/// The Wayland display's socket, as `wayland-client` resolves `WAYLAND_DISPLAY`.
fn wayland_socket(var: &dyn Fn(&str) -> Option<PathBuf>) -> Option<PathBuf> {
    let display = var("WAYLAND_DISPLAY").unwrap_or_else(|| PathBuf::from("wayland-0"));
    if display.is_absolute() {
        return Some(display);
    }
    Some(var("XDG_RUNTIME_DIR")?.join(display))
}

/// The process serving the socket at `path`, by the credentials it connected with.
fn socket_peer(path: &Path) -> Option<i32> {
    let stream = UnixStream::connect(path).ok()?;
    peer_pid(&stream)
}

/// The peer process of a connected Unix socket.
fn peer_pid(stream: &UnixStream) -> Option<i32> {
    let mut credentials = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut length = libc::socklen_t::try_from(std::mem::size_of::<libc::ucred>()).ok()?;
    // SAFETY: the descriptor is the open stream's, and `credentials` and `length` are live locals of the size passed.
    let status = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&raw mut credentials).cast(),
            &raw mut length,
        )
    };
    (status == 0 && credentials.pid > 0).then_some(credentials.pid)
}

/// The command name of process `pid`.
fn process_name(pid: i32) -> Option<String> {
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(name.trim_end().to_owned())
}

/// Whether `name` (a process's command name) is `KWin`'s: `kwin_wayland` itself, or the `kwin_wayland_wrapper` Plasma
/// starts it through (Plasma 5.23 and later), which owns the Wayland socket and hands it to `kwin_wayland`, so it is
/// the socket's peer; the kernel cuts the name to 15 bytes (`kwin_wayland_wr`).
fn is_kwin(name: &str) -> bool {
    name.starts_with("kwin_wayland")
}

/// A connection to a compositor's socket, with the request deadline.
fn connect(path: &Path) -> io::Result<UnixStream> {
    let stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(IPC_TIMEOUT))?;
    stream.set_write_timeout(Some(IPC_TIMEOUT))?;
    Ok(stream)
}

/// The index of the layout named `id` among `names`.
fn index_of(names: &[String], id: &str) -> Result<usize, super::Failure> {
    names
        .iter()
        .position(|name| name == id)
        .ok_or_else(|| format!("no layout named {id:?}").into())
}

/// A JSON array's strings.
fn names(list: &serde_json::Value) -> Vec<String> {
    list.as_array()
        .into_iter()
        .flatten()
        .filter_map(|name| name.as_str().map(str::to_owned))
        .collect()
}

#[cfg(test)]
mod tests;
