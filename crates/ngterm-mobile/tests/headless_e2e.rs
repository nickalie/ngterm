//! The mobile core against a real machine: a staged `ngterm-headless` behind an unprivileged `sshd`, driven through
//! [`MobileCore`] as the app drives it. Ignored in plain runs; `scripts/test-mobile-e2e.sh` builds and stages the
//! binaries and runs it with the stage in `MOBILE_E2E_STAGE` and a scratch directory in `MOBILE_E2E_WORK`.
//!
//! The instance is private and never the user's: `ngterm-headless`, `ngtermctl` and `ngterm-session` run with a cleared
//! environment holding only a private `NGTERM_STATE_DIR` and `HOME` under the scratch directory (so the control socket
//! is `<state>/ngterm.sock` and the live sessions' namespace is the one hashed from that state directory), every
//! `ngtermctl` call names that socket with `--socket`, and the `sshd`'s forced command exports the same before it runs
//! the client's command with the stage's binaries first on `PATH`. The test checks that before anything else: a marker
//! session made through the private socket must be all that `live tree` over ssh lists. On every exit path the
//! sessions are reaped through the private namespace's own session server.
//!
//! Once the phone has read, led and reconnected, a fake agent (`support/fake_agent.py`, a stand-in Claude Code run
//! by the system's `python3`) takes the pane's foreground: it replays Task 1's recorded idle screen, writes a
//! transcript made from the recorded session and reports its `SessionStart` through `ngtermctl agent hook` on the private
//! socket. The phone hears the agent announced, reads its chat, follows, pages back and resumes it after a severed
//! link, sends a prompt while the other client leads, sees an approval, and sees the agent cleared once it exits
//! ([`chat`]).
//!
//! A second test puts the phone in front of a Mac running upstream agterm (`support/agterm.rs`: the conformance
//! suite's stand-in, whose own window leads every pane it lists): the pane follows and is covered, what is typed while
//! it follows never reaches the session, and Take control makes the typed line run.
//!
//! Without `sshd` a test skips with a reason, unless `CI` is set: there it fails.
//!
//! Linux only, as the script is: the support code is Unix (file modes, an unprivileged `sshd`), and the other hosts'
//! CI legs build the crate's tests without it.
#![cfg(target_os = "linux")]

#[cfg(test)]
#[path = "support/agent.rs"]
mod agent;
#[cfg(test)]
#[path = "support/agterm.rs"]
mod agterm;
#[cfg(test)]
#[path = "support/chat.rs"]
mod chat;
#[cfg(test)]
#[path = "support/machine.rs"]
mod machine;
#[cfg(test)]
#[path = "support/phone.rs"]
mod phone;
#[cfg(test)]
#[path = "support/sshd.rs"]
mod sshd;
#[cfg(test)]
#[path = "support/steps.rs"]
mod steps;

use std::path::PathBuf;
use std::time::Duration;

use ngterm_mobile::PaneRole;

use self::agterm::AgtermMac;
use self::chat::Scene;
use self::machine::World;
use self::phone::{Client, GRID, MACHINE_ID, block_on, enter};
use self::steps::{assert_isolated, create_and_read, leadership, reconnect};

/// The `sshd` to run a machine behind; `None` is a skip, marked for the script, and a failure on CI.
fn sshd_or_skip() -> Option<PathBuf> {
    let found = sshd::find_sshd();
    if found.is_none() {
        assert!(
            std::env::var_os("CI").is_none_or(|value| value.is_empty()),
            "CI needs sshd for the mobile e2e test"
        );
        eprintln!("skipped: no sshd on this machine");
        // tells the script it was a skip, not a pass
        if let Some(work) = std::env::var_os(machine::WORK_VARIABLE) {
            let marked = std::fs::write(std::path::Path::new(&work).join("skipped"), "no sshd\n");
            assert!(marked.is_ok(), "mark the skip: {marked:?}");
        }
    }
    found
}

