//! The main loop's state: the core, the panes' terminals, and the tasks that change them (the GPUI frontend's
//! `state.rs`, less what a window shows).
//!
//! RULE, as there: every mutation runs as a main-loop task and ends with [`HeadlessState::settle`], which commits and
//! arms what the commit leaves to the loop; after each task the runner calls [`HeadlessState::land`], which brings the
//! terminals in line with the model ([`crate::panes`]).
//!
//! A viewer's `live.present`, an overlay job's helper and a page channel keep their connection as a stream adopted by
//! the core, exactly as in the GPUI frontend: the presentation of a session a viewer attached goes out on it, and what
//! the viewer presents (asks, program overlays, pages) runs on the viewer. What would be drawn here (banners, sounds,
//! windows, the welcome) has no display: banners are logged, the rest is dropped.

use std::panic::{self, AssertUnwindSafe};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use ngterm_app::page_channel::{PageHelperOwner, PageHelperProcess};
use ngterm_app::remote::{InboundGate, PresentationViewer, ProcessTransport};
use ngterm_core::auto_follow::AutoFollowTimer;
use ngterm_core::events::{PANE_FOCUS_DEBOUNCE, PaneFocusTimer, TREE_CHANGED_DEBOUNCE, TreeChangedTimer};
use ngterm_core::model::PendingCloseTimer;
use ngterm_core::persist::{DebouncedSave, arm_debounced_saves};
use ngterm_core::remote::page_serve::{self, BridgeReply, PageChannel, PageServeStream, PageServeWork};
use ngterm_core::{CommitTimers, ControlError, Core, Dispatched, NotificationEffect, Resume};
use ngterm_host::MainLoop;
use ngterm_proto::command::{LiveCmd, SessionCmd};
use ngterm_proto::{Command, Request, Response};
use ngterm_server::{LineStream, LineStreamLimits, LineStreamOwner, Reply, ReplySender};
use uuid::Uuid;

use crate::panes::{Panes, Report};

/// How long a quit a command asked for waits, so the command's reply is written first.
const LIVE_QUIT_DELAY: Duration = Duration::from_millis(250);

/// The bounds of an overlay job helper's connection (upstream's `overlayJobLimits`).
const JOB_STREAM_LIMITS: LineStreamLimits = LineStreamLimits {
    max_line_bytes: 256 * 1024,
    max_pending_lines: 16,
    write_timeout: Duration::from_secs(5),
};

/// The bounds of a viewer's presentation stream (upstream's `presentationLimits`).
const PRESENTATION_LIMITS: LineStreamLimits = LineStreamLimits {
    max_line_bytes: ngterm_core::remote::frames::MAX_FRAME_BYTES,
    max_pending_lines: ngterm_core::remote::frames::MAX_PENDING_FRAMES,
    write_timeout: ngterm_core::remote::WRITE_TIMEOUT,
};

/// The bounds of a page channel (upstream's `pageServeLimits`).
const PAGE_SERVE_LIMITS: LineStreamLimits = LineStreamLimits {
    max_line_bytes: page_serve::MAX_FRAME_BYTES,
    max_pending_lines: page_serve::MAX_PENDING_LINES,
    write_timeout: page_serve::WRITE_TIMEOUT,
};

/// Where handlers post tasks: the main loop, held weakly so a handler outliving it posts nothing.
pub type Poster = Weak<dyn MainLoop<HeadlessState>>;

/// Queues `task` on the main loop; a closed or gone loop drops it.
pub fn post(poster: &Poster, task: impl FnOnce(&mut HeadlessState) + Send + 'static) {
    let Some(main_loop) = poster.upgrade() else {
        return;
    };
    if main_loop.post(Box::new(task)).is_err() {
        tracing::debug!("main-loop task dropped: the loop is closed");
    }
}

/// An adopted streaming connection as the core sends to it.
struct StreamLines(LineStream);

impl ngterm_core::LineChannel for StreamLines {
    fn send(&self, line: Vec<u8>) -> bool {
        self.0.send(line)
    }

    fn shutdown(&self) {
        self.0.shutdown();
    }
}

impl PageChannel for StreamLines {
    fn send(&self, line: Vec<u8>) -> bool {
        self.0.send(line)
    }

    fn send_waiting(&self, line: Vec<u8>) -> bool {
        self.0.send_waiting(line)
    }

    fn shutdown(&self) {
        self.0.shutdown();
    }
}

/// Who waits for a deferred answer: a control connection, or a bridge request a presenter's page sent over its page
/// channel.
enum Waiter {
    Socket(ReplySender),
    PageServe(BridgeReply),
}

