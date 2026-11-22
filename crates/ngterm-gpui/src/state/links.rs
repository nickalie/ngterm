//! A clicked terminal link carried out (upstream's `LinkOpener`, `ControlServer+TextPage.openTextLink`): the link
//! policy decides the route under the link-open setting and the click's chord ([`ClickIntent`]); a link only the file
//! system can resolve (a `path:line` location, a relative path) is looked up off the main thread, the first existing
//! candidate winning, and one with no candidate on disk keeps the route it has as typed. A reveal chord selects the
//! file a local route names in the file manager after that guarded routing, and leaves a web link to its web route.
//! A local document or text page is read off the main thread before it takes its slot, a clicked log only for the
//! lines its page shows; a refused open reveals it.

use std::path::{Path, PathBuf};

use ngterm_core::html::HtmlSource;
use ngterm_core::link_policy::{self, ClickIntent, ClickOrigin, LinkLocation, Route};
use ngterm_core::persist::settings::LinkOpenMode;
use uuid::Uuid;

use super::{AppState, post};

/// A local document a click shows over its session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Document {
    Image,
    Markdown,
    /// A clicked HTML file, shown with no file grant, bridge or page JavaScript.
    Html,
    /// A text page marked at its line.
    Text(Option<i64>),
    /// A clicked log, shown as a log page following its file's end (upstream's `opensAsLog`).
    Log,
}

impl Document {
    fn source(self, path: &Path) -> HtmlSource {
        let path = path.to_string_lossy().into_owned();
        match self {
            Self::Image => HtmlSource::Image { path },
            Self::Markdown => HtmlSource::Markdown { path },
            Self::Html => HtmlSource::Document { path },
            Self::Text(line) => HtmlSource::Text { path, line },
            Self::Log => HtmlSource::Text { path, line: None },
        }
    }
}

impl AppState {
    /// A link clicked in a terminal with no directory of its own to resolve a relative link against (a HUD, a program
    /// overlay, the quick terminal): [`Self::follow_link_at`].
    pub fn follow_link(&mut self, raw: &str, origin: ClickOrigin, intent: ClickIntent) {
        self.follow_link_at(raw, origin, None, intent, None);
    }

