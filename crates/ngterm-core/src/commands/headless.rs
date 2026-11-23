//! What a headless origin (`ngterm-headless`) refuses before dispatch, as upstream's agterm-server does: the commands
//! that drive a window on the machine showing it (upstream's `HeadlessCommandPolicy`, `CatalogRoot.swift`), a
//! `--wait` it could never end (`ServerActions+Sessions.swift`), and a GUI ask or a pick with no session to bind it to
//! (`ServerActions+Ask.swift`, `ServerActions+Pick.swift`). Upstream's strings name `ngterm-headless` here
//! (conformance/docs/rust-build.md > Renamed strings); "a window on the Mac" reads "a window on the machine".

use ngterm_proto::command::{
    BrowserCmd, ConfigCmd, FontCmd, KeymapCmd, QuickCmd, SessionCmd, SidebarCmd, SurfaceCmd, ThemeCmd, WindowCmd,
};
use ngterm_proto::{Command, Request};

use crate::Core;
use crate::error::ControlError;

/// The commands only a window can carry out (upstream's `physicalWindowCommands`).
pub(crate) const PHYSICAL_WINDOW_COMMANDS: [Command; 36] = [
    Command::Window(WindowCmd::New),
    Command::Window(WindowCmd::Select),
    Command::Window(WindowCmd::Go),
    Command::Window(WindowCmd::Close),
    Command::Window(WindowCmd::Rename),
    Command::Window(WindowCmd::Delete),
    Command::Window(WindowCmd::Resize),
    Command::Window(WindowCmd::Move),
    Command::Window(WindowCmd::Zoom),
    Command::Window(WindowCmd::Fullscreen),
    Command::Window(WindowCmd::Minimize),
    Command::Window(WindowCmd::RemoteOpen),
    Command::Sidebar(SidebarCmd::Visibility),
    Command::Sidebar(SidebarCmd::Mode),
    Command::Sidebar(SidebarCmd::FlaggedLayout),
    Command::Sidebar(SidebarCmd::Density),
    Command::Sidebar(SidebarCmd::Expand),
    Command::Sidebar(SidebarCmd::Collapse),
    Command::Sidebar(SidebarCmd::Width),
    Command::Font(FontCmd::Inc),
    Command::Font(FontCmd::Dec),
    Command::Font(FontCmd::Reset),
    Command::Quick(QuickCmd::Visibility),
    Command::Quick(QuickCmd::Type),
    Command::Quick(QuickCmd::Text),
    Command::Dashboard,
    Command::Surface(SurfaceCmd::Zoom),
    Command::Surface(SurfaceCmd::Cursor),
    Command::Theme(ThemeCmd::Set),
    Command::Theme(ThemeCmd::List),
    Command::Keymap(KeymapCmd::Reload),
    Command::Keymap(KeymapCmd::List),
    Command::Keymap(KeymapCmd::Run),
    Command::Config(ConfigCmd::Reload),
    Command::Browser(BrowserCmd::Clear),
    Command::Browser(BrowserCmd::Links),
];

/// A GUI ask with no `--target` on a headless origin (upstream's `openAsk`).
pub(crate) const ASK_NEEDS_TARGET: &str =
    "ask.open needs --target on ngterm-headless, which has no window to show it in";
/// A pick with no `--session` on a headless origin (upstream's `openPick`).
pub(crate) const PICK_NEEDS_SESSION: &str =
    "pick.open needs --session on ngterm-headless, which has no window to show it in";

/// The refusal of a command that drives a window, on a headless origin; `None` for any other or on an app with windows.
/// Checked before dispatch, as upstream's server refuses it before its store sees it.
pub(crate) fn window_refusal(core: &Core, request: &Request) -> Option<ControlError> {
    let cmd = request.cmd;
    (core.presenter_only() && PHYSICAL_WINDOW_COMMANDS.contains(&cmd)).then(|| {
        ControlError::rejected(format!(
            "{cmd} is not available on ngterm-headless: it drives a window on the machine showing it"
        ))
    })
}

/// A `--wait` session or companion on a headless origin, which runs no window to hold the pane in (upstream's
/// `createSession`, `createCompanion`); checked after the command's own validation.
pub(crate) fn refuse_wait(core: &Core, cmd: SessionCmd, wait: Option<bool>) -> Result<(), ControlError> {
    if !core.presenter_only() || wait != Some(true) {
        return Ok(());
    }
    Err(ControlError::rejected(if cmd == SessionCmd::CompanionNew {
        "session.companion.new --wait is not available on ngterm-headless"
    } else {
        "session.new --wait is not available on ngterm-headless"
    }))
}
