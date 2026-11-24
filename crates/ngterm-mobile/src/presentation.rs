//! Attached sessions' presentation streams: one core [`PresentationClient`] per attached session, run on the actor
//! over the machine's connection, with what it reports turned into [`MobileEvent`]s and the answers the origin expects.
//!
//! - **The adapter.** `ChannelLinks` is the client's [`PresentationTransport`]: each launch runs the session's
//!   `ngtermctl live present` tail on an exec channel with no pty, as a local task that splits the output into lines
//!   and reports each one, and the channel's end, tagged with the launch's [`LinkKey`]; a report of an earlier launch
//!   is dropped by the client. Lines sent before the channel opened (the hello) wait for it. A line over
//!   [`MAX_FRAME_BYTES`] ends the stream, as desktop's bridge reader does, and a partial line at the end is dropped.
//!   While the machine's connection is down the adapter has none, and a launch's link closes at once
//!   ([`CONNECTION_DOWN`]), so each client keeps failing on its own backoff until the machine is back
//!   (`Presentations::machine_up`).
//! - **The owner.** `Presentations` holds the clients, passes them the actor's clock, runs their [`TICK`] (reconnect
//!   on backoff, a stream quiet for [`STALE_AFTER`] dropped and reopened) and applies their effects with the semantics
//!   desktop's viewer applies them (`viewer.rs::apply_client_effect`), re-derived for a phone:
//!   - a snapshot's status is no change on the origin ([`MobileEvent::SessionStatus`]'s `changed` is false), so the
//!     app notifies nothing for it;
//!   - the hello lists `viewer_kinds(false)`: no page kinds, as the app shows no handed-over pages; `status.shapes` and
//!     `status.checkmark`, as both apps draw every shape they name (their `StatusShape`); the opt-in
//!     `agent` kind, whose frames become [`MobileEvent::PaneAgent`] for the chat view ([`crate::chat`]). They are not
//!     cleared when the stream leaves `connected`: the origin announces every pane again when it reconnects, and a
//!     chat follows its own channel meanwhile; and the opt-in `overlay.none`, so an origin hands program overlays to a
//!     desktop beside the phone rather than to it;
//!   - an ask for an attached session is always accepted (the app shows it as a dialog when the session is on screen,
//!     otherwise as a notification) and its answer echoes the request's `owner`, kept here;
//!   - the hello lists the pick kinds, so a pick bound to an attached session (`pick --session`) is handed to the phone
//!     that presents it, accepted as an ask is (the app shows it as a picker while the session is on screen) and
//!     answered with its `owner`; and the held kinds, so a headless origin hands the phone what it held for the next
//!     presenter (its ask and pick) as one pending batch, which the client applies together or not at all. An ask or a
//!     pick the origin ends (answered elsewhere, or its `--timeout` ran out) is dismissed;
//!   - an overlay is refused with `overlay.rejected`, and a page with `page.rejected` ([`PAGES_UNSUPPORTED`]);
//!   - when the stream leaves `connected`, what it showed goes with it: context, status and HUD clear, and the asks
//!     and picks still open are dismissed (the origin takes them back), as desktop's row does;
//!   - each key the user typed into a shown pane goes to an origin that speaks `pane.input`, which clears the session's
//!     unseen badge by its own Badge reset rule (`Presentations::typed`), as for each key typed there: the origin can
//!     raise the badge with no frame to the viewer (a terminal's own notification), so no key is held back.
//!
//! A lost connection keeps the streams (`Presentations::machine_down`); a suspending core stops them without a word,
//! first letting their links write what they still hold and the transport send it, an answer just sent among it
//! (bounded by [`FLUSH_LIMIT`]; `Presentations::stop_machine`,
//! then `Presentations::suspend_machine` once the connection is closed); the machine back, each client starts again
//! if stopped and retries at once.
//!
//! The origin's respawn counts are handed on to the session's panes (`Presentations::take_respawns`, then
//! [`crate::pane`]), and its warnings name nothing the app shows: the stream's own state carries the reason it failed.
//!
//! [`STALE_AFTER`]: ngterm_core::remote::client::STALE_AFTER
//! [`PAGES_UNSUPPORTED`]: ngterm_core::remote::client::PAGES_UNSUPPORTED

