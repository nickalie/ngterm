//! A pick and an ask an origin binds to a session the phone attaches, against the real machine: a pick reaches the
//! phone with its items and its answer reaches `ngtermctl`; one the origin's `--timeout` ends, or one a lost stream
//! takes, is dismissed on the phone, and a headless origin hands a lost one to the phone again once it is back; what
//! the origin held for a session no viewer presented (an ask, a pick and a program overlay) comes as one batch when
//! the phone attaches it, the ask and the pick shown and the overlay refused; a detach leaves an open pick
//! unanswered, held for the next presenter.

use std::time::Duration;

use ngterm_mobile::{AskReply, PickReply, RemoteAsk, RemotePick};
use serde_json::Value;

use crate::machine::World;
use crate::phone::{Client, MACHINE_ID, block_on, poll};
use crate::steps::find_session;

/// The items each pick offers: a JSON item array with a subtitle and a status.
const ITEMS: &[u8] = br#"[{"id":"alpha","label":"Alpha","subtitle":"first","status":"active"},
{"id":"beta","label":"Beta","status":"blocked"}]"#;

/// How long the steps below listen for events that must not come.
const QUIET: Duration = Duration::from_secs(2);

/// How long a result may stay `pending` before the steps below take it as so.
const SETTLE: Duration = Duration::from_secs(10);

/// The steps, in order, on the session `session` the phone presents.
pub(crate) fn picks_and_held_interactions(world: &World, phone: &mut Client, session: &str) {
    an_answered_pick_reaches_ngtermctl(world, phone, session);
    the_origins_timeout_dismisses_a_pick_and_an_ask(world, phone, session);
    a_lost_stream_dismisses_a_pick_and_the_origin_hands_it_over_again(world, phone, session);
    let held = what_the_origin_held_comes_in_one_batch(world, phone);
    a_detach_leaves_an_open_pick_unanswered(world, phone, &held);
}

fn an_answered_pick_reaches_ngtermctl(world: &World, phone: &mut Client, session: &str) {
    let id = open_pick(
        world,
        session,
        &["--prompt", "QaPick", "--select", "beta", "--query", "a"],
    );
    let pick = wait_pick(phone, session, &id, 1);
    assert_eq!(pick.prompt.as_deref(), Some("QaPick"));
    assert_eq!(pick.query.as_deref(), Some("a"));
    assert_eq!(pick.selection.as_deref(), Some("beta"));
    assert!(!pick.allow_custom);
    let items: Vec<(&str, Option<&str>, Option<&str>)> = pick
        .items
        .iter()
        .map(|item| (item.id.as_str(), item.subtitle.as_deref(), item.status.as_deref()))
        .collect();
    assert_eq!(
        items,
        [
            ("alpha", Some("first"), Some("active")),
            ("beta", None, Some("blocked"))
        ]
    );
    answer_pick(
        phone,
        session,
        &id,
        PickReply::Picked {
            item: "beta".to_owned(),
        },
    );
    let result = pick_result(world, &id);
    assert_eq!(result["result"], "picked", "{result}");
    assert_eq!(result["id"], "beta", "{result}");
}

fn the_origins_timeout_dismisses_a_pick_and_an_ask(world: &World, phone: &mut Client, session: &str) {
    let id = open_pick(world, session, &["--timeout", "2"]);
    wait_pick(phone, session, &id, 1);
    phone.wait("the timed-out pick dismissed", |seen| {
        seen.pick_dismissals.contains(&(session.to_owned(), id.clone()))
    });
    assert_eq!(pick_result(world, &id)["result"], "cancelled");

    let ask = open_ask(world, session, &["--timeout", "2"]);
    wait_ask(phone, session, &ask);
    phone.wait("the timed-out ask dismissed", |seen| {
        seen.ask_dismissals.contains(&(session.to_owned(), ask.clone()))
    });
    assert_eq!(ask_result(world, &ask)["result"], "cancelled");
}

