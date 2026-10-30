//! The panes with no window: every session pane of every open window gets a terminal (an `ngterm-term` [`Pane`] of a
//! fixed grid, drawn by nobody) whose [`Surface`] sits in the pane's slot, so the core sees each pane realized and its
//! live session's daemon running. What the GPUI frontend's deck does for its pages (`ngterm-gpui`'s `deck.rs` and
//! `pane.rs`), less everything a window shows.
//!
//! - Eager, not lazy: every session wakes ([`Core::wake_session`]), since a session is offered to a viewer only once
//!   every pane of it has a running daemon (`ngterm_core::remote::tree`), and nobody here ever shows one.
//! - A pane's terminal asks for its seed once mounted ([`Core::request_pane_spawn`]) and starts with it, or once the
//!   core grants it ([`Core::take_spawn_grants`]): a live pane attaches to its daemon in process. Its client leads the
//!   pty size only while no viewer does: a viewer's attach claims the lead (`ngterm_core::live::lead`), and the grid
//!   here is whatever the last lead left until then.
//! - A pane the model dropped is torn down; a replacement the core asks for (a live restart, a lead takeover) is a
//!   fresh terminal in the same slot ([`Core::take_pane_replacements`]).
//! - The terminal's events reach the core as the deck's reports do: title, working directory, notification, exit.
//!   The rest (bell, clipboard, colors, progress, search) has nothing to show it here.
//!
//! The scratch terminal, overlays and the HUD are drawn things and are not mounted: a command that waits for one
//! answers as for an unrealized surface.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::str::FromStr as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ngterm_core::ghostty_config::GhosttyConfig;
use ngterm_core::ghostty_config::binds::{Action, KeyAction, Mods, PhysicalKey};
use ngterm_core::keystrokes::{self, KeystrokeSegment, SUBMIT_GAP};
use ngterm_core::model::{AppStore, Session, StatusPane};
use ngterm_core::pane_env::{self, SessionPane};
use ngterm_core::restore::spawn_plan::LaunchSeed;
use ngterm_core::{Core, PaneReplacement, PaneReport, PaneSpawn};
use ngterm_host::{Surface, TextScope};
use ngterm_term::{
    CursorDefaults, ExitStatus, GridSize, Host, KeyInput, Launch, Pane, PaneConfig, PaneEvent, SelectionText,
    TextRegion, ThemeColors, Wake,
};
use uuid::Uuid;

/// Where a pane's wake posts its drain: the main loop's state, through the frontend's poster.
pub type DrainPoster = Arc<dyn Fn(Uuid, String) + Send + Sync>;

/// The grid every terminal starts at: a viewer that leads resizes the pty to its own, and the terminal here keeps
/// this one (it only reads the screen, through the daemon once a viewer leads).
pub const DEFAULT_GRID: (u16, u16) = (80, 24);

/// The cell the grid reports to programs (`TIOCSWINSZ`'s pixel size), in pixels.
const CELL: (u32, u32) = (8, 16);

/// What every shell of this instance is told about it, and how its terminals start.
pub struct PaneEnvironment {
    /// `NGTERM_SOCKET`: the owned socket, or `<socket>.unavailable`.
    pub socket: String,
    /// `TERM_PROGRAM_VERSION`.
    pub program_version: String,
    /// The terminal identity and shell integration a pane launches under.
    pub host: Host,
    /// The ghostty config the terminals start with.
    pub config: GhosttyConfig,
    pub grid: (u16, u16),
    /// The local host's name, which a working directory report must name (or none).
    pub hostname: String,
}

/// What a pane's terminal ran: a command rather than the login shell, held after it exits (`ngterm-gpui`'s `Ran`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Ran {
    command: bool,
    wait: bool,
}

/// One pane's terminal, shared by the registry and the [`Surface`] in the pane's slot.
struct Cell_ {
    token: String,
    pane: RefCell<Option<Rc<Pane>>>,
    live: Cell<bool>,
    ran: Cell<Ran>,
    /// The points the terminal reports as its font size.
    points: f32,
    /// A drain is queued on the main loop: the IO thread's wakes until it runs post nothing more.
    drain_queued: Arc<AtomicBool>,
}