impl Waiter {
    fn send(self, response: Response) {
        match self {
            Self::Socket(sender) => sender.send(response),
            Self::PageServe(reply) => reply.answer(&response),
        }
    }
}

/// Where the process is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Running,
    /// A quit was asked for: the runner shuts down after the task that asked.
    QuitRequested,
    /// The quit save ran: nothing is committed or saved after it.
    Quitting,
}

/// What the main loop owns.
pub struct HeadlessState {
    pub core: Core,
    poster: Poster,
    panes: Panes,
    phase: Phase,
    heartbeat_armed: bool,
    remote_tick_armed: bool,
}

/// The presentation bridges of the sessions this instance attached from another machine ([`ngterm_app::remote`]).
impl PresentationViewer for HeadlessState {
    fn viewer_line(&mut self, key: ngterm_core::remote::LinkKey, line: &[u8]) {
        if !self.quitting() {
            self.core.viewer_line(key, line);
            self.settle();
        }
    }

    fn viewer_closed(&mut self, key: ngterm_core::remote::LinkKey, reason: &str) {
        if !self.quitting() {
            self.core.viewer_closed(key, reason);
            self.settle();
        }
    }
}

/// The helpers carrying the channels of pages origins handed over ([`ngterm_app::page_channel`]).
impl PageHelperOwner for HeadlessState {
    fn page_helper_line(&mut self, key: ngterm_core::remote::replica_page::HelperKey, line: &[u8]) {
        if !self.quitting() {
            self.core.page_helper_line(key, line);
            self.settle();
        }
    }

    fn page_helper_closed(&mut self, key: ngterm_core::remote::replica_page::HelperKey, reason: &str) {
        if !self.quitting() {
            self.core.page_helper_closed(key, reason);
            self.settle();
        }
    }
}

impl HeadlessState {
    /// The state over `core`, its rows attached from other machines bridged through child processes posting to
    /// `poster`, as the GPUI frontend's `AppState::attach`: an instance here can view another's sessions too.
    #[must_use]
    pub fn new(mut core: Core, poster: Poster, panes: Panes) -> Self {
        core.set_presentation_transport(std::rc::Rc::new(ProcessTransport::new(poster.clone())));
        core.set_page_helper_launcher(std::rc::Rc::new(PageHelperProcess::new(poster.clone())));
        Self {
            core,
            poster,
            panes,
            phase: Phase::Running,
            heartbeat_armed: false,
            remote_tick_armed: false,
        }
    }

    /// Asks for the orderly shutdown after the running task.
    pub fn request_quit(&mut self) {
        if self.phase == Phase::Running {
            self.phase = Phase::QuitRequested;
        }
    }

    #[must_use]
    pub fn quit_requested(&self) -> bool {
        self.phase == Phase::QuitRequested
    }

    /// The quit save ran.
    fn quitting(&self) -> bool {
        self.phase == Phase::Quitting
    }

    /// How many terminals run.
    #[must_use]
    pub fn terminals(&self) -> usize {
        self.panes.len()
    }

    /// The end of every mutation: commit, arm the loop's timers, take what has no display here.
    pub fn settle(&mut self) {
        if self.quitting() {
            return;
        }
        let timers = self.core.commit();
        if let Some(main_loop) = self.poster.upgrade() {
            arm_commit(&*main_loop, &timers);
        }
        if let Some(quit) = self.core.take_live_quit() {
            self.schedule_live_quit(quit.relaunch);
        }
        self.take_alerts();
        self.arm_auto_follow();
        self.arm_hud_timers();
        self.arm_job_expiries();
        self.arm_page_claim_expiries();
        self.arm_remote_tick();
    }

    /// After every task: the terminals follow the model, and what windows would take is dropped.
    pub fn land(&mut self) {
        if self.quitting() {
            return;
        }
        let before = self.panes.len();
        let changed = self.panes.sync(&mut self.core);
        drop(self.core.take_window_requests());
        self.core.clear_pane_focus_requests();
        if changed || before != self.panes.len() {
            self.settle();
        }
    }

    /// Banners are logged (no notification server); sounds have no speaker.
    fn take_alerts(&mut self) {
        drop(self.core.take_status_sounds());
        for effect in self.core.take_notifications() {
            match &effect {
                NotificationEffect::Post(post) => tracing::info!(title = post.title, body = post.body, "notification"),
                NotificationEffect::App { title, body, .. } => tracing::warn!(title, body, "notification"),
                NotificationEffect::Clear { .. } => {}
            }
        }
    }