/// The line typed into the agterm pane, and what it prints, which the line itself doesn't hold.
const AGTERM_TYPED: &str = "echo typed-$((6*7))-on-agterm";
const AGTERM_PRINTED: &str = "typed-42-on-agterm";

#[test]
#[ignore = "needs sshd and python3: run scripts/test-mobile-e2e.sh"]
fn typing_reaches_an_agterm_session_once_the_phone_takes_control_from_its_window() {
    let Some(binary) = sshd_or_skip() else {
        return;
    };
    let work = PathBuf::from(std::env::var_os(machine::WORK_VARIABLE).expect("MOBILE_E2E_WORK: run the script"));
    let mac = AgtermMac::start(&binary, &work);
    let mut phone = Client::with_keys("agterm phone", &mac.key.private_key, &mac.host_fingerprint);
    phone.connect(mac.sshd.port());
    let listed = phone.list();
    let [(session, name)] = listed.as_slice() else {
        panic!("agterm lists {listed:?}");
    };
    assert_eq!(name, agterm::SESSION);

    block_on(phone.core.attach_session(MACHINE_ID.to_owned(), session.clone())).expect("attach the session");
    let pane = phone.wait_layout().primary;
    let shown = block_on(
        phone
            .core
            .show_pane(MACHINE_ID.to_owned(), session.clone(), pane.clone(), GRID),
    )
    .expect("show the pane");
    phone.terminal = Some(shown.clone());
    // agterm's own window leads: the pane must say so and be covered, where Take control is offered
    phone.wait("the pane following agterm's window", |seen| {
        seen.role == Some(PaneRole::Follower) && seen.covered
    });
    phone.wait_screen("agterm$");

    // zmx drops what a follower sends
    assert!(shown.type_text(AGTERM_TYPED.to_owned()).expect("type"));
    assert!(shown.key(enter()).expect("press enter"));
    let dropped = phone.screen_shows("typed", Duration::from_secs(2));
    assert!(dropped.is_err(), "a follower's typing reached the agterm session");

    phone.take_lead(session, &pane);
    assert!(!phone.seen.covered, "the pane is still covered after it took the lead");
    let leading = phone.terminal.clone().expect("the pane");
    phone.wait_screen("agterm$");
    assert!(leading.type_text(AGTERM_TYPED.to_owned()).expect("type"));
    assert!(leading.key(enter()).expect("press enter"));
    phone.wait_screen(AGTERM_PRINTED);

    let claims: Vec<bool> = mac
        .zmx_calls()
        .iter()
        .map(|call| call["env"].get("ZMX_MANAGED_CLAIM").is_some())
        .collect();
    assert_eq!(
        claims,
        [false, true],
        "the first attach claims nothing; Take control claims"
    );
}

#[test]
#[ignore = "needs a staged ngterm-headless and sshd: run scripts/test-mobile-e2e.sh"]
fn the_mobile_core_drives_a_real_headless_machine_over_ssh() {
    let Some(binary) = sshd_or_skip() else {
        return;
    };
    let world = World::start(&binary);
    let mut first = Client::new("first", &world);
    first.connect(world.proxy.port());

    let marker = assert_isolated(&world, &first);
    let session = create_and_read(&mut first);
    let pane = first.pane.clone().expect("a shown pane");

    let mut second = Client::new("second", &world);
    second.connect(world.sshd.port());
    leadership(&mut first, &mut second, &session, &pane);

    reconnect(&mut first, &world.proxy);

    Scene {
        world: &world,
        first: &mut first,
        second: &mut second,
        session: &session,
        pane: &pane,
    }
    .agent_chat();

    drop(second);
    block_on(first.core.end_session(MACHINE_ID.to_owned(), session.clone())).expect("end the session");
    let listed = first.list();
    assert!(
        listed.iter().all(|(id, _)| *id != session),
        "the ended session is still listed: {listed:?}"
    );
    assert!(listed.iter().any(|(_, name)| *name == marker), "{listed:?}");
    assert!(world.headless.tree().iter().all(|(id, _)| *id != session));
}