/// The core's surface over a [`Cell_`].
struct HeadlessSurface(Rc<Cell_>);

impl HeadlessSurface {
    fn pane(&self) -> Option<Rc<Pane>> {
        self.0.pane.borrow().clone()
    }
}

/// Sends `text` as one key press typing it (an unidentified key), as the GPUI surface types a run.
fn type_run(pane: &Pane, text: &str) {
    let key = KeyInput {
        unshifted: None,
        ..KeyInput::press(PhysicalKey::UNIDENTIFIED, Mods::empty(), text)
    };
    if let Err(error) = pane.key(&key) {
        tracing::warn!(%error, "typing into a headless terminal failed");
    }
}

/// One Return keypress, press then release, through the key encoder.
fn press_return(pane: &Pane) {
    let Some(enter) = PhysicalKey::new("enter") else {
        return;
    };
    let press = KeyInput::press(enter, Mods::empty(), "");
    let release = KeyInput {
        action: KeyAction::Release,
        ..press
    };
    for key in [press, release] {
        if let Err(error) = pane.key(&key) {
            tracing::warn!(%error, "typing Return into a headless terminal failed");
        }
    }
}

impl Surface for HeadlessSurface {
    fn pane_token(&self) -> &str {
        &self.0.token
    }

    fn is_realized(&self) -> bool {
        self.0.pane.borrow().is_some()
    }

    fn is_live_session(&self) -> bool {
        self.0.live.get() && self.is_realized()
    }

    fn promote_to_primary(&mut self) {}

    /// The keyboard's path, as the GPUI surface's `type_text`: each run as key text, each line ending as Return, a
    /// final Return after [`SUBMIT_GAP`].
    fn type_text(&mut self, text: &str) -> bool {
        let Some(pane) = self.pane() else {
            return false;
        };
        let paced = keystrokes::paced(text);
        for segment in &paced.head {
            match segment {
                KeystrokeSegment::Text(run) => type_run(&pane, run),
                KeystrokeSegment::Return => press_return(&pane),
            }
        }
        if paced.paced_return {
            std::thread::sleep(SUBMIT_GAP);
            press_return(&pane);
        }
        true
    }

    fn read_text(&self, scope: TextScope) -> Option<String> {
        let region = match scope {
            TextScope::Screen => TextRegion::Viewport,
            TextScope::All => TextRegion::All,
        };
        self.pane()?.read_text(region).ok()
    }

    fn read_selection(&self) -> Option<String> {
        self.pane()?
            .selection_text(SelectionText::Raw)
            .filter(|text| !text.is_empty())
    }

    fn cursor_column(&self) -> Option<u32> {
        Some(u32::from(self.pane()?.cursor_column()))
    }

    /// The actions that write to the program (`text:`, `csi:`, `esc:` and their kin) are performed; the rest act on a
    /// view this terminal has not, and are accepted as performed by nothing, as an idle one is in a window.
    fn binding_action(&mut self, action: &str) -> bool {
        let Some(pane) = self.pane() else {
            return false;
        };
        match Action::from_str(action) {
            Ok(action) => {
                if let Some(bytes) = action.pty_bytes()
                    && pane.send(&bytes).is_err()
                {
                    tracing::debug!("a headless terminal's input queue refused a binding's bytes");
                }
            }
            Err(error) => tracing::warn!(action, ?error, "unknown binding action"),
        }
        true
    }

    fn foreground_pid(&self) -> Option<u32> {
        self.pane()?.foreground_pid()
    }

    fn font_size(&self) -> Option<f64> {
        self.is_realized().then_some(f64::from(self.0.points))
    }

    fn cell_size(&self) -> Option<(f64, f64)> {
        self.is_realized().then_some((f64::from(CELL.0), f64::from(CELL.1)))
    }

    fn teardown(self: Box<Self>) {
        self.0.pane.borrow_mut().take();
    }
}

/// Which pane of which session a terminal is.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    session: Uuid,
    token: String,
}