    /// The events of `session`'s pane `token`, reported to the model.
    pub fn drain_pane(&mut self, session: Uuid, token: &str) {
        if self.quitting() {
            return;
        }
        let mut changed = false;
        for report in self.panes.drain(session, token) {
            changed |= match report {
                Report::Pane(report) => self.core.report_pane(session, token, &report),
                Report::Notification { title, body } => self.core.terminal_notification(session, token, &title, &body),
                Report::Exited => self.core.pane_exited(session, token),
                Report::Held => self.core.remote_pane_held(session, token),
            };
        }
        if changed {
            self.settle();
        }
    }

    /// Answers one control request and settles what it changed (the GPUI frontend's `handle_request`).
    pub fn handle_request(&mut self, request: &Request) -> Reply {
        let dispatched = self.core.begin(request);
        let reply = match dispatched {
            Dispatched::Done(response) if request.cmd == Command::Session(SessionCmd::OverlayJobRun) && response.ok => {
                self.job_stream(response)
            }
            Dispatched::Done(response) if request.cmd == Command::Live(LiveCmd::Present) && response.ok => {
                self.presentation_stream(response)
            }
            Dispatched::Done(response)
                if request.cmd == Command::Session(SessionCmd::OverlayPageServe) && response.ok =>
            {
                self.page_serve_stream(response)
            }
            other => {
                self.settle();
                return self.reply(request, other);
            }
        };
        self.settle();
        reply
    }

    fn reply(&self, request: &Request, dispatched: Dispatched) -> Reply {
        match dispatched {
            Dispatched::Done(response) => response.into(),
            Dispatched::Pending { after, resume } => {
                let (reply, sender) = Reply::deferred();
                self.schedule(request.cmd, after, resume, Waiter::Socket(sender));
                reply
            }
        }
    }

    /// The answer to a won overlay job claim, which keeps the connection (upstream's `adoptOverlayJobStream`).
    fn job_stream(&mut self, response: Response) -> Reply {
        let Some(job) = result_id(&response) else {
            return response.into();
        };
        let stream = LineStream::new(JOB_STREAM_LIMITS);
        let serial = self
            .core
            .adopt_overlay_job_stream(&job, Box::new(StreamLines(stream.clone())));
        let (lines, closes) = (self.poster.clone(), self.poster.clone());
        let (line_job, close_job) = (job.clone(), job);
        let owner = LineStreamOwner::new(
            stream,
            move |line| {
                let job = line_job.clone();
                post(&lines, move |state| {
                    state.core.overlay_job_line(&job, serial, &line);
                    state.settle();
                });
            },
            move || {
                post(&closes, move |state| {
                    state.core.overlay_job_closed(&close_job, serial);
                    state.settle();
                });
            },
        );
        Reply::streaming(response, Box::new(owner))
    }

    /// The answer to an accepted `live.present`, which keeps the connection (upstream's `adoptPresentationStream`):
    /// the viewer's stream, its lines and close reported back as main-loop tasks, closed without a hello in time.
    fn presentation_stream(&mut self, response: Response) -> Reply {
        let Some(session) = result_id(&response) else {
            return response.into();
        };
        let stream = LineStream::new(PRESENTATION_LIMITS);
        let serial = self
            .core
            .adopt_presentation_stream(&session, Box::new(StreamLines(stream.clone())));
        if let Some(main_loop) = self.poster.upgrade() {
            let deadline = Box::new(move |state: &mut Self| {
                if !state.quitting() {
                    state.core.presentation_hello_deadline(serial);
                }
            });
            if main_loop
                .post_after(ngterm_core::remote::HELLO_DEADLINE, deadline)
                .is_err()
            {
                tracing::debug!("presentation hello deadline dropped: the loop is closed");
            }
        }
        self.arm_presentation_heartbeat();
        let owner = LineStreamOwner::new(
            stream,
            presentation_reader(self.poster.clone(), serial),
            presentation_closer(self.poster.clone(), serial),
        );
        Reply::streaming(response, Box::new(owner))
    }

    /// The answer to an admitted page claim, which keeps the connection (upstream's `adoptPageServeStream`).
    fn page_serve_stream(&mut self, response: Response) -> Reply {
        let Some(page) = result_id(&response) else {
            return response.into();
        };
        let stream = LineStream::new(PAGE_SERVE_LIMITS);
        let channel = Arc::new(StreamLines(stream.clone()));
        let Some(serve) = self.core.adopt_page_serve_stream(&page, channel) else {
            return Reply::streaming(response, Box::new(LineStreamOwner::new(stream, |_| {}, || {})));
        };
        let (lines, closes) = (self.poster.clone(), self.poster.clone());
        let reader = Arc::clone(&serve);
        let owner = LineStreamOwner::new(
            stream,
            move |line| {
                if let Some(work) = reader.take(&line) {
                    post(&lines, move |state| state.run_page_serve_work(work));
                }
            },
            move || {
                post(&closes, move |state| page_serve_closed(state, &serve));
            },
        );
        Reply::streaming(response, Box::new(owner))
    }

