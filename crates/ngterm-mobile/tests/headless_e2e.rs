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
//! Before that, in the pane the phone leads ([`links`]): a key it types clears the badge a notification raised on the
//! origin, in the main pane and in the split pane, a program's color scheme query gets the phone's appearance and a
//! mode 2031 program its flip, and a session link names the listed session it links to and nothing for any other link.
//!
//! Once the phone has read, led and reconnected, a fake agent (`support/fake_agent.py`, a stand-in Claude Code run
//! by the system's `python3`) takes the pane's foreground: it replays Task 1's recorded idle screen, writes a
//! transcript made from the recorded session and reports its `SessionStart` through `ngtermctl agent hook` on the private
//! socket. The phone hears the agent announced, reads its chat, follows, pages back and resumes it after a severed
//! link, sends a prompt while the other client leads, sees an approval, and sees the agent cleared once it exits
//! ([`chat`]). On the way the phone's lone Left arrow is held back from the agent's empty prompt, where Claude Code
//! would move the conversation to the background, and reaches it everywhere else ([`left_arrow`]). Then
//! `ngtermctl session restart` on the machine, with a command and without one, replaces the pane's shell: the phone
//! attaches the new shell by itself, with no detach of the session, and types into it ([`steps::restart`]).
//!
//! A second test puts the phone in front of a Mac running upstream agterm (`support/agterm.rs`: the conformance
//! suite's stand-in, whose own window leads every pane it lists): the pane follows and is covered, what is typed while
//! it follows never reaches the session, and Take control makes the typed line run.
//!
//! A third test has one machine that runs both apps (`support/both.rs`: a private `ngterm-headless` with the stand-in
//! agterm beside it, in one login). The phone lists the sessions of both, each with the app that listed it; New
//! Session goes to ngterm; an agterm session is typed into and ended through agterm, and has no chat. A second
//! server of the user's, another private `ngterm-headless`, lists the machine with `ngtermctl live tree HOST` and
//! attaches a session of each app with `ngtermctl live attach HOST ID`, each through its own app. With agterm's
//! sessions gone and agterm outside Live sessions mode, both say why it lists nothing.
//!
//! A fourth test binds picks and asks to a session the phone presents ([`picks`]): each reaches the phone and its
//! answer reaches `ngtermctl`, the origin's `--timeout` and a lost stream take one down, the headless origin hands a
//! lost pick over again, what it held for a session no viewer presented comes as one batch (the ask and the pick
//! shown, a program overlay and a page refused), and a detach leaves an open pick unanswered.
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
#[path = "support/both.rs"]
mod both;
#[cfg(test)]
#[path = "support/chat.rs"]
mod chat;
#[cfg(test)]
#[path = "support/left_arrow.rs"]
mod left_arrow;
#[cfg(test)]
#[path = "support/links.rs"]
mod links;
#[cfg(test)]
#[path = "support/machine.rs"]
mod machine;
#[cfg(test)]
#[path = "support/phone.rs"]
mod phone;
#[cfg(test)]
#[path = "support/picks.rs"]
mod picks;
#[cfg(test)]
#[path = "support/sshd.rs"]
mod sshd;
#[cfg(test)]
#[path = "support/steps.rs"]
mod steps;

use std::path::PathBuf;

use ngterm_mobile::PeerKind;

use self::agterm::AgtermMac;
use self::both::{
    AGTERM_SESSION, BothHost, HOST, Viewer, agterm_outside_live_sessions_mode_is_said_by_both, named,
    the_viewer_attaches_a_session_of_each_app, the_viewer_lists_both_apps,
};
use self::chat::Scene;
use self::machine::World;
use self::phone::{Client, MACHINE_ID, block_on, poll};
use self::steps::{
    assert_isolated, create_and_read, leadership, reconnect, restart, type_into_agterm_after_taking_control,
};

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

    type_into_agterm_after_taking_control(&mut phone, session);

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
    let _alone = machine::one_at_a_time();
    let world = World::start(&binary);
    let mut first = Client::new("first", &world);
    first.connect(world.proxy.port());

    let marker = assert_isolated(&world, &first);
    let session = create_and_read(&mut first);
    let pane = first.pane.clone().expect("a shown pane");
    links::typing_clears_the_origins_badge(&world, &mut first, &session);
    links::typing_in_the_split_pane_clears_the_origins_badge(&world, &mut first, &session);
    links::the_program_gets_the_apps_appearance(&mut first);
    links::a_session_link_names_its_session(&world, &mut first);
    // the steps below look for the line's output, which the links pushed off the screen
    first.run_line();

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
    restart(&world, &mut first, &session, &pane);
    block_on(first.core.end_session(MACHINE_ID.to_owned(), session.clone())).expect("end the session");
    let listed = first.list();
    assert!(
        listed.iter().all(|(id, _)| *id != session),
        "the ended session is still listed: {listed:?}"
    );
    assert!(listed.iter().any(|(_, name)| *name == marker), "{listed:?}");
    assert!(world.headless.tree().iter().all(|(id, _)| *id != session));
}