/// A session pane the model lays out: its identity and what its terminal starts with.
#[derive(Debug, Clone, PartialEq)]
pub struct PaneSpec {
    pub window: Uuid,
    pub workspace: Uuid,
    pub session: Uuid,
    pub pane: StatusPane,
    pub token: String,
    pub cwd: String,
}

impl PaneSpec {
    /// The panes of `session` in window `window`'s `store`: its primary, and its split while it has one (shown or
    /// not: a hidden split keeps its terminal, as in a window).
    #[must_use]
    pub fn of_session(window: Uuid, store: &AppStore, session: &Session) -> Vec<Self> {
        let Some(workspace) = store
            .workspace_for_session(session.id())
            .map(ngterm_core::model::Workspace::id)
        else {
            return Vec::new();
        };
        let spec = |pane: StatusPane, identity: Uuid| Self {
            window,
            workspace,
            session: session.id(),
            pane,
            token: pane_env::uuid_string(identity),
            cwd: session.cwd_for(pane).to_owned(),
        };
        let mut specs = vec![Self {
            cwd: session.effective_cwd().to_owned(),
            ..spec(StatusPane::Left, session.pane_identity)
        }];
        if let Some(split) = session.split_pane_identity {
            specs.push(spec(StatusPane::Right, split));
        }
        specs
    }

    /// The pane's environment: the session, window and workspace ids, `NGTERM_PANE` and the pane token.
    fn env(&self, environment: &PaneEnvironment) -> Vec<(String, String)> {
        pane_env::session(&SessionPane {
            session: self.session,
            window: Some(self.window),
            workspace: Some(self.workspace),
            socket_path: &environment.socket,
            program_version: &environment.program_version,
            pane: Some(self.pane),
            pane_token: Some(&self.token),
        })
        .into_iter()
        .collect()
    }
}

