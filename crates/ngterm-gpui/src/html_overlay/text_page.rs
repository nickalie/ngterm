//! A text page's app side (upstream's `TextPageBridge.swift`), shared by the three web engines: it builds the page's
//! documents, takes the renderer's reports through one page handler ([`HANDLER`], registered in the renderer's world
//! only), makes the app's calls into that world ([`PageChannel::call`], which answer), hands the page the model's
//! search, and owns a log page's [`LogFollower`].
//!
//! A static text page reads its file on the background executor and keeps the text, so a theme change redraws without
//! reading it again; a shown page reopens where it was scrolled. A log page's documents are its follower's. Every
//! document carries the model's search (query, revision and the match shown) and its build generation (`data-load`).
//!
//! The renderer (`text.js`) posts `{ready}` once a new document can take updates, and its wrapper
//! (`documents::text_script`) then `{ready, load}` naming the build the document came from, which releases the pushes
//! held during that build: the search, and a log page's follow state. `text.js` posts `{following, top}` after a user
//! scroll of a log page, `{search}` for a query typed into its field and `{report}` for the matches it counted. The
//! model's follow state reaches the follower on every apply ([`TextPage::apply`]), where a pause drops the reads in
//! flight and a resume ticks at once; a moved search revision is pushed to the renderer, marked typed when the page's
//! own field made it, so the echo never overwrites later typing.
//!
//! "On screen" is the page's `on_screen` as the registry applies it: the page's deck slot is in the window's tree and
//! not hidden (a hidden slot, a session in another tab or a pending close, a page moved out are all off screen). The
//! ticks run, one per [`LogFollower::interval`], only while the page follows and is on screen; they end when the page
//! leaves the screen or closes ([`TextPage::stop`]). The page's reads run on the background executor and land on the
//! main thread, which the app hands this module at its start ([`init`]).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeSet, HashMap};
use std::rc::{Rc, Weak};

use gpui_kit::{App, AppContext as _, AsyncApp, Task};
use ngterm_app::log_follower::{LogFollower, LogRead, LogSink, ReadResult, Update};
use ngterm_core::html::text_page::{self as core_text, PageState, SearchState, TextLogReport};
use ngterm_core::html::{HtmlOverlay, HtmlSource, SearchAnswer, SearchCount, SearchStep};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::state::{AppState, Poster, post};

/// The renderer's handler (`window.webkit.messageHandlers.ngtermTextPage`, upstream's `agtermTextPage`), in the
/// renderer's world.
pub(super) const HANDLER: &str = "ngtermTextPage";

/// Reads where a shown page is scrolled, so its rebuild reopens there.
const SCROLL_TOP: &str = "document.scrollingElement.scrollTop";

/// What a call into the renderer's world answered: its value as JSON (`None` for `undefined`), or why it failed.
pub(super) type Answer = Result<Option<String>, String>;

/// The engine side of a text page, as each backend reaches it.
pub(super) trait PageChannel {
    /// Whether the page still holds its slot: a released page reads nothing.
    fn holds_slot(&self) -> bool;
    /// Loads `html` as the page's document.
    fn show_document(&self, html: String);
    /// Shows that the page's document could not be built.
    fn fail(&self, message: String);
    /// Evaluates `expression` in the renderer's world; its answer arrives on the channel, when the engine gives one.
    fn call(&self, expression: String) -> Option<async_channel::Receiver<Answer>>;
}

