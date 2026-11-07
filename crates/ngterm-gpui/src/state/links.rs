//! A clicked terminal link carried out (upstream's `LinkOpener`, `ControlServer+TextPage.openTextLink`): the link
//! policy decides the route under the link-open setting and the click's chord ([`ClickIntent`]); a link only the file
//! system can resolve (a `path:line` location, a relative path) is looked up off the main thread, the first existing
//! candidate winning, and one with no candidate on disk keeps the route it has as typed. A reveal chord selects the
//! file a local route names in the file manager after that guarded routing, and leaves a web link to its web route.
//! A local document or text page is read off the main thread before it takes its slot; a refused open reveals it.

use std::path::{Path, PathBuf};

use ngterm_core::html::HtmlSource;
use ngterm_core::link_policy::{self, ClickIntent, ClickOrigin, LinkLocation, Route};
use ngterm_core::persist::settings::LinkOpenMode;
use uuid::Uuid;

use super::{AppState, post};

/// A local document a click shows over its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Document {
    Image,
    Markdown,
    /// A text page marked at its line.
    Text(Option<i64>),
}

impl Document {
    fn source(self, path: &Path) -> HtmlSource {
        let path = path.to_string_lossy().into_owned();
        match self {
            Self::Image => HtmlSource::Image { path },
            Self::Markdown => HtmlSource::Markdown { path },
            Self::Text(line) => HtmlSource::Text { path, line },
        }
    }
}

impl AppState {
    /// A link clicked in a terminal with no directory of its own to resolve a relative link against (a HUD, a program
    /// overlay, the quick terminal): [`Self::follow_link_at`].
    pub fn follow_link(&mut self, raw: &str, origin: ClickOrigin, intent: ClickIntent) {
        self.follow_link_at(raw, origin, None, intent);
    }

    /// A link clicked in a session's pane or scratch terminal: a detected link (`LinkKind::Url`) resolves a relative
    /// path against the still-live local pane's reported OSC 7 directory, never the model's cwd; an OSC 8 URI stays as
    /// written, as Ghostty's `processLinks` keeps it. A remote pane's paths name files on the origin.
    pub fn follow_pane_link(
        &mut self,
        raw: &str,
        session: Uuid,
        token: &str,
        intent: ClickIntent,
        kind: crate::terminal::LinkKind,
    ) {
        let Some(found) = self
            .core
            .library()
            .store_for_session(session)
            .and_then(|store| store.session(session))
        else {
            return;
        };
        let Some(role) = found.pane_role_for_token(token) else {
            return;
        };
        let scratch = role == ngterm_core::model::StatusPane::Scratch;
        let remote = !scratch && found.remote_host().is_some();
        let origin = if scratch {
            ClickOrigin::Scratch(session)
        } else if remote {
            ClickOrigin::RemotePane(session)
        } else {
            ClickOrigin::Pane(session)
        };
        let cwd = (kind == crate::terminal::LinkKind::Url)
            .then(|| self.link_directories.get(&(session, token.to_owned())).cloned())
            .flatten();
        self.follow_link_at(raw, origin, cwd.as_deref(), intent);
    }

    /// Carries out a clicked link (upstream's `LinkOpener.follow`): the route under the link-open setting, or under
    /// browser mode for the browser chord; the candidates [`link_policy::locations`] offers are stat'ed on a worker.
    fn follow_link_at(&mut self, raw: &str, origin: ClickOrigin, cwd: Option<&str>, intent: ClickIntent) {
        let mode = if intent == ClickIntent::Browser {
            LinkOpenMode::Browser
        } else {
            self.core.settings().effective_link_open_mode()
        };
        let reveal = intent == ClickIntent::Reveal;
        let candidates = link_policy::locations(raw, origin, cwd);
        if candidates.is_empty() {
            self.carry_out(raw, link_policy::route(raw, mode, origin), reveal);
            return;
        }
        let poster = self.poster.clone();
        let raw = raw.to_owned();
        self.spawn_link_work(move || {
            let found = first_existing(candidates);
            post(&poster, move |state: &mut AppState| {
                if state.quitting {
                    return;
                }
                let route = found.map_or_else(
                    || link_policy::route(&raw, mode, origin),
                    |location| link_policy::route_for_location(&location, mode, origin),
                );
                state.carry_out(&raw, route, reveal);
            });
        });
    }