/// Every session pane of the open windows, as the model lays them out.
#[must_use]
pub fn layout(core: &Core) -> Vec<PaneSpec> {
    let library = core.library();
    library
        .open_ids()
        .into_iter()
        .filter_map(|window| Some((window, library.store(window)?)))
        .flat_map(|(window, store)| {
            store
                .sessions()
                .flat_map(|session| PaneSpec::of_session(window, store, session))
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The registry of the terminals (module docs).
pub struct Panes {
    environment: PaneEnvironment,
    drain: DrainPoster,
    cells: HashMap<Key, Rc<Cell_>>,
    specs: HashMap<Key, PaneSpec>,
}

impl Panes {
    #[must_use]
    pub fn new(environment: PaneEnvironment, drain: DrainPoster) -> Self {
        Self {
            environment,
            drain,
            cells: HashMap::new(),
            specs: HashMap::new(),
        }
    }

    /// How many terminals are mounted.
    #[must_use]
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Brings the terminals in line with the model: the replacements the core asked for, the panes it dropped torn
    /// down, the ones it lays out mounted, every session woken, and the seeds granted handed to their terminals.
    /// Whether the model changed (a replacement dropped), which the caller settles.
    pub fn sync(&mut self, core: &mut Core) -> bool {
        let changed = self.replace(core);
        let wanted = layout(core);
        let held: std::collections::HashSet<Uuid> = core
            .library()
            .open_ids()
            .into_iter()
            .filter_map(|window| core.library().store(window))
            .flat_map(AppStore::pending_held_session_ids)
            .collect();
        self.retain(&wanted, &held);
        let mut fresh = Vec::new();
        for spec in wanted {
            core.wake_session(spec.session);
            let key = Key {
                session: spec.session,
                token: spec.token.clone(),
            };
            if !self.cells.contains_key(&key) {
                self.mount(core, &spec);
                fresh.push(spec);
            }
        }
        for spec in fresh {
            if let Some(seed) = core.request_pane_spawn(spec.session, &spec.token) {
                self.launch_seeded(&spec.session, &spec.token, &seed);
            }
        }
        for grant in core.take_spawn_grants() {
            let PaneSpawn { session, token, seed } = grant;
            self.launch_seeded(&session, &token, &seed);
        }
        changed
    }

    /// Tears down the terminals no pane the model lays out carries (a session held by a pending close keeps its
    /// terminals until the close is final or undone).
    fn retain(&mut self, wanted: &[PaneSpec], held: &std::collections::HashSet<Uuid>) {
        let keep = |key: &Key| {
            held.contains(&key.session)
                || wanted
                    .iter()
                    .any(|spec| spec.session == key.session && spec.token == key.token)
        };
        let gone: Vec<Key> = self.cells.keys().filter(|key| !keep(key)).cloned().collect();
        for key in gone {
            if let Some(cell) = self.cells.remove(&key) {
                cell.pane.borrow_mut().take();
            }
            self.specs.remove(&key);
        }
    }

    /// A gated terminal for `spec`, its surface in the pane's slot.
    fn mount(&mut self, core: &mut Core, spec: &PaneSpec) {
        let cell = self.cell(&spec.token);
        let session = core
            .library_mut()
            .store_mut(spec.window)
            .and_then(|store| store.session_mut(spec.session));
        if let Some(session) = session
            && let Some(previous) = session
                .pane_slot_mut(spec.pane)
                .replace(Box::new(HeadlessSurface(Rc::clone(&cell))))
        {
            tracing::error!(session = %spec.session, pane = spec.pane.as_str(), "a pane got a second terminal");
            previous.teardown();
        }
        let key = Key {
            session: spec.session,
            token: spec.token.clone(),
        };
        self.cells.insert(key.clone(), cell);
        self.specs.insert(key, spec.clone());
    }

    fn cell(&self, token: &str) -> Rc<Cell_> {
        Rc::new(Cell_ {
            token: token.to_owned(),
            pane: RefCell::new(None),
            live: Cell::new(false),
            ran: Cell::new(Ran::default()),
            points: self.environment.config.font.size,
            drain_queued: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Starts the terminal of `session`'s pane `token` with the core's `seed`.
    fn launch_seeded(&self, session: &Uuid, token: &str, seed: &LaunchSeed) {
        let key = Key {
            session: *session,
            token: token.to_owned(),
        };
        let (Some(cell), Some(spec)) = (self.cells.get(&key), self.specs.get(&key)) else {
            tracing::warn!(%session, token, "a granted pane has no terminal");
            return;
        };
        let launch = Launch::from_seed(seed, Some(PathBuf::from(&spec.cwd)), &spec.env(&self.environment));
        self.start(cell, *session, &launch, seed.live);
    }

    /// Starts `cell`'s terminal running `launch`.
    fn start(&self, cell: &Rc<Cell_>, session: Uuid, launch: &Launch, live: bool) {
        cell.ran.set(Ran {
            command: launch.command.is_some(),
            wait: launch.wait_after_command,
        });
        cell.live.set(live);
        let config = self.pane_config(session, cell);
        match Pane::launch(launch, &self.environment.host, &config) {
            Ok(pane) => {
                *cell.pane.borrow_mut() = Some(Rc::new(pane));
                tracing::info!(%session, token = cell.token, live, "headless terminal started");
            }
            Err(error) => tracing::error!(%session, token = cell.token, %error, "a headless terminal did not start"),
        }
    }

    /// The pane's start: the config's colors, cursor and scrollback on the fixed grid, its wake posting one drain at a
    /// time.
    fn pane_config(&self, session: Uuid, cell: &Rc<Cell_>) -> PaneConfig {
        let config = &self.environment.config;
        let (cols, rows) = self.environment.grid;
        let mut pane = PaneConfig::new(GridSize {
            cols,
            rows,
            cell_width: CELL.0,
            cell_height: CELL.1,
        });
        pane.scrollback = config.scrollback.into();
        pane.colors = ThemeColors::from_config(&config.colors, &config.cursor);
        pane.cursor = CursorDefaults::from_config(&config.cursor);
        let (drain, queued, token) = (
            Arc::clone(&self.drain),
            Arc::clone(&cell.drain_queued),
            cell.token.clone(),
        );
        let wake: Wake = Arc::new(move || {
            if !queued.swap(true, Ordering::AcqRel) {
                drain(session, token.clone());
            }
        });
        pane.wake = Some(wake);
        pane
    }

    /// Puts a fresh terminal in place of each pane the core asked to replace (the GPUI deck's `replace_pane`).
    fn replace(&mut self, core: &mut Core) -> bool {
        let mut changed = false;
        for replacement in core.take_pane_replacements() {
            if self.replace_one(core, &replacement) {
                core.pane_replaced(&replacement);
            } else {
                tracing::warn!(session = %replacement.session, token = replacement.token, "no pane to replace");
                changed |= core.replacement_dropped(&replacement);
            }
        }
        changed
    }

    fn replace_one(&mut self, core: &mut Core, replacement: &PaneReplacement) -> bool {
        let key = Key {
            session: replacement.session,
            token: replacement.token.clone(),
        };
        let Some(spec) = self.specs.get(&key).cloned() else {
            return false;
        };
        let mut env = spec.env(&self.environment);
        for (name, value) in &replacement.env {
            match env.iter_mut().find(|(known, _)| known == name) {
                Some(entry) => entry.1.clone_from(value),
                None => env.push((name.clone(), value.clone())),
            }
        }
        let launch = Launch {
            command: Some(replacement.command.clone()),
            initial_input: None,
            working_directory: Some(PathBuf::from(
                replacement.cwd.clone().unwrap_or_else(|| spec.cwd.clone()),
            )),
            wait_after_command: replacement.remote,
            env,
            env_remove: Vec::new(),
            live: replacement.attach.clone(),
        };
        let fresh = self.cell(&spec.token);
        let installed = core
            .library_mut()
            .store_mut(spec.window)
            .and_then(|store| store.session_mut(spec.session))
            .map(|session| {
                session
                    .pane_slot_mut(spec.pane)
                    .replace(Box::new(HeadlessSurface(Rc::clone(&fresh))))
            });
        let Some(previous) = installed else {
            return false;
        };
        if let Some(previous) = previous {
            previous.teardown();
        }
        self.start(&fresh, spec.session, &launch, !replacement.remote);
        if let Some(old) = self.cells.insert(key, fresh) {
            old.pane.borrow_mut().take();
        }
        true
    }

    /// Takes the events of `session`'s pane `token` since its last drain, as the reports the core takes.
    #[must_use]
    pub fn drain(&self, session: Uuid, token: &str) -> Vec<Report> {
        let key = Key {
            session,
            token: token.to_owned(),
        };
        let Some(cell) = self.cells.get(&key) else {
            return Vec::new();
        };
        cell.drain_queued.store(false, Ordering::Release);
        let Some(pane) = cell.pane.borrow().clone() else {
            return Vec::new();
        };
        let ran = cell.ran.get();
        pane.take_events()
            .iter()
            .filter_map(|event| report(event, ran, &self.environment.hostname))
            .collect()
    }

    /// Drops every terminal (the quit): each live client detaches, and its daemon keeps the session.
    pub fn teardown_all(&mut self) {
        for cell in self.cells.values() {
            cell.pane.borrow_mut().take();
        }
        self.cells.clear();
        self.specs.clear();
    }
}

/// What a terminal event asks of the model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Report {
    Pane(PaneReport),
    Notification {
        title: String,
        body: String,
    },
    /// The program ended: the pane closes by its role.
    Exited,
    /// A held command ended (a remote replica's ssh): the core decides what stays.
    Held,
}

/// What `event` reports, for a terminal that ran `ran` on the machine named `hostname`. An exit held on screen
/// closes at once unless it was a held command: no key will ever close it here.
fn report(event: &PaneEvent, ran: Ran, hostname: &str) -> Option<Report> {
    match event {
        PaneEvent::Title(title) => Some(Report::Pane(PaneReport::Title(title.clone()))),
        PaneEvent::Pwd(raw) => {
            ngterm_term::pwd::reported_path(raw, hostname).map(|path| Report::Pane(PaneReport::Pwd(path)))
        }
        PaneEvent::Notification { title, body } => Some(Report::Notification {
            title: title.clone(),
            body: body.clone(),
        }),
        PaneEvent::Exited(ExitStatus { held: true, .. }) if ran.command && ran.wait => Some(Report::Held),
        PaneEvent::Exited(_) => Some(Report::Exited),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
