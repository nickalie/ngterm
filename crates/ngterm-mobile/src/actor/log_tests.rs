//! What the core's paths write to the diagnostic log ([`crate::applog`]), through the whole core over the fake
//! transport: a connect, its failure, a disconnect, suspend and resume; a listing with the CLI asked and what
//! answered; attaching, showing a pane, its role and Take control. Each test puts [`MARKER`] wherever private text
//! could be (a failure's wording, what the machine printed) and checks the log never holds it.

use std::sync::Arc;

use futures_lite::future::block_on;

use super::pane_tests::{Fixture, MACHINE, PANE, SESSION, lead_of, listing, role_title, send};
use crate::api::{EventSink, Machine, MobileCore, MobileError, MobileEvent};
use crate::applog::tests::{Captured, MARKER};
use crate::pane::PaneRole;
use crate::transport::fake::{FakeTransport, Script};

struct Unheard;

impl EventSink for Unheard {
    fn on_event(&self, _: MobileEvent) {}
}

fn machine(id: &str, label: &str, host: &str) -> Machine {
    Machine {
        id: id.to_owned(),
        label: label.to_owned(),
        host: host.to_owned(),
        port: 2222,
        user: "nick".to_owned(),
        key_id: "key".to_owned(),
        connect_timeout_secs: None,
    }
}

fn assert_lines(text: &str, wanted: &[&str]) {
    for line in wanted {
        assert!(text.contains(line), "no line {line:?} in:\n{text}");
    }
}

#[test]
fn a_machines_connection_is_logged_as_the_user_sees_it_without_a_failures_wording() {
    let captured = Captured::new();
    captured.run(|| {
        let transport = FakeTransport::new();
        transport.refuse("down.example", &format!("no route to {MARKER}"));
        let core = MobileCore::with_transport(transport.clone(), Arc::new(Unheard)).unwrap();

        assert_eq!(block_on(core.connect(machine("m1", "My Box", "box.example"))), Ok(()));
        let failed = block_on(core.connect(machine("m2", "Other", "down.example")));
        assert!(matches!(failed, Err(MobileError::Unreachable { .. })), "{failed:?}");
        transport.drop_connections("m1");
        captured.text_with("machine reconnecting machine=m1");
        core.network_changed().unwrap();
        captured.text_with("network changed");
        core.suspend().unwrap();
        core.resume().unwrap();
        core.disconnect("m1".to_owned()).unwrap();
        captured.text_with("disconnect asked");
    });

    assert_lines(
        &captured.text(),
        &[
            "I link: connect asked machine=m1 label=My Box user=nick host=box.example port=2222",
            "I link: machine connected machine=m1",
            "I link: connect asked machine=m2 label=Other user=nick host=down.example port=2222",
            "W link: connect failed machine=m2 class=unreachable reconnect=false",
            "W link: machine unreachable machine=m2",
            "W link: connection lost machine=m1",
            "W link: machine reconnecting machine=m1 next_in_ms=1000",
            "I link: network changed retried=1",
            "I link: suspend connected=",
            "I link: resume machines=1",
            "I link: disconnect asked machine=m1",
        ],
    );
    captured.assert_clean();
}

#[test]
fn a_listing_logs_the_cli_asked_and_what_answered_never_what_the_machine_printed() {
    let captured = Captured::new();
    captured.run(|| {
        let listings = vec![
            listing(true),
            Script::failing(3, &format!("{MARKER}: command not found")),
            listing(false),
            Script::ok(&format!("not json {MARKER}")),
        ];
        let fixture = Fixture::connected_with(listings);
        assert!(block_on(fixture.core.list_sessions(MACHINE.to_owned())).is_err());
        assert_eq!(block_on(fixture.core.list_sessions(MACHINE.to_owned())), Ok(Vec::new()));
        assert!(block_on(fixture.core.list_sessions(MACHINE.to_owned())).is_err());
    });

    let text = captured.text();
    assert_lines(
        &text,
        &[
            "I sessions: listing asked cli=ngtermctl-and-agtermctl shell=posix",
            "I sessions: listing answered answer=sessions peer=ngterm beside=none skipped=none sessions=1 pickable=1 host=box",
            "I sessions: session list reported machine=m1 sessions=1",
            "W sessions: listing failed class=exit-status status=3 stdout_bytes=0 stderr_bytes=32 host=box",
            "I sessions: listing answered answer=empty peer=ngterm beside=none skipped=none sessions=0 pickable=0 host=box",
            "W sessions: listing failed class=unreadable-answer status=0 stdout_bytes=22 stderr_bytes=0 host=box",
        ],
    );
    assert_eq!(
        text.matches("listing asked").count(),
        4,
        "one line per listing:\n{text}"
    );
    captured.assert_clean();
}

#[test]
fn attaching_showing_a_pane_its_role_and_take_control_are_logged() {
    let captured = Captured::new();
    captured.run(|| {
        let mut fixture = Fixture::connected();
        let core = Arc::clone(&fixture.core);
        block_on(core.attach_session(MACHINE.to_owned(), SESSION.to_owned())).unwrap();
        assert!(block_on(core.attach_session(MACHINE.to_owned(), "NOPE".to_owned())).is_err());
        let _pane = fixture.show();
        let attach = fixture.attach();
        fixture.attached();
        send(
            &attach.peer,
            &role_title(&lead_of(&attach.command).nonce, "follower", 1),
        );
        while fixture.lead().0 != Some(PaneRole::Follower) {}
        core.take_lead(MACHINE.to_owned(), SESSION.to_owned(), PANE.to_owned())
            .unwrap();
        core.hide_pane(MACHINE.to_owned(), SESSION.to_owned(), PANE.to_owned())
            .unwrap();
        core.detach_session(MACHINE.to_owned(), SESSION.to_owned()).unwrap();
        captured.text_with("detach asked");
    });

    assert_lines(
        &captured.text(),
        &[
            &format!("I stream: attach asked result=ok machine=m1 session={SESSION}"),
            "I stream: attach asked result=not-listed machine=m1 session=NOPE",
            &format!("I stream: stream state state=connecting session={SESSION}"),
            &format!("I pane: show asked result=ok cols=80 rows=24 session={SESSION} pane={PANE}"),
            &format!("I pane: attached session={SESSION} pane={PANE}"),
            &format!("I pane: role role=follower covered=true session={SESSION} pane={PANE}"),
            &format!("I pane: take control asked session={SESSION} pane={PANE}"),
            &format!("I pane: hide asked pane={PANE}"),
            &format!("I stream: detach asked machine=m1 session={SESSION}"),
        ],
    );
}