/// A report the renderer posted (upstream's `PageHandler`).
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Message {
    /// A new document can take updates; `load` is the build it came from, which only the wrapper's report names.
    Ready { load: Option<u64> },
    /// The user scrolled a log page: whether it now sits at the bottom, and its offset.
    Following { following: bool, top: Option<f64> },
    /// A query typed into the page's own field, empty for a closed search.
    Search(String),
    /// What the page counted for `query` at `revision`.
    Report {
        revision: u64,
        query: String,
        count: SearchCount,
    },
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
        if let Some(query) = body.get("search").and_then(Value::as_str) {
            return Some(Self::Search(query.to_owned()));
        }
        if let Some(report) = body.get("report") {
            return Self::report(report);
        }
        let following = body.get("following")?.as_bool()?;
        Some(Self::Following {
            following,
            top: body.get("top").and_then(Value::as_f64),
        })
    }

    fn report(report: &Value) -> Option<Self> {
        let number = |name: &str| report.get(name).and_then(Value::as_u64);
        let count = |name: &str| number(name).and_then(|value| usize::try_from(value).ok());
        Some(Self::Report {
            revision: number("revision")?,
            query: report.get("query")?.as_str()?.to_owned(),
            count: SearchCount {
                matches: count("matches")?,
                current: count("current")?,
            },
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
    call_of("ngtermTextUpdate", &arguments)
}

/// The renderer's follow call (upstream's `agtermTextSetFollowing`).
fn following_call(following: bool) -> String {
    format!("ngtermTextSetFollowing({following})")
}

/// The renderer's search call (upstream's `agtermTextSearchSet`): the model's query at `revision`, `typed` when the
/// page's own field made that revision.
fn search_call(search: &SearchState, typed: bool) -> String {
    call_of(
        "ngtermTextSearchSet",
        &[json!(search.query), json!(search.revision), json!(typed)],
    )
}

/// The renderer's step call (upstream's `agtermTextSearchStep`), which answers the count.
fn step_call(step: &SearchStep) -> String {
    call_of(
        "ngtermTextSearchStep",
        &[json!(step.forward), json!(step.query), json!(step.revision)],
    )
}

/// The count a step's answer carries: `{matches, current}`.
fn step_count(answer: &str) -> Option<SearchCount> {
    let value: Value = serde_json::from_str(answer).ok()?;
    let count = |name: &str| {
        value
            .get(name)
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
    };
    Some(SearchCount {
        matches: count("matches")?,
        current: count("current")?,
    })
}

/// `function(arguments…)`, each argument as JSON, so a text stays data.
fn call_of(function: &str, arguments: &[Value]) -> String {
    let arguments: Vec<String> = arguments.iter().map(Value::to_string).collect();
    format!("{function}({})", arguments.join(","))
}

/// The search a page's documents start with: the model's query and revision, and the match it showed last.
fn search_state(overlay: &HtmlOverlay) -> SearchState {
    SearchState {
        query: overlay
            .text_search
            .as_ref()
            .map(|search| search.query.clone())
            .unwrap_or_default(),
        revision: overlay.search_revision,
        current: overlay.text_search.as_ref().map_or(0, |search| search.current),
    }
}

thread_local! {
    /// The app the reads and ticks run in, from [`init`].
    static APP: RefCell<Option<AsyncApp>> = const { RefCell::new(None) };
    /// The live bridges by page id, which the model's tasks and the control API reach.
    static BRIDGES: RefCell<HashMap<Uuid, Weak<TextPage>>> = RefCell::new(HashMap::new());
}

/// Hands the text pages the app their reads and ticks run in (each backend's registry `init`).
pub(super) fn init(cx: &App) {
    APP.with(|app| app.replace(Some(cx.to_async())));
}

fn app() -> Option<AsyncApp> {
    APP.with(|app| app.borrow().clone())
}

fn bridge(page: Uuid) -> Option<Rc<TextPage>> {
    BRIDGES.with_borrow(|bridges| bridges.get(&page).and_then(Weak::upgrade))
}

/// Steps text page `page`'s search once (the core's `HtmlHost::step_text_search`); `None` while its renderer is not
/// up to take it.
pub(super) fn step(page: Uuid, step: &SearchStep) -> Option<SearchAnswer> {
    bridge(page)?.step(step)
}

/// Moves the keyboard into text page `page`'s own search field (upstream's `focusSearch`); the deck gives the page's
/// element the keyboard.
pub(crate) fn focus_search(page: Uuid) {
    if let Some(bridge) = bridge(page) {
        bridge.run("ngtermTextFocusSearch()".to_owned());
    }
}

/// A static text page's document state.
#[derive(Default)]
struct Static {
    /// The text shown, kept so a theme change redraws without reading the file again.
    text: RefCell<Option<String>>,
    /// Bumped by every document build and by [`TextPage::stop`], so a slow build cannot replace a newer one.
    load: Cell<u64>,
    /// From a build until its renderer reports ready; the ready report delivers the latest search.
    pending: Cell<bool>,
}

/// One text page's bridge.
pub(super) struct TextPage {
    id: Uuid,
    path: String,
    line: Option<i64>,
    poster: Poster,
    channel: Weak<dyn PageChannel>,
    /// A log page's follower; `None` on a static text page, whose documents [`Static`] tracks.
    follower: Option<RefCell<LogFollower>>,
    page: Static,
    /// The model's search as last applied.
    search: RefCell<SearchState>,
    /// The revisions the page's own field produced, so their pushes reach it as echoes and never overwrite its typing.
    typed: RefCell<BTreeSet<u64>>,
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
        let HtmlSource::Text { path, line } = &overlay.source else {
            return None;
        };
        let follower = overlay.text_log.map(|log| {
            let following = overlay.log_state.map_or(log.follow, |state| state.following);
            let mut follower = LogFollower::new(path.clone(), log, following, super::documents::text_style());
            // the page is off screen until its first apply
            follower.set_on_screen(false);
            RefCell::new(follower)
        });
        let page = Rc::new_cyclic(|me| Self {
            id: overlay.id,
            path: path.clone(),
            line: *line,
            poster: poster.clone(),
            channel,
            follower,
            page: Static::default(),
            search: RefCell::new(search_state(overlay)),
            typed: RefCell::new(BTreeSet::new()),
            ticks: RefCell::new(None),
            posted_following: Cell::new(None),
            me: me.clone(),
        });
        BRIDGES.with_borrow_mut(|bridges| bridges.insert(overlay.id, Rc::downgrade(&page)));
        Some(page)
    }

    /// Whether this is a log page, whose documents its follower builds.
    #[cfg(test)]
    pub(super) fn is_log(&self) -> bool {
        self.follower.is_some()
    }

    /// Builds the page's document from a fresh read, or from what it shows when `fresh` is false (a theme change); the
    /// search rides the document, so a rebuild keeps the query and the current match (upstream's `loadDocument`).
    pub(super) fn load_document(&self, fresh: bool) {
        let Some(follower) = &self.follower else {
            return self.load_static(fresh);
        };
        let read = {
            let mut follower = follower.borrow_mut();
            follower.set_search(self.search.borrow().clone());
            follower.load_document(fresh, &self.sink())
        };
        if let Some(read) = read {
            self.run_read(read);
        }
    }

    /// A static page's build: the file read on the background executor when `fresh` or never read, the scroll offset
    /// of a page already shown read first, so a reload or a theme change never moves the view.
    fn load_static(&self, fresh: bool) {
        let load = self.page.load.get().wrapping_add(1);
        self.page.load.set(load);
        self.page.pending.set(true);
        let shown = self.page.text.borrow().is_some();
        let read = (fresh || !shown).then(|| self.path.clone());
        let Some(app) = app() else {
            // the unit tests: in place, with no offset to keep
            let read = read.map(|path| ngterm_app::documents::load_text(std::path::Path::new(&path)));
            return self.build(load, read, None);
        };
        let top = shown
            .then(|| self.channel.upgrade()?.call(SCROLL_TOP.to_owned()))
            .flatten();
        let me = self.me.clone();
        app.spawn(async move |cx: &mut AsyncApp| {
            let top = match top {
                Some(answer) => answer.recv().await.ok().and_then(Result::ok).flatten(),
                None => None,
            };
            let read = match read {
                Some(path) => Some(
                    cx.background_spawn(async move { ngterm_app::documents::load_text(std::path::Path::new(&path)) })
                        .await,
                ),
                None => None,
            };
            if let Some(page) = me.upgrade() {
                page.build(load, read, top.and_then(|top| top.parse().ok()));
            }
        })
        .detach();
    }

    /// Shows build `load`'s document, unless a newer build or the page's close overtook it.
    fn build(&self, load: u64, read: Option<Result<String, String>>, anchor: Option<f64>) {
        if load != self.page.load.get() {
            return;
        }
        let Some(channel) = self.channel.upgrade() else { return };
        match read {
            Some(Err(failure)) => return channel.fail(failure),
            Some(Ok(text)) => {
                self.page.text.replace(Some(text));
            }
            None => {}
        }
        let text = self.page.text.borrow();
        let Some(text) = text.as_deref() else { return };
        let search = self.search.borrow();
        let state = PageState {
            anchor,
            load: Some(load),
            search: Some(&search),
            ..PageState::default()
        };
        let style = super::documents::text_style();
        channel.show_document(core_text::html(
            text,
            core_text::language(&self.path),
            self.line,
            &style,
            &state,
        ));
    }

    /// Whether a document build waits for its renderer: nothing is pushed into it meanwhile.
    fn document_pending(&self) -> bool {
        self.follower
            .as_ref()
            .map_or(self.page.pending.get(), |follower| follower.borrow().document_pending())
    }

    /// Takes the model's latest value and whether the page is on screen (upstream's `apply` and `onScreen`): a log
    /// page's follow state, and a search whose revision moved.
    pub(super) fn apply(&self, overlay: &HtmlOverlay, on_screen: bool) {
        self.apply_following(overlay, on_screen);
        let next = search_state(overlay);
        let moved = next.revision != self.search.borrow().revision;
        self.search.replace(next);
        if moved && !self.document_pending() {
            self.push_search();
        }
    }

    fn apply_following(&self, overlay: &HtmlOverlay, on_screen: bool) {
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
        match Message::parse(body) {
            Some(Message::Ready { load: Some(load) }) => self.renderer_ready(load),
            Some(Message::Following { following, top }) => self.scrolled(following, top),
            Some(Message::Search(query)) => self.typed(query),
            Some(Message::Report { revision, query, count }) => {
                let id = self.id;
                post(&self.poster, move |state: &mut AppState| {
                    state.core.set_text_search_report(id, revision, &query, count);
                });
            }
            // `text.js`'s own ready report names no build: the wrapper's next one does
            Some(Message::Ready { load: None }) | None => {}
        }
    }

    /// The document built as `load` can take updates: the latest search goes to it, and a log page's state.
    fn renderer_ready(&self, load: u64) {
        match &self.follower {
            Some(follower) => {
                let released = {
                    let mut follower = follower.borrow_mut();
                    let pending = follower.document_pending();
                    follower.renderer_ready(load, &self.sink());
                    pending && !follower.document_pending()
                };
                if released {
                    self.push_search();
                }
                self.update_ticks();
            }
            None => {
                if self.page.pending.get() && load == self.page.load.get() {
                    self.page.pending.set(false);
                    self.push_search();
                }
            }
        }
    }

    fn scrolled(&self, following: bool, top: Option<f64>) {
        let Some(follower) = &self.follower else { return };
        {
            let mut follower = follower.borrow_mut();
            let before = follower.following();
            follower.scrolled(following, top, &self.sink());
            if follower.following() != before {
                self.posted_following.set(Some(follower.following()));
            }
        }
        self.update_ticks();
    }

    /// A query typed into the page's field goes to the model; the revision it makes is marked typed before the apply
    /// that pushes it (upstream's `typed`).
    fn typed(&self, query: String) {
        let id = self.id;
        post(&self.poster, move |state: &mut AppState| {
            let Some(revision) = state.core.set_text_search_from_page(id, &query) else {
                return;
            };
            if let Some(page) = bridge(id) {
                page.typed.borrow_mut().insert(revision);
            }
            state.settle();
        });
    }

    /// Hands the renderer the model's search, marked typed when the page's own field made its revision.
    fn push_search(&self) {
        let search = self.search.borrow().clone();
        let typed = {
            let mut typed = self.typed.borrow_mut();
            let own = typed.contains(&search.revision);
            typed.retain(|revision| *revision > search.revision);
            own
        };
        self.run(search_call(&search, typed));
    }

    /// Steps the search once (upstream's `step`): the renderer applies the step's query first when it holds an older
    /// revision, and answers its count; `None` while a document build waits for its renderer.
    fn step(&self, step: &SearchStep) -> Option<SearchAnswer> {
        if self.document_pending() {
            return None;
        }
        let answer = self.channel.upgrade()?.call(step_call(step))?;
        let app = app()?;
        let (sender, counted) = std::sync::mpsc::channel();
        app.spawn(async move |_: &mut AsyncApp| {
            let count = answer.recv().await.ok().and_then(Result::ok).flatten();
            if let Some(count) = count.as_deref().and_then(step_count) {
                let _ = sender.send(count);
            }
        })
        .detach();
        Some(counted)
    }

    /// Ends the ticks and drops every read and build in flight (the page closed).
    pub(super) fn stop(&self) {
        if let Some(follower) = &self.follower {
            follower.borrow_mut().stop();
        }
        self.page.load.set(self.page.load.get().wrapping_add(1));
        self.ticks.replace(None);
        BRIDGES.with_borrow_mut(|bridges| {
            if bridges.get(&self.id).is_some_and(|bridge| bridge.ptr_eq(&self.me)) {
                bridges.remove(&self.id);
            }
        });
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

    /// Runs `expression` in the renderer's world, its answer dropped.
    fn run(&self, expression: String) {
        if let Some(channel) = self.channel.upgrade() {
            drop(channel.call(expression));
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

    /// Upstream's `PageHandler`: a ready report (the build it names optional), a scroll report (its offset optional), a
    /// typed query, a count, nothing else.
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
        assert_eq!(Message::parse(r#"{"search":""}"#), Some(Message::Search(String::new())));
        assert_eq!(
            Message::parse(r#"{"report":{"revision":4,"query":"a","matches":3,"current":2}}"#),
            Some(Message::Report {
                revision: 4,
                query: "a".to_owned(),
                count: SearchCount { matches: 3, current: 2 },
            })
        );
        for other in [
            r#"{"ready":false}"#,
            r#"{"search":1}"#,
            r#"{"report":{"revision":4,"matches":3,"current":2}}"#,
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
        let search = SearchState {
            query: "a\"</script>".to_owned(),
            revision: 3,
            current: 1,
        };
        assert_eq!(
            search_call(&search, true),
            r#"ngtermTextSearchSet("a\"</script>",3,true)"#
        );
        let step = SearchStep {
            forward: false,
            query: "b".to_owned(),
            revision: 2,
        };
        assert_eq!(step_call(&step), r#"ngtermTextSearchStep(false,"b",2)"#);
    }

    /// A step answers the renderer's `{matches, current}`; anything else is no count.
    #[test]
    fn a_step_answer_carries_its_count() {
        assert_eq!(
            step_count(r#"{"matches":3,"current":1}"#),
            Some(SearchCount { matches: 3, current: 1 })
        );
        for other in ["null", r#"{"matches":3}"#, r#"{"matches":-1,"current":0}"#, "nope"] {
            assert_eq!(step_count(other), None, "{other}");
        }
    }

    // MARK: - a static page's documents and its search

    /// The engine's stand-in: every document, failure and call recorded; each call answered with `answer` when set.
    #[derive(Default)]
    struct Channel {
        documents: RefCell<Vec<String>>,
        failures: RefCell<Vec<String>>,
        calls: RefCell<Vec<String>>,
        answer: RefCell<Option<String>>,
    }

    impl PageChannel for Channel {
        fn holds_slot(&self) -> bool {
            true
        }
        fn show_document(&self, html: String) {
            self.documents.borrow_mut().push(html);
        }
        fn fail(&self, message: String) {
            self.failures.borrow_mut().push(message);
        }
        fn call(&self, expression: String) -> Option<async_channel::Receiver<Answer>> {
            self.calls.borrow_mut().push(expression);
            let (sender, answer) = async_channel::bounded(1);
            if let Some(value) = self.answer.borrow().clone() {
                let _ = sender.try_send(Ok(Some(value)));
            }
            Some(answer)
        }
    }

    impl Channel {
        fn take_calls(&self) -> Vec<String> {
            std::mem::take(&mut *self.calls.borrow_mut())
        }

        fn last_document(&self) -> String {
            self.documents.borrow().last().cloned().expect("a document")
        }
    }

    /// A static text page over `path` whose model holds `query` at `revision`, its bridge on `channel`.
    fn static_page(path: &std::path::Path, query: &str, revision: u64) -> (HtmlOverlay, Rc<Channel>, Rc<TextPage>) {
        let mut overlay = HtmlOverlay::new(
            HtmlSource::Text {
                path: path.to_string_lossy().into_owned(),
                line: Some(2),
            },
            ngterm_core::html::HtmlPageFlags::default(),
        );
        overlay.search_revision = revision;
        overlay.text_search = (!query.is_empty()).then(|| ngterm_core::html::text_page::TextSearch {
            query: query.to_owned(),
            matches: 3,
            current: 2,
        });
        let channel = Rc::new(Channel::default());
        let engine: Rc<dyn PageChannel> = channel.clone();
        let page = TextPage::new(&overlay, Rc::downgrade(&engine), &crate::state::detached_poster()).unwrap();
        (overlay, channel, page)
    }

    /// `overlay` with the model's query moved to `query` at `revision`.
    fn searched(overlay: &HtmlOverlay, query: &str, revision: u64) -> HtmlOverlay {
        let mut overlay = overlay.clone();
        overlay.search_revision = revision;
        overlay.text_search = Some(ngterm_core::html::text_page::TextSearch::new(query));
        overlay
    }

    /// The ready report the wrapper posts for `document`.
    fn ready(document: &str) -> String {
        let (_, rest) = document
            .split_once(" data-load=\"")
            .expect("a document names its build");
        format!(r#"{{"ready":true,"load":{}}}"#, rest.split_once('"').unwrap().0)
    }

    /// Upstream's `TextPageBridge.loadDocument` and `apply`: a static page's document carries the model's search and
    /// its build; the search goes to the renderer only once that build's renderer is ready, and then whenever its
    /// revision moves, marked typed for a revision the page's own field made.
    #[test]
    fn a_static_page_builds_with_its_search_and_pushes_it_once_ready() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let (overlay, channel, page) = static_page(&path, "two", 1);
        assert!(!page.is_log());
        page.load_document(true);
        let built = channel.last_document();
        assert!(
            built.contains(
                " hidden data-revision=\"1\" data-query=\"two\" data-current=\"2\" data-line=\"2\" data-load=\"1\">one\ntwo\n</div>"
            ),
            "{built}"
        );
        page.apply(&searched(&overlay, "tw", 2), true);
        assert!(
            channel.take_calls().is_empty(),
            "nothing goes into a document still loading"
        );
        page.message(r#"{"ready":true}"#);
        page.message(r#"{"ready":true,"load":7}"#);
        assert!(
            channel.take_calls().is_empty(),
            "another build's report releases nothing"
        );
        page.message(&ready(&built));
        assert_eq!(channel.take_calls(), [r#"ngtermTextSearchSet("tw",2,false)"#]);
        page.apply(&searched(&overlay, "tw", 2), true);
        assert!(
            channel.take_calls().is_empty(),
            "an unmoved revision is not pushed again"
        );
        page.typed.borrow_mut().extend([3, 5]);
        page.apply(&searched(&overlay, "t", 3), true);
        assert_eq!(channel.take_calls(), [r#"ngtermTextSearchSet("t",3,true)"#]);
        assert_eq!(*page.typed.borrow(), BTreeSet::from([5]), "older typed revisions go");
        page.apply(&searched(&overlay, "x", 4), true);
        assert_eq!(channel.take_calls(), [r#"ngtermTextSearchSet("x",4,false)"#]);
        page.stop();
        assert!(bridge(overlay.id).is_none(), "a stopped page leaves the bridges");
    }

    /// A static page whose file cannot be read shows the probe's failure.
    #[test]
    fn a_static_page_that_cannot_be_read_fails() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blob.txt");
        std::fs::write(&path, b"a\0b").unwrap();
        let (_, channel, page) = static_page(&path, "", 0);
        page.load_document(true);
        assert!(channel.documents.borrow().is_empty());
        assert_eq!(
            *channel.failures.borrow(),
            [format!("not a UTF-8 text file: {}", path.display())]
        );
    }

    /// Upstream's `step` and `loadDocument(fresh: false)`, on the app's executor: a step goes to the ready renderer and
    /// answers its count, and is refused while a build waits; a rebuild reopens where the page was scrolled from the
    /// text it keeps, never reading the file again.
    #[gpui_kit::test]
    fn a_step_answers_the_renderers_count_and_a_rebuild_keeps_the_view(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| init(cx));
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notes.txt");
        std::fs::write(&path, "one\ntwo\n").unwrap();
        let (overlay, channel, page) = static_page(&path, "o", 1);
        page.load_document(true);
        cx.run_until_parked();
        let built = channel.last_document();
        let request = SearchStep {
            forward: true,
            query: "o".to_owned(),
            revision: 1,
        };
        assert!(step(overlay.id, &request).is_none(), "the build waits for its renderer");
        page.message(&ready(&built));
        channel.take_calls();
        channel.answer.replace(Some(r#"{"matches":2,"current":2}"#.to_owned()));
        let answer = step(overlay.id, &request).expect("a ready renderer takes the step");
        cx.run_until_parked();
        assert_eq!(answer.try_recv(), Ok(SearchCount { matches: 2, current: 2 }));
        assert_eq!(channel.take_calls(), [r#"ngtermTextSearchStep(true,"o",1)"#]);

        std::fs::remove_file(&path).unwrap();
        channel.answer.replace(Some("120".to_owned()));
        page.load_document(false);
        cx.run_until_parked();
        assert_eq!(channel.take_calls(), [SCROLL_TOP]);
        let rebuilt = channel.last_document();
        assert!(
            rebuilt.contains(
                " data-revision=\"1\" data-query=\"o\" data-current=\"2\" data-line=\"2\" data-anchor=\"120.0\""
            ) && rebuilt.contains(">one\ntwo\n</div>"),
            "{rebuilt}"
        );
        assert!(channel.failures.borrow().is_empty(), "the kept text, not a read");
        focus_search(overlay.id);
        assert_eq!(channel.take_calls(), ["ngtermTextFocusSearch()"]);
    }
}
