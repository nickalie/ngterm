//! Local document validation, source projection and natural-size opening (upstream Image/Markdown page cases).

use std::cell::RefCell;
use std::rc::Rc;

use ngterm_proto::Args;
use ngterm_proto::command::SessionCmd;
use serde_json::{Value, json};
use uuid::Uuid;

use super::{Fixture, args, failure, wire};
use crate::html::{HtmlHost, HtmlNavigation, HtmlSource, image_page};

#[derive(Default)]
struct Log {
    images: Vec<String>,
    markdown: Vec<String>,
    failure: Option<String>,
}
struct Host(Rc<RefCell<Log>>);
impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }
    fn navigate(&self, _: Uuid, _: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
    fn probe_image(&self, path: &str) -> Result<image_page::ImageData, String> {
        let mut log = self.0.borrow_mut();
        log.images.push(path.to_owned());
        if let Some(error) = &log.failure {
            return Err(error.clone());
        }
        Ok(image_page::ImageData {
            bytes: vec![],
            mime_type: "image/png".to_owned(),
            width: 1000,
            height: 500,
            orientation: 1,
        })
    }
    fn check_markdown(&self, path: &str) -> Result<(), String> {
        let mut log = self.0.borrow_mut();
        log.markdown.push(path.to_owned());
        log.failure.clone().map_or(Ok(()), Err)
    }
    fn page_zoom(&self) -> f64 {
        1.5
    }
    fn display_scale(&self, _: Uuid) -> f64 {
        2.0
    }
}
fn fixture() -> (Fixture, Rc<RefCell<Log>>) {
    let mut f = Fixture::new(1);
    let log = Rc::new(RefCell::new(Log::default()));
    f.core.set_html_host(Box::new(Host(Rc::clone(&log))));
    for session in f.sessions.clone() {
        f.core.report_hud_frames(
            session,
            &crate::hud::HudPaneFrames {
                detail: Some(crate::hud::HudPaneFrame {
                    width: 2000.0,
                    height: 1400.0,
                    ..crate::hud::HudPaneFrame::default()
                }),
                ..crate::hud::HudPaneFrames::default()
            },
        );
    }
    (f, log)
}
fn path(name: &str) -> String {
    crate::html::os_path(&format!("/tmp/{name}"))
}
fn node(f: &mut Fixture, id: Uuid) -> Value {
    let tree = f.core.tree(None).unwrap();
    serde_json::to_value(tree).unwrap()["workspaces"][0]["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == wire(id))
        .unwrap()
        .clone()
}

#[test]
fn documents_reject_sources_flags_and_paths_before_resolving_a_target() {
    let mut f = Fixture::new(0);
    let image = path("x.png");
    let markdown = path("x.md");
    let cases = [
        (
            json!({"image": image, "html": path("x.html")}),
            "session.overlay.open takes one of a command, --html, --url, --image, --markdown or --text",
        ),
        (
            json!({"image": image, "url": "https://example.com/"}),
            "session.overlay.open takes one of a command, --html, --url, --image, --markdown or --text",
        ),
        (
            json!({"image": image, "markdownFile": markdown}),
            "session.overlay.open takes one of a command, --html, --url, --image, --markdown or --text",
        ),
        (
            json!({"markdownFile": markdown, "command": "cat"}),
            "session.overlay.open takes one of a command, --html, --url, --image, --markdown or --text",
        ),
        (
            json!({"image": image, "cwd": path(".")}),
            "session.overlay.open: --image takes no --wait, --cwd, --navigation, --js, --chromeless, --persistent or --browse",
        ),
        (
            json!({"markdownFile": markdown, "javascript": true}),
            "session.overlay.open: --markdown takes no --wait, --cwd, --navigation, --js, --chromeless, --persistent or --browse",
        ),
        (
            json!({"image": "x.png"}),
            "session.overlay.open: --image must be an absolute path",
        ),
        (
            json!({"markdownFile": "x.md"}),
            "session.overlay.open: --markdown must be an absolute path",
        ),
        (
            json!({"image": path("x.svg")}),
            "session.overlay.open: --image must be a png, jpg, jpeg, gif or webp file",
        ),
        (
            json!({"markdownFile": path("x.html")}),
            "session.overlay.open: --markdown must be a .md or .markdown file",
        ),
    ];
    for (value, error) in cases {
        let input: Args = serde_json::from_value(value).unwrap();
        for command in [SessionCmd::OverlayOpen, SessionCmd::CompanionPage] {
            assert_eq!(f.session(command, Some("missing"), input.clone()), failure(error));
        }
    }
    for (key, value) in [("image", image), ("markdownFile", markdown)] {
        let input: Args = serde_json::from_value(json!({key: value})).unwrap();
        assert_eq!(
            f.session(SessionCmd::CompanionPage, Some("missing"), input),
            failure("a page companion needs --html or --url")
        );
    }
}

#[test]
fn document_options_refuse_each_true_flag_but_allow_explicit_false() {
    let mut f = Fixture::new(0);
    for (source, name) in [("image", "x.png"), ("markdownFile", "x.md")] {
        for flag in ["wait", "navigation", "javascript", "chromeless", "persistent", "browse"] {
            for value in [true, false] {
                let mut object = json!({source: path(name)});
                object.as_object_mut().unwrap().insert(flag.to_owned(), json!(value));
                let input: Args = serde_json::from_value(object).unwrap();
                let result = crate::commands::overlay::overlay_content("", &input, None);
                assert_eq!(result.is_err(), value, "{source}/{flag}={value}");
                let kind = if source == "image" { "image" } else { "markdown" };
                let error = if value {
                    format!(
                        "session.overlay.open: --{kind} takes no --wait, --cwd, --navigation, --js, --chromeless, --persistent or --browse"
                    )
                } else {
                    "a page companion needs --html or --url".to_owned()
                };
                assert_eq!(
                    f.session(SessionCmd::CompanionPage, Some("missing"), input),
                    failure(&error),
                    "{source}/{flag}={value}"
                );
            }
        }
    }
}

#[test]
fn natural_image_uses_window_scale_and_zoom_and_projects_only_its_source() {
    let (mut f, log) = fixture();
    let id = f.sessions[0];
    let selected = f.sessions[1];
    let response = f.session(
        SessionCmd::OverlayOpen,
        Some(&wire(id)),
        args(|a| a.image = Some(path("x.PNG"))),
    );
    assert!(response.ok, "{response:?}");
    let read = node(&mut f, id);
    assert_eq!(read["overlaySizePercent"], 95);
    assert_eq!(read["overlayWidth"].as_f64(), Some(750.0));
    assert_eq!(read["overlayHeight"].as_f64(), Some(375.0));
    let page = &read["htmlOverlays"][0];
    assert_eq!(page["image"], path("x.PNG"));
    for key in ["file", "url", "markdown", "cwd"] {
        assert!(page.get(key).is_none(), "{key}: {page}");
    }
    assert_eq!(f.store().selected_session_id(), Some(selected));
    assert_eq!(log.borrow().images, vec![path("x.PNG")]);
}

#[test]
fn explicit_image_size_and_markdown_source_stay_explicit() {
    for markdown in [false, true] {
        let (mut f, _) = fixture();
        let id = f.sessions[0];
        let response = f.session(
            SessionCmd::OverlayOpen,
            Some(&wire(id)),
            args(|a| {
                if markdown {
                    a.markdown_file = Some(path("doc.markdown"));
                } else {
                    a.image = Some(path("x.gif"));
                }
                a.size_percent = Some(70);
                a.width = Some(321);
            }),
        );
        assert!(response.ok, "{response:?}");
        let read = node(&mut f, id);
        assert_eq!(read["overlaySizePercent"], 70);
        assert_eq!(read["overlayWidth"].as_f64(), Some(321.0));
        if markdown {
            assert_eq!(read["htmlOverlays"][0]["markdown"], path("doc.markdown"));
        }
    }
}

#[test]
fn failed_document_probe_opens_nothing_and_keeps_selection() {
    for markdown in [false, true] {
        let (mut f, log) = fixture();
        let id = f.sessions[0];
        log.borrow_mut().failure = Some("not a regular file".to_owned());
        let response = f.session(
            SessionCmd::OverlayOpen,
            Some(&wire(id)),
            args(|a| {
                if markdown {
                    a.markdown_file = Some(path("x.md"));
                } else {
                    a.image = Some(path("x.webp"));
                }
            }),
        );
        assert_eq!(response, failure("session.overlay.open: not a regular file"));
        assert!(node(&mut f, id).get("htmlOverlays").is_none());
    }
}

#[test]
fn document_sources_are_themed_and_bridged_independently() {
    for markdown in [false, true] {
        let source = if markdown {
            HtmlSource::Markdown { path: path("doc.md") }
        } else {
            HtmlSource::Image { path: path("x.png") }
        };
        let page = crate::html::HtmlOverlay::new(source.clone(), crate::html::HtmlPageFlags::default());
        assert_eq!(page.themed(), markdown);
        assert!(!page.bridged());
        assert!(page.text_loaded());
    }
}

#[test]
fn document_click_refuses_a_busy_slot_without_selecting() {
    let (mut f, _) = fixture();
    let id = f.sessions[0];
    let selected = f.store().selected_session_id();
    let source = HtmlSource::Markdown { path: path("doc.md") };
    assert!(f.core.open_document_link(source.clone(), id, Some(95), None));
    assert!(!f.core.open_document_link(source, id, Some(95), None));
    assert_eq!(f.store().selected_session_id(), selected);
}

#[test]
fn document_click_rechecks_hud_zoom_and_closed_owner() {
    for guard in ["hud", "zoom", "closed"] {
        let (mut f, _) = fixture();
        let id = f.sessions[0];
        let selected = f.store().selected_session_id();
        let bodies = tempfile::tempdir().unwrap();
        match guard {
            "hud" => {
                f.core.set_hud_body_dir(bodies.path().join("bodies"));
                let response = f.session(
                    SessionCmd::HudOpen,
                    Some(&wire(id)),
                    args(|a| a.message = Some("working".to_owned())),
                );
                assert!(response.ok, "{response:?}");
            }
            "zoom" => f.core.set_zoom_target(
                f.window,
                Some(crate::zoom::SurfaceId {
                    session: id,
                    surface: crate::zoom::ZoomSurface::Primary,
                }),
            ),
            "closed" => {
                assert!(f.store_mut().unwrap().close_session(id).is_some());
            }
            _ => unreachable!(),
        }
        assert!(
            !f.core
                .open_document_link(HtmlSource::Markdown { path: path("doc.md") }, id, Some(95), None),
            "{guard}"
        );
        assert_eq!(f.store().selected_session_id(), selected, "{guard}");
    }
}