#[test]
#[ignore = "needs a staged ngterm-headless, sshd and python3: run scripts/test-mobile-e2e.sh"]
fn a_machine_running_both_apps_lists_the_sessions_of_both_and_each_works_through_its_app() {
    let Some(binary) = sshd_or_skip() else {
        return;
    };
    let _alone = machine::one_at_a_time();
    let host = BothHost::start(&binary);
    let mut phone = Client::with_keys("both phone", &host.key.private_key, &host.host_key.fingerprint);
    phone.connect(host.sshd.port());

    // the listing: ngterm's own first session, then agterm's, each with its app
    let listed = poll("the sessions of both apps", || {
        let listed = phone.sessions();
        (listed.len() >= 2).then_some(listed)
    });
    let agterm = named(&listed, AGTERM_SESSION).clone();
    assert_eq!(agterm.app, Some(PeerKind::Agterm), "{listed:?}");
    assert_eq!(listed.last(), Some(&agterm), "agterm's sessions follow ngterm's");
    let own = &listed[..listed.len() - 1];
    assert!(
        own.iter().all(|session| session.app == Some(PeerKind::Ngterm)),
        "{listed:?}"
    );
    assert_eq!(own.len(), host.headless.tree().len(), "{listed:?}");

    // New Session on such a machine is ngterm's, and works as on an ngterm machine; agterm's stays listed beside it
    let created = create_and_read(&mut phone);
    let listed = phone.sessions();
    let ngterm = listed
        .iter()
        .find(|session| session.id == created)
        .unwrap_or_else(|| panic!("the new session is not listed: {listed:?}"))
        .clone();
    assert_eq!(ngterm.app, Some(PeerKind::Ngterm));
    assert_eq!(named(&listed, AGTERM_SESSION), &agterm);
    assert_eq!(
        host.agterm.sessions(),
        [AGTERM_SESSION],
        "agterm got a session it never made"
    );

    // another server of the user's lists the machine and attaches a session of each app
    let viewer = Viewer::start(&host);
    the_viewer_lists_both_apps(&viewer, &ngterm, &agterm);
    the_viewer_attaches_a_session_of_each_app(&viewer, &host, &ngterm, &agterm);

    // the agterm session on a phone: no chat, typed into once control is taken, ended through agterm
    let mut second = Client::with_keys("both phone 2", &host.key.private_key, &host.host_key.fingerprint);
    second.connect(host.sshd.port());
    assert_eq!(named(&second.sessions(), AGTERM_SESSION), &agterm);
    type_into_agterm_after_taking_control(&mut second, &agterm.id);
    let pane = second.wait_layout().primary;
    let chat = block_on(second.core.open_chat(MACHINE_ID.to_owned(), agterm.id.clone(), pane));
    assert!(chat.is_err(), "a chat opened on an agterm session");
    block_on(second.core.end_session(MACHINE_ID.to_owned(), agterm.id.clone())).expect("end agterm's session");
    assert!(host.agterm.sessions().is_empty(), "agterm still has the ended session");
    let listed = second.sessions();
    assert!(
        listed.iter().all(|session| session.app == Some(PeerKind::Ngterm)),
        "{listed:?}"
    );
    assert!(listed.contains(&ngterm), "{listed:?}");
    // a listing of both where agterm has nothing left says so under ngterm's sessions
    let text = viewer
        .headless
        .ctl(&["live", "tree", HOST])
        .expect("the viewer's listing");
    assert!(
        text.contains(&format!("no attachable sessions on {HOST} (agterm)")) && text.contains(&ngterm.id),
        "{text}"
    );
    assert!(
        !text.contains("Live sessions mode"),
        "agterm in Live sessions mode got a hint: {text}"
    );

    agterm_outside_live_sessions_mode_is_said_by_both(&viewer, &host, &mut second);
}

#[test]
#[ignore = "needs a staged ngterm-headless and sshd: run scripts/test-mobile-e2e.sh"]
fn picks_and_held_interactions_reach_the_phone_and_end_as_the_origin_ends_them() {
    let Some(binary) = sshd_or_skip() else {
        return;
    };
    let _alone = machine::one_at_a_time();
    let world = World::start(&binary);
    let mut phone = Client::new("pick phone", &world);
    phone.connect(world.proxy.port());
    assert_isolated(&world, &phone);
    let session = create_and_read(&mut phone);
    picks::picks_and_held_interactions(&world, &mut phone, &session);
}
