//! A text page's app side (upstream's `TextPageBridge.swift`), shared by the three web engines: the renderer's reports
//! through one page handler ([`HANDLER`], registered in the renderer's world only), the app's calls into that world
//! ([`PageChannel::call`], which answer), and a log page's [`LogFollower`].
//!
//! The renderer (`text.js`) posts `{ready}` once a new document can take updates, and its wrapper
//! (`documents::text_script`) then `{ready, load}` naming the build the document came from, which releases the pushes
//! held during that build and resends the follow state; `text.js` posts `{following, top}` after a user scroll. The model's follow state
//! reaches the follower on every apply ([`TextPage::apply`]), where a pause drops the reads in flight and a resume
//! ticks at once.
//!
//! "On screen" is the page's `on_screen` as the registry applies it: the page's deck slot is in the window's tree and
//! not hidden (a hidden slot, a session in another tab or a pending close, a page moved out are all off screen). The
//! ticks run, one per [`LogFollower::interval`], only while the page follows and is on screen; they end when the page
//! leaves the screen or closes ([`TextPage::stop`]). The page's reads run on the background executor and land on the
//! main thread, which the app hands this module at its start ([`init`]).

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use gpui_kit::{App, AppContext as _, AsyncApp, Task};
use ngterm_app::log_follower::{LogFollower, LogRead, LogSink, ReadResult, Update};
use ngterm_core::html::text_page::TextLogReport;
use ngterm_core::html::{HtmlOverlay, HtmlSource};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::state::{AppState, Poster, post};

/// The renderer's handler (`window.webkit.messageHandlers.ngtermTextPage`, upstream's `agtermTextPage`), in the
/// renderer's world.
pub(super) const HANDLER: &str = "ngtermTextPage";

/// What a call into the renderer's world answered: its value as JSON (`None` for `undefined`), or why it failed.
pub(super) type Answer = Result<Option<String>, String>;

/// The engine side of a text page, as each backend reaches it.
pub(super) trait PageChannel {
    /// Whether the page still holds its slot: a released page reads nothing.
    fn holds_slot(&self) -> bool;
    /// Loads `html` as the page's document.
    fn show_document(&self, html: String);
    /// Evaluates `expression` in the renderer's world; its answer arrives on the channel, when the engine gives one.
    fn call(&self, expression: String) -> Option<async_channel::Receiver<Answer>>;
}

/// A report the renderer posted (upstream's `PageHandler`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Message {
    /// A new document can take updates; `load` is the build it came from, which only a log document names.
    Ready { load: Option<u64> },
    /// The user scrolled a log page: whether it now sits at the bottom, and its offset.
    Following { following: bool, top: Option<f64> },
}

impl Message {
    /// The report `body` (a JSON object) carries; `None` for anything else.
    pub(super) fn parse(body: &str) -> Option<Self> {
        let body: Value = serde_json::from_str(body).ok()?;
        if body.get("ready").and_then(Value::as_bool) == Some(true) {
            return Some(Self::Ready {
                load: body.get("load").and_then(Value::as_u64),
            });
        }
        let following = body.get("following")?.as_bool()?;
        Some(Self::Following {
            following,
            top: body.get("top").and_then(Value::as_f64),
        })
    }
}

/// The renderer's update call (upstream's `agtermTextUpdate`).
fn update_call(update: &Update) -> String {
    let arguments = [
        json!(update.text),
        json!(update.report.clipped),
        json!(update.report.waiting),
        json!(update.report.unreadable),
        json!(update.language),
        json!({
            "dropped": update.shift.dropped,
            "cut": update.shift.cut,
            "replaced": update.shift.replaced,
        }),
    ];
    let arguments: Vec<String> = arguments.iter().map(Value::to_string).collect();
    format!("ngtermTextUpdate({})", arguments.join(","))
}

/// The renderer's follow call (upstream's `agtermTextSetFollowing`).
fn following_call(following: bool) -> String {
    format!("ngtermTextSetFollowing({following})")
}

thread_local! {
    /// The app the reads and ticks run in, from [`init`].
    static APP: RefCell<Option<AsyncApp>> = const { RefCell::new(None) };
}

/// Hands the text pages the app their reads and ticks run in (each backend's registry `init`).
pub(super) fn init(cx: &App) {
    APP.with(|app| app.replace(Some(cx.to_async())));
}

fn app() -> Option<AsyncApp> {
    APP.with(|app| app.borrow().clone())
}