mod picker;
mod records;
#[cfg(test)]
mod tests;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use ngterm_core::model::{RemoteConnection, StatusKeystroke};
use ngterm_core::pane_env::uuid_string;
use ngterm_core::remote::client::{ClientEffect, PAGES_UNSUPPORTED, PresentationClient};
use ngterm_core::remote::frames::{
    Body, MAX_FRAME_BYTES, PresentationAgent, PresentationAskAnswer, PresentationOverlayChange, PresentationPageChange,
    PresentationPane, PresentationPickAnswer, PresentationRespawns,
};
use ngterm_core::remote::origin::OriginForm;
use ngterm_core::remote::peer::RemotePeer;
use ngterm_core::remote::ssh::present_tail;
use ngterm_core::remote::tree::INVALID_REMOTE_SESSION;
use ngterm_core::remote::{LinkKey, PresentationLink, PresentationTransport};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use uuid::Uuid;

pub use self::picker::{PickRow, pick_rows};
pub use self::records::{
    AskReply, PickReply, PresentationRole, RemoteAsk, RemoteAskButton, RemoteAskChoice, RemoteAskChoices,
    RemoteAskInput, RemoteHud, RemoteLayout, RemoteNotice, RemotePick, RemotePickItem, RemoteStatus, StreamState,
};
use crate::api::{MobileError, MobileEvent};
use crate::pane::PaneRef;
use crate::transport::{ChannelEnd, ChannelOutput, Connection};

/// How often the clients are ticked (desktop's one-second remote tick).
pub const TICK: Duration = Duration::from_secs(1);

/// Why a stream launched while its machine's connection is down ended at once.
pub const CONNECTION_DOWN: &str = "the connection to the machine is down";

/// Why a stream ended that sent a line over [`MAX_FRAME_BYTES`].
pub const OVERSIZE_LINE: &str = "the presentation stream sent an oversize line";

/// How long a stopped link waits for the transport to send the lines it queued before it closes the channel: a close
/// cancels a write still waiting for ssh's window, and a machine that takes nothing must not hold the close for ever.
pub const FLUSH_LIMIT: Duration = Duration::from_millis(500);

/// What a link reports to its client's owner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LinkEvent {
    /// One line, its newline stripped.
    Line(Vec<u8>),
    /// The stream ended, for this reason.
    Closed(String),
}

/// Where the links report: the owner, which hands each report to the client its key names.
pub(crate) type Reporter = Rc<dyn Fn(LinkKey, LinkEvent)>;

/// A client's [`PresentationTransport`]: its streams run on exec channels of its machine's connection, while there
/// is one.
pub(crate) struct ChannelLinks<C> {
    /// `None` while the machine's connection is down.
    connection: RefCell<Option<Rc<C>>>,
    /// The app that listed its session, whose CLI opens the stream whatever the machine's own app is; `None` only
    /// where no listing named it (tests).
    app: Option<RemotePeer>,
    report: Reporter,
    /// The launches' tasks that may still be running, so a suspend can wait for them to write what they hold.
    tasks: RefCell<Vec<JoinHandle<()>>>,
}

impl<C> ChannelLinks<C> {
    /// Streams on `connection`'s channels, reporting to `report`.
    pub(crate) fn new(connection: Rc<C>, report: Reporter) -> Self {
        Self {
            connection: RefCell::new(Some(connection)),
            app: None,
            report,
            tasks: RefCell::new(Vec::new()),
        }
    }

    /// These links for a session of `app`, whose stream that app's CLI opens whatever the machine's own app is.
    pub(crate) fn of_app(self, app: RemotePeer) -> Self {
        Self { app: Some(app), ..self }
    }

    /// The machine's connection from now on: `None` while it is down.
    fn set_connection(&self, connection: Option<Rc<C>>) {
        *self.connection.borrow_mut() = connection;
    }

    /// The launches' tasks not yet finished, handed over to be waited on.
    fn take_tasks(&self) -> Vec<JoinHandle<()>> {
        std::mem::take(&mut *self.tasks.borrow_mut())
    }
}

impl<C: Connection> ChannelLinks<C> {
    /// How its session's remote commands are formed: its machine's, for its own app when it names one; a machine down
    /// is taken for a POSIX one.
    fn origin_form(&self) -> OriginForm {
        let form = self
            .connection
            .borrow()
            .as_ref()
            .map_or_else(OriginForm::default, |connection| connection.origin_form());
        OriginForm {
            peer: self.app.unwrap_or(form.peer),
            ..form
        }
    }
}