    /// Does what `route` says; with `reveal`, a local route's file is revealed instead (upstream's `carryOut`).
    fn carry_out(&mut self, raw: &str, route: Route, reveal: bool) {
        if reveal && let Some(file) = route.local_file() {
            self.frontend.reveals.push(file.to_path_buf());
            return;
        }
        match route {
            Route::Overlay {
                url,
                session,
                size_percent,
            } => {
                let shown = url::Url::parse(&url)
                    .is_ok_and(|parsed| self.core.open_link_overlay(parsed, session, size_percent));
                if shown {
                    self.settle();
                } else {
                    self.frontend.open_urls.push(url);
                }
            }
            Route::Image { path, session } => self.open_document_link(path, session, None, Document::Image),
            Route::Markdown {
                path,
                session,
                size_percent,
            } => self.open_document_link(path, session, size_percent, Document::Markdown),
            Route::Text {
                path,
                line,
                session,
                size_percent,
            } => self.open_document_link(path, session, size_percent, Document::Text(line)),
            Route::Browser(url) => self.frontend.open_urls.push(url),
            Route::Reveal(path) => self.frontend.reveals.push(path),
            Route::Ignore => tracing::debug!(raw, "link click ignored by the link policy"),
        }
    }

    /// Shows a local document over `session` once a worker read it, the slot checked again then; a refused or failed
    /// open reveals the file.
    fn open_document_link(&mut self, path: PathBuf, session: Uuid, size_percent: Option<i64>, document: Document) {
        if !self.core.can_show_link_overlay(session) {
            self.frontend.reveals.push(path);
            return;
        }
        let poster = self.poster.clone();
        self.spawn_link_work(move || {
            let checked = match document {
                Document::Image => ngterm_app::documents::probe_image(&path).map(Some),
                Document::Markdown => ngterm_app::documents::check_markdown(&path).map(|()| None),
                Document::Text(_) => ngterm_app::documents::check_text(&path).map(|()| None),
            };
            post(&poster, move |state: &mut AppState| {
                if state.quitting {
                    return;
                }
                let source = document.source(&path);
                let shown =
                    checked.is_ok_and(|probe| state.core.open_document_link(source, session, size_percent, probe));
                if shown {
                    state.settle();
                } else {
                    state.frontend.reveals.push(path);
                }
            });
        });
    }

    /// Runs `work` on a worker thread: opening a file on slow storage can block.
    #[cfg_attr(
        not(test),
        expect(
            clippy::unused_self,
            reason = "a test build keeps the worker's handle to wait for it"
        )
    )]
    fn spawn_link_work(&mut self, work: impl FnOnce() + Send + 'static) {
        let handle = std::thread::spawn(work);
        #[cfg(test)]
        self.link_lookups.push(handle);
        #[cfg(not(test))]
        drop(handle);
    }

    /// Waits for every link lookup and document read started so far, whose results are then posted.
    #[cfg(test)]
    pub(crate) fn join_link_work(&mut self) {
        for handle in std::mem::take(&mut self.link_lookups) {
            handle.join().expect("a link worker");
        }
    }
}

/// The first candidate on disk, tried in order (upstream's `exists` over `LinkPolicy.locations`).
fn first_existing(candidates: Vec<LinkLocation>) -> Option<LinkLocation> {
    candidates
        .into_iter()
        .find(|candidate| Path::new(&candidate.path).try_exists().is_ok_and(|exists| exists))
}
