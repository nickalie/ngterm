//! A phone of the e2e test: a [`MobileCore`] over russh with the test's key and the `sshd`'s host key pinned, what
//! its events said, and reading a shown pane's screen from the packed row buffer.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ngterm_mobile::{
    ChatEvent, ChatState, EventSink, HostKeyStore, KeyVault, Machine, MachineState, MobileCore, MobileEvent, Pane,
    PaneEnd, PaneKey, PaneKeyAction, PaneRole, PaneSize, RemoteLayout, VaultError,
};

use crate::machine::{World, account};

/// How long any one step may take.
pub(crate) const WAIT: Duration = Duration::from_secs(45);
pub(crate) const MACHINE_ID: &str = "e2e";
pub(crate) const KEY_ID: &str = "e2e-key";
pub(crate) const GRID: PaneSize = PaneSize {
    cols: 80,
    rows: 24,
    cell_width: 10,
    cell_height: 20,
};
/// The line the test runs in the pane: the same in sh, bash, zsh and fish (the account's login shell runs the pane).
pub(crate) const TYPED: &str = "printf 'e2e-%s\\n' 42";
/// What [`TYPED`] prints, which the typed line itself doesn't hold.
pub(crate) const PRINTED: &str = "e2e-42";

/// What a client's events said so far.
#[derive(Default)]
pub(crate) struct Seen {
    pub(crate) layout: Option<RemoteLayout>,
    pub(crate) role: Option<PaneRole>,
    /// Whether the last `PaneLead` covered the pane.
    pub(crate) covered: bool,
    /// Every role a `PaneLead` gave, in order.
    pub(crate) roles: Vec<Option<PaneRole>>,
    pub(crate) attached: usize,
    pub(crate) lost: usize,
    pub(crate) states: Vec<MachineState>,
    /// Every `PaneAgent`, in order: the pane, its agent and its conversation.
    pub(crate) agents: Vec<(String, Option<String>, Option<String>)>,
    /// Every chat event, in order, of every batch.
    pub(crate) chat: Vec<ChatEvent>,
    pub(crate) chat_states: Vec<ChatState>,
}

impl std::fmt::Debug for Seen {
    /// The chat as its count and last few events: a failure's message stays readable with a backlog seen.
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let recent = &self.chat[self.chat.len().saturating_sub(4)..];
        formatter
            .debug_struct("Seen")
            .field("layout", &self.layout)
            .field("role", &self.role)
            .field("covered", &self.covered)
            .field("roles", &self.roles)
            .field("attached", &self.attached)
            .field("lost", &self.lost)
            .field("states", &self.states)
            .field("agents", &self.agents)
            .field("chat_events", &self.chat.len())
            .field("chat_recent", &recent)
            .field("chat_states", &self.chat_states)
            .finish()
    }
}

impl Seen {
    pub(crate) fn record(&mut self, event: MobileEvent) {
        match event {
            MobileEvent::SessionLayout { layout, .. } => self.layout = Some(layout),
            MobileEvent::PaneLead { role, covered, .. } => {
                self.role = role;
                self.covered = covered;
                self.roles.push(role);
            }
            MobileEvent::PaneAttached { .. } => self.attached += 1,
            MobileEvent::PaneEnded {
                end: PaneEnd::Lost { .. },
                ..
            } => self.lost += 1,
            MobileEvent::MachineState { state, .. } => self.states.push(state),
            MobileEvent::PaneAgent {
                pane_id,
                agent,
                conversation_id,
                ..
            } => self.agents.push((pane_id, agent, conversation_id)),
            MobileEvent::Chat { events, .. } => self.chat.extend(events),
            MobileEvent::ChatState { state, .. } => self.chat_states.push(state),
            _ => {}
        }
    }

    /// Where the chat's last event with `id` is.
    pub(crate) fn chat_at(&self, id: &str) -> Option<usize> {
        self.chat.iter().rposition(|event| event.id == id)
    }

    /// Whether the chat holds an event with each of `ids`.
    pub(crate) fn chat_holds(&self, ids: &[String]) -> bool {
        ids.iter().all(|id| self.chat_at(id).is_some())
    }
}

/// One phone: a [`MobileCore`] over russh, with the test's key and the `sshd`'s host key pinned.
pub(crate) struct Client {
    pub(crate) label: &'static str,
    pub(crate) core: Arc<MobileCore>,
    pub(crate) events: Receiver<MobileEvent>,
    pub(crate) seen: Seen,
    /// The shown pane's id, and its handle.
    pub(crate) pane: Option<String>,
    pub(crate) terminal: Option<Arc<Pane>>,
}