/// One text page's bridge.
pub(super) struct TextPage {
    id: Uuid,
    poster: Poster,
    channel: Weak<dyn PageChannel>,
    /// A log page's follower; `None` on a static text page.
    follower: Option<RefCell<LogFollower>>,
    /// The ticks while they run; dropping it ends them.
    ticks: RefCell<Option<Task<()>>>,
    /// A follow change the user's scroll made, posted to the model and not yet back in an apply: until it is, an
    /// apply carries the model's older value, which must not undo the scroll (upstream's model takes it in place).
    posted_following: Cell<Option<bool>>,
    me: Weak<TextPage>,
}

impl TextPage {
    /// The bridge of `overlay`'s page reached through `channel`; `None` for any page but a text page.
    pub(super) fn new(overlay: &HtmlOverlay, channel: Weak<dyn PageChannel>, poster: &Poster) -> Option<Rc<Self>> {
        let HtmlSource::Text { path, .. } = &overlay.source else {
            return None;
        };
        let follower = overlay.text_log.map(|log| {
            let following = overlay.log_state.map_or(log.follow, |state| state.following);
            let mut follower = LogFollower::new(path.clone(), log, following, super::documents::text_style());
            // the page is off screen until its first apply
            follower.set_on_screen(false);
            RefCell::new(follower)
        });
        Some(Rc::new_cyclic(|me| Self {
            id: overlay.id,
            poster: poster.clone(),
            channel,
            follower,
            ticks: RefCell::new(None),
            posted_following: Cell::new(None),
            me: me.clone(),
        }))
    }

    /// Whether this is a log page, whose documents its follower builds.
    pub(super) fn is_log(&self) -> bool {
        self.follower.is_some()
    }

    /// Builds a log page's document from a fresh read, or from what it shows when `fresh` is false (a theme change).
    pub(super) fn load_document(&self, fresh: bool) {
        let Some(follower) = &self.follower else { return };
        let read = follower.borrow_mut().load_document(fresh, &self.sink());
        if let Some(read) = read {
            self.run_read(read);
        }
    }

    /// Takes the model's latest value and whether the page is on screen (upstream's `apply` and `onScreen`).
    pub(super) fn apply(&self, overlay: &HtmlOverlay, on_screen: bool) {
        let Some(follower) = &self.follower else { return };
        {
            let mut follower = follower.borrow_mut();
            follower.set_on_screen(on_screen);
            let following = overlay.log_state.is_some_and(|state| state.following);
            match self.posted_following.get() {
                Some(posted) if posted != following => {}
                _ => {
                    self.posted_following.set(None);
                    follower.set_following(following, &self.sink());
                }
            }
        }
        self.update_ticks();
    }

    /// One report the renderer posted (`body`, its JSON).
    pub(super) fn message(&self, body: &str) {
        let Some(follower) = &self.follower else { return };
        match Message::parse(body) {
            Some(Message::Ready { load: Some(load) }) => follower.borrow_mut().renderer_ready(load, &self.sink()),
            Some(Message::Following { following, top }) => {
                let mut follower = follower.borrow_mut();
                let before = follower.following();
                follower.scrolled(following, top, &self.sink());
                if follower.following() != before {
                    self.posted_following.set(Some(follower.following()));
                }
            }
            // `text.js`'s own ready report names no build: the wrapper's next one does
            Some(Message::Ready { load: None }) | None => return,
        }
        self.update_ticks();
    }

    /// Ends the ticks and drops every read and build in flight (the page closed).
    pub(super) fn stop(&self) {
        if let Some(follower) = &self.follower {
            follower.borrow_mut().stop();
        }
        self.ticks.replace(None);
    }

    /// Whether the ticks run.
    #[cfg(all(test, target_os = "linux"))]
    pub(super) fn ticking(&self) -> bool {
        self.ticks.borrow().is_some()
    }