    /// A link clicked in a session's pane or scratch terminal: a detected link (`LinkKind::Url`) resolves a relative
    /// path against the local pane's reported directory, or the scratch's start or reported directory.
    /// An OSC 8 URI stays as written. A remote pane's paths name files on the origin, which opens them for this machine
    /// ([`ngterm_core::remote::link_click`]).
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
        let remote = found.remote_host_for(role).is_some();
        let origin = if scratch {
            ClickOrigin::Scratch(session)
        } else if remote {
            ClickOrigin::RemotePane(session)
        } else {
            ClickOrigin::Pane(session)
        };
        let pane = match role {
            ngterm_core::model::StatusPane::Left => Some(found.pane_identity),
            ngterm_core::model::StatusPane::Right => found.split_pane_identity,
            ngterm_core::model::StatusPane::Scratch => None,
        };
        let peer = found
            .remote_presentation()
            .filter(|remote| {
                (scratch && remote.binding.catalog_owned)
                    || pane.is_some_and(|pane| remote.binding.remote_pane(pane).is_some())
            })
            .and_then(|_| found.remote_peer());
        let id = link_policy::session_link(raw, ngterm_core::remote::peer::RemotePeer::Ngterm)
            .or_else(|| peer.and_then(|peer| link_policy::session_link(raw, peer)));
        if let Some(id) = id {
            if intent != ClickIntent::Reveal && self.core.follow_session_link(id, origin, pane) {
                self.settle();
            }
            return;
        }
        let cwd = (kind == crate::terminal::LinkKind::Url)
            .then(|| {
                // The raw report, so the control-character guard of `locations` still applies; a scratch that has
                // not reported yet uses its start directory.
                let reported = self.link_directories.get(&(session, token.to_owned())).cloned();
                if scratch {
                    reported.or_else(|| Some(found.cwd_for(role).to_owned()))
                } else {
                    reported
                }
            })
            .flatten();
        self.follow_link_at(raw, origin, cwd.as_deref(), intent, pane);
    }

    /// Carries out a clicked link (upstream's `LinkOpener.follow`): the route under the link-open setting, or under
    /// browser mode for the browser chord; the candidates [`link_policy::locations`] offers are stat'ed on a worker.
    /// `pane` is the clicked pane's identity, which a remote pane's forwarded click names.
    fn follow_link_at(
        &mut self,
        raw: &str,
        origin: ClickOrigin,
        cwd: Option<&str>,
        intent: ClickIntent,
        pane: Option<Uuid>,
    ) {
        let mode = if intent == ClickIntent::Browser {
            LinkOpenMode::Browser
        } else {
            self.core.settings().effective_link_open_mode()
        };
        let reveal = intent == ClickIntent::Reveal;
        let candidates = link_policy::locations(raw, origin, cwd);
        if candidates.is_empty() {
            let route = link_policy::route(raw, mode, origin);
            if let Route::ForwardRemote { raw, session } = route {
                let click = ngterm_core::remote::link_click::RemoteLinkClick {
                    raw,
                    session,
                    pane,
                    mode,
                };
                self.core.forward_remote_link_click(&click, reveal);
                self.settle();
                return;
            }
            self.carry_out(raw, route, reveal);
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
        let log = route.opens_as_log();
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
            Route::Document {
                path,
                session,
                size_percent,
            } => self.open_document_link(path, session, size_percent, Document::Html),
            Route::Text {
                path,
                line,
                session,
                size_percent,
            } => {
                // a page the presenting machine is handed stays the static text page it already shows there
                let document = if log && self.core.click_opens_log_here(session) {
                    Document::Log
                } else {
                    Document::Text(line)
                };
                self.open_document_link(path, session, size_percent, document);
            }
            // a remote pane has no candidates to look up, so its click never waits for this
            Route::SelectSession(_) | Route::ForwardRemote { .. } => {}
            Route::Browser(url) => self.frontend.open_urls.push(url),
            Route::Reveal(path) => self.frontend.reveals.push(path),
            Route::Ignore => tracing::debug!(raw, "link click ignored by the link policy"),
        }
    }

    /// Shows a local document over `session` once a worker read it, the slot checked again then; a refused or failed
    /// open reveals the file.
    pub(super) fn open_document_link(
        &mut self,
        path: PathBuf,
        session: Uuid,
        size_percent: Option<i64>,
        document: Document,
    ) {
        if !self.core.can_show_link_overlay(session) {
            self.frontend.reveals.push(path);
            return;
        }
        let poster = self.poster.clone();
        let home = self.core.library().home().to_owned();
        self.spawn_link_work(move || {
            let checked = match document {
                Document::Image => ngterm_app::documents::probe_image(&path).map(Some),
                Document::Markdown => ngterm_app::documents::check_markdown(&path).map(|()| None),
                Document::Html => ngterm_app::documents::check_document(&path).map(|()| None),
                Document::Text(_) => ngterm_app::documents::check_text(&path).map(|()| None),
                Document::Log => {
                    ngterm_app::documents::check_log(&path, ngterm_core::html::text_page::DEFAULT_TAIL).map(|()| None)
                }
            };
            // a clicked Markdown file follows its links in its folder, the grant read with the file
            let links = (document == Document::Markdown && checked.is_ok())
                .then(|| ngterm_core::html::markdown_page::link_grant(&path.to_string_lossy(), &home))
                .flatten();
            post(&poster, move |state: &mut AppState| {
                if state.quitting {
                    return;
                }
                let source = document.source(&path);
                let shown = checked.is_ok_and(|probe| match (document, source) {
                    (Document::Log, HtmlSource::Text { path, .. }) => {
                        state.core.open_log_link(path, session, size_percent)
                    }
                    (Document::Markdown, HtmlSource::Markdown { path }) => {
                        state.core.open_markdown_link(path, session, size_percent, links)
                    }
                    (_, source) => state.core.open_document_link(source, session, size_percent, probe),
                });
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
