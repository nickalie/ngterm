//! A phone of the e2e test: a [`MobileCore`] over russh with the test's key and the `sshd`'s host key pinned, what
//! its events said, and reading a shown pane's screen from the packed row buffer.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ngterm_mobile::{
    EventSink, HostKeyStore, KeyVault, Machine, MachineState, MobileCore, MobileEvent, Pane, PaneEnd, PaneKey,
    PaneKeyAction, PaneRole, PaneSize, RemoteLayout, VaultError,
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
#[derive(Debug, Default)]
pub(crate) struct Seen {
    pub(crate) layout: Option<RemoteLayout>,
    pub(crate) role: Option<PaneRole>,
    /// Every role a `PaneLead` gave, in order.
    pub(crate) roles: Vec<Option<PaneRole>>,
    pub(crate) attached: usize,
    pub(crate) lost: usize,
    pub(crate) states: Vec<MachineState>,
}

impl Seen {
    pub(crate) fn record(&mut self, event: MobileEvent) {
        match event {
            MobileEvent::SessionLayout { layout, .. } => self.layout = Some(layout),
            MobileEvent::PaneLead { role, .. } => {
                self.role = role;
                self.roles.push(role);
            }
            MobileEvent::PaneAttached { .. } => self.attached += 1,
            MobileEvent::PaneEnded {
                end: PaneEnd::Lost { .. },
                ..
            } => self.lost += 1,
            MobileEvent::MachineState { state, .. } => self.states.push(state),
            _ => {}
        }
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
        let (sender, events) = mpsc::channel();
        let core = MobileCore::new(
            Arc::new(Sink(Mutex::new(sender))),
            Arc::new(Vault(world.key.private_key.clone())),
            Arc::new(Pinned(world.host_fingerprint.clone())),
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
            pinned_host_key: None,
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
            while let Ok(event) = self.events.try_recv() {
                self.seen.record(event);
            }
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