    fn sink(&self) -> Sink<'_> {
        Sink {
            page: self,
            channel: self.channel.upgrade(),
        }
    }

    /// Starts the ticks when the follower wants them and they do not run, ticking at once; ends them when it no longer
    /// does (upstream's `updateLoop`).
    fn update_ticks(&self) {
        let Some(follower) = &self.follower else { return };
        let (run, interval) = {
            let follower = follower.borrow();
            (follower.wants_ticks(), follower.interval())
        };
        if run == self.ticks.borrow().is_some() {
            return;
        }
        self.ticks.replace(None);
        if !run {
            return;
        }
        let Some(app) = app() else { return };
        let me = self.me.clone();
        let ticks = app.spawn(async move |cx: &mut AsyncApp| run_ticks(&me, interval, cx).await);
        self.ticks.replace(Some(ticks));
    }

    fn tick(&self) -> Option<LogRead> {
        self.follower.as_ref()?.borrow_mut().tick(&self.sink())
    }

    fn finish(&self, read: &LogRead, result: ReadResult) {
        if let Some(follower) = &self.follower {
            follower.borrow_mut().finish(read, result, &self.sink());
        }
    }

    /// Runs `read` on the background executor and lands it here; with no app (the unit tests), in place.
    fn run_read(&self, read: LogRead) {
        let Some(app) = app() else {
            let result = read.run();
            self.finish(&read, result);
            return;
        };
        let me = self.me.clone();
        app.spawn(async move |cx: &mut AsyncApp| {
            let job = read.clone();
            let result = cx.background_spawn(async move { job.run() }).await;
            if let Some(page) = me.upgrade() {
                page.finish(&read, result);
            }
        })
        .detach();
    }
}

/// Ticks `page` every `interval` until it is gone or the task is dropped, the first at once; each tick's read runs on
/// the background executor.
async fn run_ticks(page: &Weak<TextPage>, interval: std::time::Duration, cx: &mut AsyncApp) {
    while page.strong_count() > 0 {
        let read = page.upgrade().and_then(|page| page.tick());
        if let Some(read) = read {
            let job = read.clone();
            let result = cx.background_spawn(async move { job.run() }).await;
            if let Some(page) = page.upgrade() {
                page.finish(&read, result);
            }
        }
        cx.background_executor().timer(interval).await;
    }
}

/// The follower's way to the page and the model.
struct Sink<'a> {
    page: &'a TextPage,
    channel: Option<Rc<dyn PageChannel>>,
}

impl Sink<'_> {
    fn run(&self, expression: String) {
        if let Some(channel) = &self.channel {
            drop(channel.call(expression));
        }
    }
}

impl LogSink for Sink<'_> {
    fn in_slot(&self) -> bool {
        self.channel.as_ref().is_some_and(|channel| channel.holds_slot())
    }

    fn report(&self, report: TextLogReport) {
        let id = self.page.id;
        post(&self.page.poster, move |state: &mut AppState| {
            state.core.set_text_log_report(id, report);
        });
    }

    fn following(&self, following: bool) {
        let id = self.page.id;
        post(&self.page.poster, move |state: &mut AppState| {
            if state.core.set_text_log_following_from_page(id, following) {
                state.settle();
            }
        });
    }

    fn load(&self, html: String) {
        if let Some(channel) = &self.channel {
            channel.show_document(html);
        }
    }

    fn update(&self, update: &Update) {
        self.run(update_call(update));
    }

    fn set_following(&self, following: bool) {
        self.run(following_call(following));
    }
}

#[cfg(test)]
mod tests {
    use ngterm_app::log_follower::Shift;

    use super::*;

    /// Upstream's `PageHandler`: a ready report (the build it names optional), a scroll report (its offset optional),
    /// nothing else.
    #[test]
    fn the_renderer_reports_parse() {
        assert_eq!(Message::parse(r#"{"ready":true}"#), Some(Message::Ready { load: None }));
        assert_eq!(
            Message::parse(r#"{"ready":true,"load":3}"#),
            Some(Message::Ready { load: Some(3) })
        );
        assert_eq!(
            Message::parse(r#"{"following":false,"top":120.5}"#),
            Some(Message::Following {
                following: false,
                top: Some(120.5)
            })
        );
        assert_eq!(
            Message::parse(r#"{"following":true}"#),
            Some(Message::Following {
                following: true,
                top: None
            })
        );
        for other in [
            r#"{"ready":false}"#,
            r#"{"search":"a"}"#,
            "[]",
            "not json",
            r#"{"following":1}"#,
        ] {
            assert_eq!(Message::parse(other), None, "{other}");
        }
    }

    /// The calls carry their arguments as JSON, so the text stays data.
    #[test]
    fn the_calls_carry_their_arguments_as_json() {
        let update = Update {
            text: "a\"b\n</script>".to_owned(),
            report: TextLogReport {
                lines: 1,
                clipped: true,
                waiting: false,
                unreadable: true,
            },
            language: None,
            shift: Shift {
                dropped: 2,
                cut: true,
                replaced: false,
            },
        };
        assert_eq!(
            update_call(&update),
            r#"ngtermTextUpdate("a\"b\n</script>",true,false,true,null,{"dropped":2,"cut":true,"replaced":false})"#
        );
        assert_eq!(following_call(false), "ngtermTextSetFollowing(false)");
    }
}