fn a_lost_stream_dismisses_a_pick_and_the_origin_hands_it_over_again(world: &World, phone: &mut Client, session: &str) {
    let id = open_pick(world, session, &["--allow-custom"]);
    wait_pick(phone, session, &id, 1);
    world.proxy.sever();
    phone.wait("the pick dismissed with its stream", |seen| {
        seen.pick_dismissals.contains(&(session.to_owned(), id.clone()))
    });
    assert_eq!(
        pick_result(world, &id)["result"],
        "pending",
        "the lost stream answered it"
    );
    // the headless origin held it for the next presenter: the same phone, back
    let again = wait_pick(phone, session, &id, 2);
    assert!(again.allow_custom);
    answer_pick(
        phone,
        session,
        &id,
        PickReply::Custom {
            query: "qa custom".to_owned(),
        },
    );
    let result = pick_result(world, &id);
    assert_eq!(result["result"], "custom", "{result}");
    assert_eq!(result["query"], "qa custom", "{result}");
}

/// Answers the session it made, which the phone now presents.
fn what_the_origin_held_comes_in_one_batch(world: &World, phone: &mut Client) -> String {
    let session = world
        .headless
        .ctl(&["session", "new", "--no-select", "--name", "qa-held"])
        .expect("a session no viewer presents")
        .trim()
        .to_owned();
    let ask = open_ask(world, &session, &[]);
    let pick = open_pick(world, &session, &["--prompt", "QaHeld"]);
    world
        .headless
        .ctl(&["session", "overlay", "open", "true", "--target", &session])
        .expect("a held program overlay");
    let before = kinds(world, &session);
    assert!(
        ["ask", "pick", "job"].iter().all(|kind| before
            .iter()
            .any(|(held, state)| held == kind && state == "unpresented")),
        "held before the phone attached: {before:?}"
    );

    phone.drain();
    // the phone attaches what its last listing holds
    assert!(
        phone.list().iter().any(|(id, _)| *id == session),
        "qa-held is not listed"
    );
    block_on(phone.core.attach_session(MACHINE_ID.to_owned(), session.clone())).expect("attach the held session");
    wait_ask(phone, &session, &ask);
    wait_pick(phone, &session, &pick, 1);
    // the program overlay is refused, and ends on the origin; the ask and the pick are offered
    let after = poll("the refusals on the origin", || {
        let after = kinds(world, &session);
        after
            .iter()
            .all(|(kind, _)| kind == "ask" || kind == "pick")
            .then_some(after)
    });
    assert_eq!(
        after,
        [
            ("ask".to_owned(), "offered".to_owned()),
            ("pick".to_owned(), "offered".to_owned())
        ]
    );

    phone
        .core
        .answer_ask(
            MACHINE_ID.to_owned(),
            session.clone(),
            ask.clone(),
            AskReply {
                button: Some("qaok".to_owned()),
                text: None,
                selected: None,
            },
        )
        .expect("answer the ask");
    let answered = ask_result(world, &ask);
    assert_eq!(answered["result"], "answered", "{answered}");
    assert_eq!(answered["id"], "qaok", "{answered}");
    answer_pick(phone, &session, &pick, PickReply::Cancelled);
    assert_eq!(pick_result(world, &pick)["result"], "cancelled");
    session
}

fn a_detach_leaves_an_open_pick_unanswered(world: &World, phone: &mut Client, session: &str) {
    let id = open_pick(world, session, &["--prompt", "QaDetach"]);
    wait_pick(phone, session, &id, 1);
    phone
        .core
        .detach_session(MACHINE_ID.to_owned(), session.to_owned())
        .expect("detach the session");
    let back = poll("the pick held again", || {
        let kinds = kinds(world, session);
        kinds
            .contains(&("pick".to_owned(), "unpresented".to_owned()))
            .then_some(kinds)
    });
    assert_eq!(
        pick_result(world, &id)["result"],
        "pending",
        "the detach answered it: {back:?}"
    );
    // an answer after the detach goes nowhere
    answer_pick(
        phone,
        session,
        &id,
        PickReply::Picked {
            item: "alpha".to_owned(),
        },
    );
    phone.settle(QUIET);
    assert_eq!(pick_result(world, &id)["result"], "pending");
    // the session attached again: the origin hands the pick over again, and its answer reaches ngtermctl
    block_on(phone.core.attach_session(MACHINE_ID.to_owned(), session.to_owned())).expect("attach it again");
    wait_pick(phone, session, &id, 2);
    answer_pick(
        phone,
        session,
        &id,
        PickReply::Picked {
            item: "beta".to_owned(),
        },
    );
    let result = pick_result(world, &id);
    assert_eq!(result["result"], "picked", "the answer after a reattach: {result}");
    assert_eq!(result["id"], "beta", "{result}");
}