impl<C: Connection> PresentationTransport for ChannelLinks<C> {
    fn open(&self, argv: &[String], key: LinkKey) -> Box<dyn PresentationLink> {
        let (input, queue) = mpsc::unbounded_channel();
        let report = Rc::clone(&self.report);
        let Some(connection) = self.connection.borrow().clone() else {
            // Reported from a task: the owner is busy with the client that opened it.
            tokio::task::spawn_local(async move { report(key, LinkEvent::Closed(CONNECTION_DOWN.to_owned())) });
            return Box::new(ChannelLink { input });
        };
        // The argv is the remote command alone: the connection is the ssh part.
        let command = argv.last().cloned().unwrap_or_default();
        let task = tokio::task::spawn_local(run_link(connection, command, key, queue, report));
        let mut tasks = self.tasks.borrow_mut();
        tasks.retain(|task| !task.is_finished());
        tasks.push(task);
        Box::new(ChannelLink { input })
    }
}

/// What a link's task is asked to do.
enum LinkInput {
    /// Send one line, its newline included.
    Line(Vec<u8>),
    /// Close the channel and report nothing more.
    Stop,
}

/// One launch's stream, as the client holds it; dropping it stops the stream as [`PresentationLink::stop`] does.
struct ChannelLink {
    input: mpsc::UnboundedSender<LinkInput>,
}

impl PresentationLink for ChannelLink {
    fn send(&self, line: &[u8]) {
        // A task that has ended has no channel to write to.
        let _ = self.input.send(LinkInput::Line(line.to_vec()));
    }

    fn stop(&self) {
        let _ = self.input.send(LinkInput::Stop);
    }
}

/// One launch: opens the channel, sends what was queued meanwhile, then reports each line and the end, and sends
/// what the client sends, until either side ends it.
async fn run_link<C: Connection>(
    connection: Rc<C>,
    command: String,
    key: LinkKey,
    mut inputs: mpsc::UnboundedReceiver<LinkInput>,
    report: Reporter,
) {
    let mut pending = Vec::new();
    let opened = {
        let exec = connection.exec(&command, None);
        tokio::pin!(exec);
        loop {
            tokio::select! {
                opened = &mut exec => break opened,
                input = inputs.recv() => match input {
                    Some(LinkInput::Line(line)) => pending.push(line),
                    Some(LinkInput::Stop) | None => return,
                },
            }
        }
    };
    let mut channel = match opened {
        Ok(channel) => channel,
        Err(error) => return report(key, LinkEvent::Closed(error.to_string())),
    };
    for line in pending {
        // A channel already gone reports its end below.
        let _ = channel.writer.write(&line);
    }
    let mut lines = LineSplitter::default();
    loop {
        tokio::select! {
            // The client's word first: a stop ends the stream even when its end is already queued.
            biased;
            input = inputs.recv() => match input {
                Some(LinkInput::Line(line)) => {
                    let _ = channel.writer.write(&line);
                }
                Some(LinkInput::Stop) | None => {
                    // What was written (an answer just sent) goes out first: the close would cancel it.
                    let _ = tokio::time::timeout(FLUSH_LIMIT, channel.writer.flushed()).await;
                    let _ = channel.writer.close();
                    return;
                }
            },
            output = channel.reader.read() => {
                if let Some(reason) = lines.take(output, |line| report(key, LinkEvent::Line(line))) {
                    if reason == OVERSIZE_LINE {
                        let _ = channel.writer.close();
                    }
                    return report(key, LinkEvent::Closed(reason));
                }
            }
        }
    }
}

/// Splits a stream's standard output into lines.
#[derive(Default)]
struct LineSplitter {
    partial: Vec<u8>,
}

impl LineSplitter {
    /// Hands each whole line of `output` to `line`; the stream's end reason when it ended (an oversize line ends it).
    fn take(&mut self, output: ChannelOutput, mut line: impl FnMut(Vec<u8>)) -> Option<String> {
        let bytes = match output {
            ChannelOutput::Data(bytes) => bytes,
            // Desktop logs the bridge's standard error; nothing here shows it.
            ChannelOutput::Stderr(_) => return None,
            ChannelOutput::End(end) => return Some(end_reason(end)),
        };
        self.partial.extend_from_slice(&bytes);
        while let Some(at) = self.partial.iter().position(|&byte| byte == b'\n') {
            let mut whole: Vec<u8> = self.partial.drain(..=at).collect();
            whole.pop();
            if whole.len() > MAX_FRAME_BYTES {
                return Some(OVERSIZE_LINE.to_owned());
            }
            line(whole);
        }
        (self.partial.len() > MAX_FRAME_BYTES).then(|| OVERSIZE_LINE.to_owned())
    }
}

