//! Companion command lines and refusals before a page row can be shown.

use ngterm_proto::AppIdentity;
use serde_json::json;

use super::*;
use crate::html::{HtmlHost, HtmlNavigation};
use crate::model::{NewSession, WindowLibrary};
use crate::{CoreConfig, CoreState};

fn core() -> (Core, Uuid, Uuid) {
    let mut library = WindowLibrary::new("/tmp");
    let window = library.new_window(None);
    let store = library.store_mut(window).unwrap();
    let workspace = store.workspaces()[0].id();
    let row = store.add_session(workspace, NewSession::at_cwd("/tmp")).unwrap();
    let core = Core::new(
        CoreConfig {
            app: AppIdentity::from_recorded("0.1.0", None),
            capabilities: Vec::new(),
        },
        CoreState {
            library,
            settings: crate::persist::AppSettings::default(),
            persistence: None,
        },
    );
    (core, window, row)
}

fn result(status: i32, stdout: &str, stderr: &str) -> RemoteCommandResult {
    RemoteCommandResult {
        status,
        stdout: stdout.to_owned(),
        stderr: stderr.to_owned(),
    }
}

fn page() -> PresentationPage {
    serde_json::from_value(json!({
        "page": uuid_string(Uuid::new_v4()), "token": "tok-1", "identity": "report.html",
        "source": {"file": {"path": "/srv/report.html"}}, "chromeless": false,
        "javascript": true, "navigation": false, "browse": false, "persistent": false, "follow": false
    }))
    .unwrap()
}

#[test]
fn terminal_options_reach_the_origin_as_separate_arguments() {
    let request = RemoteCompanionRequest::Terminal(CompanionOptions {
        cwd: Some("/repo with spaces".to_owned()),
        command: Some("printf 'hello'".to_owned()),
        name: Some("build logs".to_owned()),
        wait: true,
        ..CompanionOptions::default()
    });
    assert_eq!(
        request.arguments("parent"),
        [
            "session",
            "new",
            "--parent",
            "parent",
            "--no-select",
            "--cwd",
            "/repo with spaces",
            "--command",
            "printf 'hello'",
            "--wait",
            "--name",
            "build logs"
        ]
    );
}

#[test]
fn page_options_keep_the_origins_grant_and_every_requested_flag() {
    let page = RemotePage {
        source: HtmlSource::File {
            path: "/srv/a.html".to_owned(),
            grant_root: Some("/srv".to_owned()),
        },
        flags: HtmlPageFlags {
            navigation: true,
            javascript: true,
            chromeless: true,
            persistent: true,
            browse: true,
        },
        background_color: Some("#102030".to_owned()),
    };
    assert_eq!(
        page_arguments(&page, "parent"),
        [
            "session",
            "overlay",
            "open",
            "--companion",
            "--target",
            "parent",
            "--html",
            "/srv/a.html",
            "--cwd",
            "/srv",
            "--navigation",
            "--js",
            "--chromeless",
            "--persistent",
            "--browse",
            "--background-color",
            "#102030"
        ]
    );
}

#[test]
fn a_page_handover_reports_transport_and_receipt_failures_exactly() {
    let cases = [
        (
            result(64, "", "Unknown option '--target'"),
            "the ngterm on studio is too old for page companions",
        ),
        (result(255, "", "  ssh: connection lost\n"), "ssh: connection lost"),
        (result(0, "not json", " \n"), "studio gave no answer"),
        (
            result(0, r#"{"ok":true,"result":{}}"#, ""),
            "studio did not hand the page over",
        ),
        (result(0, r#"{"ok":false,"error":"page ended"}"#, ""), "page ended"),
        // upstream decodes the page with the whole reply, so a page that won't decode is no answer at all
        (
            result(0, r#"{"ok":true,"result":{"companionPage":{}}}"#, ""),
            "studio gave no answer",
        ),
        (
            result(
                0,
                r#"{"ok":false,"error":"page ended","result":{"companionPage":{}}}"#,
                " ssh: mux closed\n",
            ),
            "ssh: mux closed",
        ),
        (
            result(0, r#"{"ok":true,"result":{"companionPage":null}}"#, ""),
            "studio did not hand the page over",
        ),
        (
            result(
                0,
                r#"{"ok":false,"error":"invalid request: session.companion.page.attach"}"#,
                "",
            ),
            "the ngterm on studio is too old for page companions",
        ),
    ];
    for (receipt, expected) in cases {
        assert_eq!(handed_page(&receipt, "studio").unwrap_err(), expected);
    }
}

struct Host;

impl HtmlHost for Host {
    fn persistent_store_failure(&self) -> Option<String> {
        None
    }

    fn navigate(&self, _page: Uuid, _navigation: HtmlNavigation) -> Result<(), String> {
        Ok(())
    }
}

#[test]
fn an_unusable_page_handover_never_adds_a_companion_row() {
    let (mut core, window, parent) = core();
    core.set_html_host(Box::new(Host));
    let attach = PageAttach {
        origin_session: "remote-page".to_owned(),
        host: "studio".to_owned(),
        parent,
        select: false,
        reuse_shown: false,
        answer: Answer::default(),
    };
    let before = core.library().store(window).unwrap().sessions().count();
    let mut bad = page();
    bad.page = "invalid".to_owned();
    assert_eq!(
        core.show_companion_page(&attach, &bad).unwrap_err(),
        "the page cannot be shown here"
    );
    bad = page();
    bad.source = PresentationPageSource::Url {
        address: "file:///etc/passwd".to_owned(),
    };
    assert_eq!(
        core.show_companion_page(&attach, &bad).unwrap_err(),
        "the page cannot be shown here"
    );
    bad = page();
    bad.token = "bad token".to_owned();
    assert_eq!(
        core.show_companion_page(&attach, &bad).unwrap_err(),
        "the page cannot be reached on studio"
    );
    assert_eq!(core.library().store(window).unwrap().sessions().count(), before);
}

#[test]
fn a_gui_companion_for_a_missing_row_posts_a_failure_banner() {
    let (mut core, window, _) = core();
    let missing = Uuid::new_v4();
    assert!(core.begin_gui_remote_companion(RemoteCompanionRequest::Duplicate, window, missing, "New Companion"));
    let banners = core.take_notifications();
    assert_eq!(banners.len(), 1);
    assert!(matches!(&banners[0], crate::NotificationEffect::App { title, body, .. }
        if title == "Command failed" && body == "New Companion (the remote session is not attached)"));
    assert!(core.remote.viewer.companion_flows.is_empty());
}