fn open_pick(world: &World, session: &str, arguments: &[&str]) -> String {
    let mut all = vec!["pick", "--no-block", "--session", session];
    all.extend_from_slice(arguments);
    opened_id(&world.headless.ctl_fed(&all, Some(ITEMS)).expect("open a pick"))
}

fn open_ask(world: &World, session: &str, arguments: &[&str]) -> String {
    let mut all = vec![
        "ask",
        "QaAsk",
        "--no-block",
        "--button",
        "qaok=Proceed",
        "--target",
        session,
    ];
    all.extend_from_slice(arguments);
    opened_id(&world.headless.ctl(&all).expect("open an ask"))
}

fn opened_id(output: &str) -> String {
    let opened: Value = serde_json::from_str(output.trim()).unwrap_or_else(|_| panic!("not an opened id: {output}"));
    opened["id"]
        .as_str()
        .unwrap_or_else(|| panic!("no id: {output}"))
        .to_owned()
}

/// The pick `id` of `session`, once the phone was handed it the `count`th time.
fn wait_pick(phone: &mut Client, session: &str, id: &str, count: usize) -> RemotePick {
    let handed = |seen: &crate::phone::Seen| {
        seen.picks
            .iter()
            .filter(|(at, pick)| at == session && pick.id == id)
            .count()
    };
    phone.wait("the pick handed over", |seen| handed(seen) >= count);
    phone
        .seen
        .picks
        .iter()
        .rev()
        .find(|(at, pick)| at == session && pick.id == id)
        .map(|(_, pick)| pick.clone())
        .expect("the pick")
}

fn wait_ask(phone: &mut Client, session: &str, id: &str) -> RemoteAsk {
    phone.wait("the ask handed over", |seen| {
        seen.asks.iter().any(|(at, ask)| at == session && ask.id == id)
    });
    let (_, ask) = phone
        .seen
        .asks
        .iter()
        .rev()
        .find(|(at, ask)| at == session && ask.id == id)
        .expect("the ask");
    assert_eq!(ask.title, "QaAsk");
    ask.clone()
}

fn answer_pick(phone: &Client, session: &str, id: &str, reply: PickReply) {
    phone
        .core
        .answer_pick(MACHINE_ID.to_owned(), session.to_owned(), id.to_owned(), reply)
        .expect("answer the pick");
}

/// The pick's result once it is not pending, or `pending` when it stays so for a short while.
fn pick_result(world: &World, id: &str) -> Value {
    settled_result(world, "pick", id)
}

fn ask_result(world: &World, id: &str) -> Value {
    settled_result(world, "ask", id)
}

/// `ngtermctl <family> result <id>`'s outcome, polled until it is not pending; `pending` after [`SETTLE`].
fn settled_result(world: &World, family: &str, id: &str) -> Value {
    let deadline = std::time::Instant::now() + SETTLE;
    loop {
        let output = world
            .headless
            .ctl(&[family, "result", id])
            .unwrap_or_else(|failed| failed);
        let result: Value = serde_json::from_str(output.trim()).unwrap_or(Value::Null);
        if result["result"] != "pending" || std::time::Instant::now() > deadline {
            return result;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// The kinds and states of what `session` holds, from the origin's tree.
fn kinds(world: &World, session: &str) -> Vec<(String, String)> {
    let output = world.headless.ctl(&["tree", "--json"]).expect("the tree");
    let tree: Value = serde_json::from_str(&output).expect("the tree's json");
    let mut node = None;
    find_session(&tree["result"], session, &mut node);
    let node = node.unwrap_or_else(|| panic!("no node {session} in {tree}"));
    node["interactions"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|entry| {
            (
                entry["kind"].as_str().unwrap_or_default().to_owned(),
                entry["state"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}