/// How a stream ended, in desktop's bridge's words (`exit N`, `signal NAME`), or why its link went.
fn end_reason(end: ChannelEnd) -> String {
    match end {
        ChannelEnd::Exit { code: Some(code), .. } => format!("exit {code}"),
        ChannelEnd::Exit {
            code: None,
            signal: Some(signal),
        } => format!("signal {signal}"),
        ChannelEnd::Exit {
            code: None,
            signal: None,
        } => "ended".to_owned(),
        ChannelEnd::Lost(reason) => reason,
    }
}

/// One attached session's stream and what it keeps beside it.
struct Attached<C> {
    machine_id: String,
    session_id: String,
    client: PresentationClient,
    links: ChannelLinks<C>,
    /// Whether the stream is up: leaving it clears what it showed.
    connected: bool,
    /// The asks shown and not yet answered or dismissed, with the `owner` their answer echoes.
    asks: HashMap<String, i64>,
    /// The picks shown and not yet answered or dismissed, with the `owner` their answer echoes.
    picks: HashMap<String, i64>,
    /// The respawn counts reported and not yet handed to the panes, each with whether a snapshot carried it.
    respawns: Vec<(PresentationRespawns, bool)>,
}

/// Respawn counts an attached session's origin reported, for its panes ([`crate::pane`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Respawns {
    pub(crate) machine_id: String,
    pub(crate) session_id: String,
    pub(crate) counts: PresentationRespawns,
    /// Whether a snapshot carried them (a reconnect's baseline) rather than a respawn frame.
    pub(crate) seed: bool,
}

/// Every attached session's stream.
pub(crate) struct Presentations<C> {
    attached: Vec<Attached<C>>,
    /// The id of the last client made, so a session attached again never reuses an earlier client's link keys.
    last_client: u64,
}

impl<C> Default for Presentations<C> {
    fn default() -> Self {
        Self {
            attached: Vec::new(),
            last_client: 0,
        }
    }
}

impl<C: Connection> Presentations<C> {
    /// Opens `session_id`'s stream on `links`, to an origin speaking presentation `version` (`None`: it predates
    /// the stream, which is reported unsupported and never opened). Attaching an attached session changes nothing.
    ///
    /// # Errors
    ///
    /// [`MobileError::Remote`] with [`INVALID_REMOTE_SESSION`] for an id that is not a session's.
    pub(crate) fn attach(
        &mut self,
        machine_id: &str,
        session_id: &str,
        version: Option<i64>,
        links: ChannelLinks<C>,
        now: Instant,
    ) -> Result<Vec<MobileEvent>, MobileError> {
        if self.find(machine_id, session_id).is_some() {
            return Ok(Vec::new());
        }
        let invalid = || MobileError::Remote {
            reason: INVALID_REMOTE_SESSION.to_owned(),
        };
        let session = Uuid::parse_str(session_id).map_err(|_| invalid())?;
        let tail = present_tail(links.origin_form(), session_id).map_err(|_| invalid())?;
        self.last_client += 1;
        let mut client = PresentationClient::new(session, self.last_client, vec![tail], version);
        client.set_shows_agents(true);
        client.set_refuses_overlays(true);
        client.set_shows_picks(true);
        client.set_takes_held(true);
        let mut attached = Attached {
            machine_id: machine_id.to_owned(),
            session_id: session_id.to_owned(),
            client,
            links,
            connected: false,
            asks: HashMap::new(),
            picks: HashMap::new(),
            respawns: Vec::new(),
        };
        let effects = attached.client.start(&attached.links, now);
        let events = attached.apply_all(effects);
        self.attached.push(attached);
        Ok(events)
    }

    /// Ends `session_id`'s stream, reporting nothing: the app is leaving the session.
    pub(crate) fn detach(&mut self, machine_id: &str, session_id: &str) {
        if let Some(index) = self.find(machine_id, session_id) {
            self.attached.remove(index).client.stop();
        }
    }