impl Client {
    pub(crate) fn new(label: &'static str, world: &World) -> Self {
        Self::with_keys(label, &world.key.private_key, &world.host_fingerprint)
    }

    /// A phone holding `private_key`, with the machine's host key `fingerprint` pinned.
    pub(crate) fn with_keys(label: &'static str, private_key: &[u8], fingerprint: &str) -> Self {
        let (sender, events) = mpsc::channel();
        let core = MobileCore::new(
            Arc::new(Sink(Mutex::new(sender))),
            Arc::new(Vault(private_key.to_vec())),
            Arc::new(Pinned(fingerprint.to_owned())),
        )
        .expect("a core");
        Self {
            label,
            core,
            events,
            seen: Seen::default(),
            pane: None,
            terminal: None,
        }
    }

    pub(crate) fn connect(&self, port: u16) {
        let machine = Machine {
            id: MACHINE_ID.to_owned(),
            label: "e2e machine".to_owned(),
            host: "127.0.0.1".to_owned(),
            port,
            user: account(),
            key_id: KEY_ID.to_owned(),
        };
        block_on(self.core.connect(machine)).unwrap_or_else(|error| panic!("{}: connect: {error:?}", self.label));
    }

    /// The machine's sessions over ssh, as (id, name) pairs.
    pub(crate) fn list(&self) -> Vec<(String, String)> {
        block_on(self.core.list_sessions(MACHINE_ID.to_owned()))
            .unwrap_or_else(|error| panic!("{}: list: {error:?}", self.label))
            .into_iter()
            .map(|session| (session.id, session.name))
            .collect()
    }