    /// Runs what a page channel left for the main loop: a bridge call, dispatched and answered on the channel.
    fn run_page_serve_work(&mut self, work: PageServeWork) {
        if self.quitting() {
            return;
        }
        let Some(call) = self.core.page_serve_work(work) else {
            self.settle();
            return;
        };
        let cmd = call.request.cmd;
        let dispatched = panic::catch_unwind(AssertUnwindSafe(|| self.core.begin(&call.request)));
        self.settle();
        match dispatched {
            Ok(Dispatched::Done(response)) => call.reply.answer(&response),
            Ok(Dispatched::Pending { after, resume }) => {
                self.schedule(cmd, after, resume, Waiter::PageServe(call.reply));
            }
            Err(_) => {
                tracing::error!(%cmd, "control handler panicked on a page channel's request");
                call.reply.answer(&ControlError::HandlerPanicked(cmd).into());
            }
        }
    }

    /// Posts `task` after `delay`, logging a closed loop under `what`.
    fn after(&self, delay: Duration, what: &str, task: impl FnOnce(&mut Self) + Send + 'static) -> bool {
        let Some(main_loop) = self.poster.upgrade() else {
            return false;
        };
        let posted = main_loop.post_after(delay, Box::new(task)).is_ok();
        if !posted {
            tracing::debug!(what, "delayed task dropped: the loop is closed");
        }
        posted
    }

    fn arm_page_claim_expiries(&mut self) {
        for delay in self.core.take_page_claim_expiries() {
            self.after(delay, "page claim expiry", |state| {
                if !state.quitting() {
                    state.core.expire_remote_page_claims();
                    state.settle();
                }
            });
        }
    }

    fn arm_job_expiries(&mut self) {
        for delay in self.core.take_overlay_job_expiries() {
            self.after(delay, "overlay job expiry", |state| {
                if !state.quitting() {
                    state.core.expire_overlay_jobs();
                    state.settle();
                }
            });
        }
    }

    fn arm_hud_timers(&mut self) {
        for timer in self.core.take_hud_timers() {
            self.after(timer.delay, "hud auto-hide", move |state| {
                if !state.quitting() && state.core.fire_hud_hide(timer) {
                    state.settle();
                }
            });
        }
    }

    /// Auto-follow moves the selection only: no window holds the keyboard here.
    fn arm_auto_follow(&mut self) {
        for timer in self.core.take_auto_follow_timers() {
            self.after(timer.delay, "auto-follow", move |state| state.fire_auto_follow(timer));
        }
    }

    fn fire_auto_follow(&mut self, timer: AutoFollowTimer) {
        if self.quitting() {
            return;
        }
        drop(self.core.fire_auto_follow(timer));
        self.settle();
    }

    /// The presentation heartbeat (upstream's `startPresentationHeartbeat`); it re-arms itself while a stream is up.
    fn arm_presentation_heartbeat(&mut self) {
        if self.heartbeat_armed {
            return;
        }
        self.heartbeat_armed = self.after(ngterm_core::remote::HEARTBEAT_INTERVAL, "heartbeat", |state| {
            state.heartbeat_armed = false;
            if state.quitting() {
                return;
            }
            let streams_up = state.core.presentation_heartbeat();
            state.settle();
            if streams_up {
                state.arm_presentation_heartbeat();
            }
        });
    }

    /// The remote tick when the core asks for one (upstream's `startRemoteTick`).
    fn arm_remote_tick(&mut self) {
        if self.remote_tick_armed {
            return;
        }
        let Some(delay) = self.core.remote_wake() else {
            return;
        };
        self.remote_tick_armed = self.after(delay, "remote tick", |state| {
            state.remote_tick_armed = false;
            if state.quitting() {
                return;
            }
            state.core.remote_tick(Instant::now());
            state.settle();
        });
    }

    fn schedule(&self, cmd: Command, after: Duration, resume: Resume, sender: Waiter) {
        self.after(after, "deferred reply", move |state| state.resume(cmd, resume, sender));
    }