    /// Ends the streams of every session of `machine_id`, reporting nothing.
    pub(crate) fn forget_machine(&mut self, machine_id: &str) {
        self.attached.retain_mut(|attached| {
            let keep = attached.machine_id != machine_id;
            if !keep {
                attached.client.stop();
            }
            keep
        });
    }

    /// `machine_id`'s connection was lost: its streams stay, and each one launched until [`Presentations::machine_up`]
    /// closes at once, so its client fails on its own backoff.
    pub(crate) fn machine_down(&mut self, machine_id: &str) {
        for attached in self.of_machine(machine_id) {
            attached.links.set_connection(None);
        }
    }

    /// Stops the streams of `machine_id`, whose connection the core is about to close to suspend, reporting nothing,
    /// and hands over their links' tasks: each writes the lines it still holds (an answer just sent), waits up to
    /// [`FLUSH_LIMIT`] for the transport to send them, closes its channel and ends, so the core waits for them before
    /// it closes the connection. They start again once the
    /// machine is back ([`Presentations::machine_up`]).
    pub(crate) fn stop_machine(&mut self, machine_id: &str) -> Vec<JoinHandle<()>> {
        let mut tasks = Vec::new();
        for attached in self.of_machine(machine_id) {
            attached.client.stop();
            tasks.extend(attached.links.take_tasks());
        }
        tasks
    }

    /// Stops the streams of `machine_id`, whose connection the core closed to suspend, reporting nothing: they start
    /// again once the machine is back ([`Presentations::machine_up`]).
    pub(crate) fn suspend_machine(&mut self, machine_id: &str) {
        for attached in self.of_machine(machine_id) {
            attached.links.set_connection(None);
            attached.client.stop();
        }
    }

    /// `machine_id` is connected again, on `connection`: each of its streams starts again if stopped, and retries at
    /// once with its backoff over.
    pub(crate) fn machine_up(&mut self, machine_id: &str, connection: &Rc<C>, now: Instant) -> Vec<MobileEvent> {
        let mut events = Vec::new();
        for attached in self.of_machine(machine_id) {
            attached.links.set_connection(Some(Rc::clone(connection)));
            let mut effects = attached.client.start(&attached.links, now);
            attached.client.retry_now(now);
            effects.extend(attached.client.tick(&attached.links, now));
            events.extend(attached.apply_all(effects));
        }
        events
    }

    fn of_machine<'a>(&'a mut self, machine_id: &'a str) -> impl Iterator<Item = &'a mut Attached<C>> {
        self.attached
            .iter_mut()
            .filter(move |attached| attached.machine_id == machine_id)
    }

    /// What a link reported, handed to the client its key names; a report of a client since gone is dropped.
    pub(crate) fn link_event(&mut self, key: LinkKey, event: LinkEvent, now: Instant) -> Vec<MobileEvent> {
        let Some(attached) = self
            .attached
            .iter_mut()
            .find(|attached| attached.client.id() == key.client)
        else {
            return Vec::new();
        };
        let effects = match event {
            LinkEvent::Line(line) => attached.client.receive(key.launch, &line, now),
            LinkEvent::Closed(reason) => attached.client.link_closed(key.launch, &reason, now),
        };
        attached.apply_all(effects)
    }

    /// Runs every client's tick: a due retry reopens its stream, a stream quiet too long is dropped.
    pub(crate) fn tick(&mut self, now: Instant) -> Vec<MobileEvent> {
        let mut events = Vec::new();
        for attached in &mut self.attached {
            let effects = attached.client.tick(&attached.links, now);
            events.extend(attached.apply_all(effects));
        }
        events
    }

    /// Sends the user's answer to the ask `ask_id` of `session_id`, echoing the request's owner; false when no such
    /// ask is open (answered, dismissed, or its stream went).
    pub(crate) fn answer_ask(&mut self, machine_id: &str, session_id: &str, ask_id: &str, reply: AskReply) -> bool {
        let Some(index) = self.find(machine_id, session_id) else {
            return false;
        };
        let attached = &mut self.attached[index];
        let Some(owner) = attached.asks.remove(ask_id) else {
            return false;
        };
        attached.client.answer(Body::AskResolve(PresentationAskAnswer {
            id: ask_id.to_owned(),
            owner,
            button: reply.button,
            text: reply.text,
            selected: reply.selected,
        }));
        true
    }

