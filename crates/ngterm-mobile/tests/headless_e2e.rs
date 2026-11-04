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
//! Without `sshd` the test skips with a reason, unless `CI` is set: there it fails.
//!
//! Linux only, as the script is: the support code is Unix (file modes, an unprivileged `sshd`), and the other hosts'
//! CI legs build the crate's tests without it.
#![cfg(target_os = "linux")]

#[cfg(test)]
#[path = "support/agent.rs"]
mod agent;
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

use self::chat::Scene;
use self::machine::World;
use self::phone::{Client, MACHINE_ID, block_on};
use self::steps::{assert_isolated, create_and_read, leadership, reconnect};

#[test]
#[ignore = "needs a staged ngterm-headless and sshd: run scripts/test-mobile-e2e.sh"]
fn the_mobile_core_drives_a_real_headless_machine_over_ssh() {
    let Some(binary) = sshd::find_sshd() else {
        assert!(
            std::env::var_os("CI").is_none_or(|value| value.is_empty()),
            "CI needs sshd for the mobile e2e test"
        );
        eprintln!("skipped: no sshd on this machine");
        // tells the script it was a skip, not a pass
        if let Some(work) = std::env::var_os(machine::WORK_VARIABLE) {
            std::fs::write(std::path::Path::new(&work).join("skipped"), "no sshd\n").expect("mark the skip");
        }
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