    /// One step of a waiting command; a panic in it answers `ok:false` as a handler's panic does.
    fn resume(&mut self, cmd: Command, resume: Resume, sender: Waiter) {
        let step = panic::catch_unwind(AssertUnwindSafe(|| resume(&mut self.core)));
        self.settle();
        match step {
            Ok(Dispatched::Done(response)) => sender.send(response),
            Ok(Dispatched::Pending { after, resume }) => self.schedule(cmd, after, resume, sender),
            Err(_) => {
                tracing::error!(%cmd, "control handler panicked in a deferred step");
                sender.send(Response::from(ControlError::HandlerPanicked(cmd)));
            }
        }
    }

    /// Quits a moment after a command asked for it (`live.reset`), so its reply is written first. Nothing relaunches
    /// a headless instance: whoever started it (a service manager) does.
    fn schedule_live_quit(&self, relaunch: bool) {
        tracing::info!(relaunch, "quitting for the live sessions reset");
        self.after(LIVE_QUIT_DELAY, "live sessions reset quit", Self::request_quit);
    }

    /// The quit save, once, before any terminal goes: every pending close made final and every window written, the
    /// viewers' streams shut. The sessions themselves live on with the session server.
    pub fn save_on_quit(&mut self) {
        if self.quitting() {
            return;
        }
        self.core.capture_on_exit();
        if !self.core.save_all() {
            tracing::error!("the quit save did not write everything");
        }
        self.core.shutdown_presentation_streams();
        self.core.shutdown_remote_presentations();
        self.phase = Phase::Quitting;
    }

    /// Detaches every terminal from its session (after the quit save).
    pub fn teardown(&mut self) {
        self.panes.teardown_all();
    }
}

/// Arms what a commit leaves to the main loop (the GPUI frontend's `arm_commit`).
fn arm_commit(main_loop: &dyn MainLoop<HeadlessState>, timers: &CommitTimers) {
    arm_debounced_saves(main_loop, &timers.debounced, fire_debounced);
    for timer in timers.pending_closes.iter().copied() {
        let _ = main_loop.post_after(
            timer.grace,
            Box::new(move |state: &mut HeadlessState| finalize_pending_close(state, timer)),
        );
    }
    for timer in timers.tree_changed.iter().copied() {
        let _ = main_loop.post_after(
            TREE_CHANGED_DEBOUNCE,
            Box::new(move |state: &mut HeadlessState| fire_tree_changed(state, timer)),
        );
    }
    for timer in timers.pane_focused.iter().copied() {
        let _ = main_loop.post_after(
            PANE_FOCUS_DEBOUNCE,
            Box::new(move |state: &mut HeadlessState| fire_pane_focus(state, timer)),
        );
    }
}

/// The `id` field of a response's result.
fn result_id(response: &Response) -> Option<String> {
    response
        .result
        .as_ref()
        .and_then(|result| result.get("id"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
}

/// A presentation stream's line handler: each line a main-loop task, at most `INBOUND_LIMIT` waiting at once.
fn presentation_reader(poster: Poster, serial: u64) -> impl FnMut(Vec<u8>) + Send + 'static {
    let gate = InboundGate::new(ngterm_core::remote::INBOUND_LIMIT);
    move |line| {
        gate.acquire();
        let released = gate.clone();
        let task = move |state: &mut HeadlessState| {
            if !state.quitting() {
                state.core.presentation_line(serial, &line);
                state.settle();
            }
            released.release();
        };
        let queued = poster
            .upgrade()
            .is_some_and(|main_loop| main_loop.post(Box::new(task)).is_ok());
        if !queued {
            gate.release();
        }
    }
}

fn presentation_closer(poster: Poster, serial: u64) -> impl FnOnce() + Send + 'static {
    move || {
        post(&poster, move |state| {
            if !state.quitting() {
                state.core.presentation_closed(serial);
                state.settle();
            }
        });
    }
}

fn page_serve_closed(state: &mut HeadlessState, stream: &Arc<PageServeStream>) {
    if !state.quitting() {
        state.core.page_serve_closed(stream);
        state.settle();
    }
}

fn fire_debounced(state: &mut HeadlessState, save: DebouncedSave) {
    if !state.quitting() {
        state.core.fire_debounced(save);
    }
}

fn fire_tree_changed(state: &mut HeadlessState, timer: TreeChangedTimer) {
    if !state.quitting() {
        state.core.fire_tree_changed(timer);
    }
}

fn fire_pane_focus(state: &mut HeadlessState, timer: PaneFocusTimer) {
    if state.quitting() {
        return;
    }
    state.core.fire_pane_focus(timer);
    state.settle();
}

fn finalize_pending_close(state: &mut HeadlessState, timer: PendingCloseTimer) {
    if state.quitting() {
        return;
    }
    state.core.finalize_pending_close(timer);
    state.settle();
}