    /// Sends the user's answer to the pick `pick_id` of `session_id`, echoing the request's owner; false when no such
    /// pick is open (answered, dismissed, or its stream went).
    pub(crate) fn answer_pick(&mut self, machine_id: &str, session_id: &str, pick_id: &str, reply: PickReply) -> bool {
        let Some(index) = self.find(machine_id, session_id) else {
            return false;
        };
        let attached = &mut self.attached[index];
        let Some(owner) = attached.picks.remove(pick_id) else {
            return false;
        };
        let (item, query) = match reply {
            PickReply::Picked { item } => (Some(item), None),
            PickReply::Custom { query } => (None, Some(query)),
            PickReply::Cancelled => (None, None),
        };
        attached.client.answer(Body::PickResolve(PresentationPickAnswer {
            id: pick_id.to_owned(),
            owner,
            item,
            query,
        }));
        true
    }

    /// Tells the origin of `pane`'s session of a key the user typed there, for its Badge reset rule; whether it went.
    pub(crate) fn typed(&mut self, pane: &PaneRef, key: StatusKeystroke) -> bool {
        let Some(index) = self.find(&pane.machine_id, &pane.session_id) else {
            return false;
        };
        let Ok(identity) = Uuid::parse_str(&pane.pane_id) else {
            return false;
        };
        self.attached[index].client.typed(identity, key)
    }

    /// The respawn counts reported since the last call, oldest first.
    pub(crate) fn take_respawns(&mut self) -> Vec<Respawns> {
        let mut taken = Vec::new();
        for attached in &mut self.attached {
            taken.extend(attached.respawns.drain(..).map(|(counts, seed)| Respawns {
                machine_id: attached.machine_id.clone(),
                session_id: attached.session_id.clone(),
                counts,
                seed,
            }));
        }
        taken
    }

    fn find(&self, machine_id: &str, session_id: &str) -> Option<usize> {
        self.attached
            .iter()
            .position(|attached| attached.machine_id == machine_id && attached.session_id == session_id)
    }
}

impl<C> Attached<C> {
    fn apply_all(&mut self, effects: Vec<ClientEffect>) -> Vec<MobileEvent> {
        let mut events = Vec::new();
        for effect in effects {
            self.apply(effect, &mut events);
        }
        events
    }

    /// One effect: the event it makes, and the origin's answer when the app can't show what it handed over.
    fn apply(&mut self, effect: ClientEffect, events: &mut Vec<MobileEvent>) {
        let (machine_id, session_id) = (self.machine_id.clone(), self.session_id.clone());
        let event = match effect {
            ClientEffect::Connection(connection) => return self.set_connection(connection, events),
            ClientEffect::Status(status) => MobileEvent::SessionStatus {
                machine_id,
                session_id,
                status: status.map(RemoteStatus::from),
                changed: true,
            },
            ClientEffect::SnapshotStatus(status) => MobileEvent::SessionStatus {
                machine_id,
                session_id,
                status: status.map(RemoteStatus::from),
                changed: false,
            },
            ClientEffect::Context(context) => MobileEvent::SessionContext {
                machine_id,
                session_id,
                context,
            },
            ClientEffect::Name(name) => MobileEvent::SessionName {
                machine_id,
                session_id,
                name,
            },
            ClientEffect::Layout(layout) => MobileEvent::SessionLayout {
                machine_id,
                session_id,
                layout: layout.into(),
            },
            ClientEffect::Mode(mode) => MobileEvent::SessionMode {
                machine_id,
                session_id,
                role: mode.into(),
            },
            ClientEffect::Hud(hud) => MobileEvent::Hud {
                machine_id,
                session_id,
                hud: hud.map(RemoteHud::from),
            },
            ClientEffect::Notify(notify) => MobileEvent::Notify {
                machine_id,
                session_id,
                notice: notify.into(),
            },
            ClientEffect::Sound(sound) => MobileEvent::Sound {
                machine_id,
                session_id,
                name: sound.name,
            },
            effect @ (ClientEffect::AskRequest(_)
            | ClientEffect::AskDismiss(_)
            | ClientEffect::PickRequest(_)
            | ClientEffect::PickDismiss(_)) => return events.extend(self.interaction(effect)),
            ClientEffect::Respawns(respawns, seed) => return self.respawns.push((respawns, seed)),
            ClientEffect::Agent(agent) => {
                let Some(event) = self.agent(agent) else {
                    return;
                };
                event
            }
            other => return self.refuse(other),
        };
        events.push(event);
    }