    /// Takes in events until `done` holds of what they said.
    pub(crate) fn wait(&mut self, what: &str, done: impl Fn(&Seen) -> bool) {
        let deadline = Instant::now() + WAIT;
        while !done(&self.seen) {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(event) => self.seen.record(event),
                Err(error) => panic!(
                    "{}: no {what} within {WAIT:?} ({error}); seen {:?}",
                    self.label, self.seen
                ),
            }
        }
    }

    /// Takes in the events already sent. A step drains before it reads a count of [`Seen`] or clears a value its wait
    /// then looks at, so an event an earlier step caused cannot land after that and satisfy the wait.
    pub(crate) fn drain(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            self.seen.record(event);
        }
    }

    /// Takes control of the shown pane and waits for that takeover's own report: its attach's role-less `PaneLead`,
    /// then `Leader`. A role an earlier attach reported says nothing of this one, which replaces the pane's attach
    /// even when it already leads; input typed before the new attach leads goes down the old one and is dropped once
    /// the new one claims.
    pub(crate) fn take_lead(&mut self, session: &str, pane: &str) {
        self.drain();
        let roles = self.seen.roles.len();
        self.core
            .take_lead(MACHINE_ID.to_owned(), session.to_owned(), pane.to_owned())
            .unwrap_or_else(|error| panic!("{}: take the lead: {error:?}", self.label));
        self.wait("the lead taken", |seen| {
            let fresh = &seen.roles[roles..];
            fresh
                .iter()
                .position(Option::is_none)
                .is_some_and(|attached| fresh[attached..].contains(&Some(PaneRole::Leader)))
        });
    }

    /// Takes in the events that arrive within `wait`.
    pub(crate) fn settle(&mut self, wait: Duration) {
        let deadline = Instant::now() + wait;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            match self.events.recv_timeout(left) {
                Ok(event) => self.seen.record(event),
                Err(_) => break,
            }
        }
    }

    pub(crate) fn wait_layout(&mut self) -> RemoteLayout {
        self.wait("layout", |seen| seen.layout.is_some());
        self.seen.layout.clone().expect("a layout")
    }

    /// Waits for the shown pane's screen to hold `needle`.
    pub(crate) fn wait_screen(&mut self, needle: &str) {
        if let Err(screen) = self.screen_shows(needle, WAIT) {
            panic!("{}: the screen never showed {needle:?}:\n{screen}", self.label);
        }
    }

    /// Whether the shown pane's screen holds `needle` within `wait`; the last screen read when it doesn't.
    pub(crate) fn screen_shows(&mut self, needle: &str, wait: Duration) -> Result<(), String> {
        let deadline = Instant::now() + wait;
        let mut screen = String::new();
        while Instant::now() < deadline {
            // events keep coming while the screen fills
            self.drain();
            if let Some(buffer) = self.terminal.as_ref().and_then(|pane| pane.frame(0).ok()) {
                screen = screen_text(&buffer);
                if screen.contains(needle) {
                    return Ok(());
                }
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err(screen)
    }

    /// Types [`TYPED`] and Enter until the screen shows [`PRINTED`]: a shell that is still starting (fish answers its
    /// terminal queries first) may drop what arrives before its first prompt.
    pub(crate) fn run_line(&mut self) {
        let mut screen = String::new();
        for _ in 0..4 {
            let pane = self.terminal.clone().expect("a shown pane");
            assert!(pane.type_text(TYPED.to_owned()).expect("type"));
            assert!(pane.key(enter()).expect("press enter"));
            match self.screen_shows(PRINTED, WAIT / 4) {
                Ok(()) => return,
                Err(last) => screen = last,
            }
        }
        panic!(
            "{}: the screen never showed {PRINTED:?}:\n{screen}\nseen {:?}",
            self.label, self.seen
        );
    }
}

pub(crate) struct Sink(Mutex<Sender<MobileEvent>>);

impl EventSink for Sink {
    fn on_event(&self, event: MobileEvent) {
        let _ = self.0.lock().unwrap_or_else(PoisonError::into_inner).send(event);
    }
}

pub(crate) struct Vault(Vec<u8>);

impl KeyVault for Vault {
    fn load(&self, key_id: String) -> Result<Vec<u8>, VaultError> {
        if key_id == KEY_ID {
            Ok(self.0.clone())
        } else {
            Err(VaultError::Missing { key_id })
        }
    }
}

/// The `sshd`'s host key, pinned from the start.
pub(crate) struct Pinned(String);

impl HostKeyStore for Pinned {
    fn pinned(&self, _machine_id: String) -> Option<String> {
        Some(self.0.clone())
    }

    fn pin(&self, _machine_id: String, _fingerprint: String) {}
}

pub(crate) fn block_on<T>(future: impl Future<Output = T>) -> T {
    futures_lite::future::block_on(future)
}

/// Retries `attempt` until it answers, failing after [`WAIT`].
pub(crate) fn poll<T>(what: &str, mut attempt: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + WAIT;
    loop {
        if let Some(value) = attempt() {
            return value;
        }
        assert!(Instant::now() < deadline, "no {what} within {WAIT:?}");
        thread::sleep(Duration::from_millis(100));
    }
}

pub(crate) fn nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_nanos())
}

pub(crate) fn enter() -> PaneKey {
    PaneKey {
        action: PaneKeyAction::Press,
        key: "enter".to_owned(),
        mods: 0,
        consumed: 0,
        composing: false,
        text: String::new(),
        unshifted: None,
    }
}

/// The rows of a packed row buffer (`ngterm_mobile::rows`' layout) as lines of text, each run's text at its column.
pub(crate) fn screen_text(buffer: &[u8]) -> String {
    let mut reader = Reader(buffer);
    reader.skip(38);
    let rows = reader.u16();
    let mut lines = Vec::new();
    for _ in 0..rows {
        let (_index, _flags) = (reader.u16(), reader.take(1));
        let mut line = String::new();
        for _ in 0..reader.u16() {
            let column = usize::from(reader.u16());
            reader.skip(18);
            let length = usize::try_from(reader.u32()).expect("a run length");
            let run = String::from_utf8_lossy(reader.take(length)).into_owned();
            while line.chars().count() < column {
                line.push(' ');
            }
            line.push_str(&run);
        }
        lines.push(line);
    }
    lines.join("\n")
}

pub(crate) struct Reader<'a>(&'a [u8]);

impl Reader<'_> {
    pub(crate) fn take(&mut self, count: usize) -> &[u8] {
        let (head, rest) = self.0.split_at(count);
        self.0 = rest;
        head
    }

    pub(crate) fn skip(&mut self, count: usize) {
        self.take(count);
    }

    pub(crate) fn u16(&mut self) -> u16 {
        u16::from_le_bytes(self.take(2).try_into().expect("two bytes"))
    }

    pub(crate) fn u32(&mut self) -> u32 {
        u32::from_le_bytes(self.take(4).try_into().expect("four bytes"))
    }
}