    /// An ask or a pick the origin handed over or ended, as its event; the end of one not shown makes none.
    fn interaction(&mut self, effect: ClientEffect) -> Option<MobileEvent> {
        let (machine_id, session_id) = (self.machine_id.clone(), self.session_id.clone());
        Some(match effect {
            ClientEffect::AskRequest(ask) => {
                self.asks.insert(ask.id.clone(), ask.owner);
                MobileEvent::Ask {
                    machine_id,
                    session_id,
                    ask: ask.into(),
                }
            }
            ClientEffect::AskDismiss(reference) => {
                self.asks.remove(&reference.id)?;
                MobileEvent::AskDismiss {
                    machine_id,
                    session_id,
                    ask_id: reference.id,
                }
            }
            ClientEffect::PickRequest(pick) => {
                self.picks.insert(pick.id.clone(), pick.owner);
                MobileEvent::Pick {
                    machine_id,
                    session_id,
                    pick: pick.into(),
                }
            }
            ClientEffect::PickDismiss(reference) => {
                // one under an earlier presenter generation names another hand-over
                if self.picks.get(&reference.id) != Some(&reference.owner) {
                    return None;
                }
                self.picks.remove(&reference.id);
                MobileEvent::PickDismiss {
                    machine_id,
                    session_id,
                    pick_id: reference.id,
                }
            }
            _ => return None,
        })
    }

    /// What a pane runs, as its event; the scratch pane keeps no agent record.
    fn agent(&self, agent: PresentationAgent) -> Option<MobileEvent> {
        let PresentationPane::Identity(pane) = agent.pane else {
            return None;
        };
        Some(MobileEvent::PaneAgent {
            machine_id: self.machine_id.clone(),
            session_id: self.session_id.clone(),
            pane_id: uuid_string(pane),
            agent: agent.agent,
            conversation_id: agent.conversation_id,
        })
    }

    /// What the app never shows: an overlay or a page is refused, so the origin takes it back at once; their closes
    /// and commands and the warnings change nothing here.
    fn refuse(&self, effect: ClientEffect) {
        match effect {
            ClientEffect::OverlayRequest(overlay) => self
                .client
                .answer(Body::OverlayRejected(PresentationOverlayChange::job(overlay.job))),
            ClientEffect::PageRequest(page) => self.client.answer(Body::PageRejected(PresentationPageChange {
                page: page.page,
                reason: Some(PAGES_UNSUPPORTED.to_owned()),
            })),
            _ => {}
        }
    }

    /// The stream's new state; leaving `connected` clears what the stream showed and dismisses its open asks and
    /// picks.
    fn set_connection(&mut self, connection: RemoteConnection, events: &mut Vec<MobileEvent>) {
        let connected = connection == RemoteConnection::Connected;
        let left = self.connected && !connected;
        self.connected = connected;
        let (machine_id, session_id) = (self.machine_id.clone(), self.session_id.clone());
        events.push(MobileEvent::SessionConnection {
            machine_id: machine_id.clone(),
            session_id: session_id.clone(),
            state: connection.into(),
        });
        if !left {
            return;
        }
        let mut asks: Vec<String> = self.asks.drain().map(|(id, _)| id).collect();
        asks.sort();
        events.extend(asks.into_iter().map(|ask_id| MobileEvent::AskDismiss {
            machine_id: machine_id.clone(),
            session_id: session_id.clone(),
            ask_id,
        }));
        let mut picks: Vec<String> = self.picks.drain().map(|(id, _)| id).collect();
        picks.sort();
        events.extend(picks.into_iter().map(|pick_id| MobileEvent::PickDismiss {
            machine_id: machine_id.clone(),
            session_id: session_id.clone(),
            pick_id,
        }));
        events.push(MobileEvent::SessionContext {
            machine_id: machine_id.clone(),
            session_id: session_id.clone(),
            context: None,
        });
        events.push(MobileEvent::SessionStatus {
            machine_id: machine_id.clone(),
            session_id: session_id.clone(),
            status: None,
            changed: false,
        });
        events.push(MobileEvent::Hud {
            machine_id,
            session_id,
            hud: None,
        });
    }
}
