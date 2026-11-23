//! Keys a viewer typed into an attached session's panes, on the origin (this build's `pane.input`, the remote half of
//! upstream's Badge reset, #960): the origin lists the kind, and clears the session's unseen badge by its own rule
//! for a key in a main or split pane from any viewer of the session, never for the scratch pane or a pane it does
//! not know.

use serde_json::{Value, json};
use uuid::Uuid;

use super::remote::{frame, subscribe_listing};
use super::remote_page::Origin;
use super::wire;
use crate::model::StatusReset;
use crate::remote::hub::{PANE_INPUT_KIND, SUPPORTED_KINDS};

/// A presented split session with `unseen` notifications not seen, under the Badge reset `rule`.
fn origin(rule: StatusReset, unseen: usize) -> Origin {
    let mut origin = Origin::build(true, &SUPPORTED_KINDS, false);
    origin
        .fixture()
        .core
        .update_settings(|settings| settings.badge_reset = Some(rule.raw().to_owned()));
    let session = origin.session();
    origin
        .fixture()
        .store_mut()
        .unwrap()
        .session_mut(session)
        .unwrap()
        .unseen_count = unseen;
    origin
}

fn unseen(origin: &Origin) -> usize {
    origin
        .live
        .fixture
        .store()
        .session(origin.session())
        .unwrap()
        .unseen_count
}

fn panes(origin: &Origin) -> (Uuid, Uuid) {
    let session = origin.live.fixture.store().session(origin.session()).unwrap();
    (session.pane_identity, session.split_pane_identity.unwrap())
}

/// A mirror of the session types `key` into `pane`.
fn typed(origin: &mut Origin, pane: &Value, key: &str) {
    let session = origin.session();
    let core = &mut origin.live.fixture.core;
    let (serial, mirror) = subscribe_listing(core, session, "mirror", &SUPPORTED_KINDS);
    let line = frame(
        &mirror,
        &json!({"kind": PANE_INPUT_KIND, "input": {"pane": pane, "key": key}}),
    );
    core.presentation_line(serial, &line);
    core.commit();
}

#[test]
fn the_origin_lists_pane_input_in_its_hello_answer() {
    let origin = origin(StatusReset::FirstKey, 0);
    let hello = &origin.presenter.frames("hello")[0];
    assert!(
        hello["hello"]["kinds"]
            .as_array()
            .unwrap()
            .contains(&json!(PANE_INPUT_KIND)),
        "{hello}"
    );
}

#[test]
fn a_viewers_first_key_in_the_main_or_split_pane_clears_the_badge() {
    for split in [false, true] {
        let mut origin = origin(StatusReset::FirstKey, 2);
        let (left, right) = panes(&origin);
        let pane = if split { right } else { left };
        typed(&mut origin, &json!(wire(pane)), "other");
        assert_eq!(unseen(&origin), 0, "split: {split}");
    }
}

#[test]
fn under_on_enter_only_a_submit_clears_the_badge_and_an_unknown_key_is_plain_typing() {
    let mut origin = origin(StatusReset::Enter, 2);
    let (left, _) = panes(&origin);
    for key in ["other", "interrupt", "tab"] {
        typed(&mut origin, &json!(wire(left)), key);
        assert_eq!(unseen(&origin), 2, "{key}");
    }
    typed(&mut origin, &json!(wire(left)), "submit");
    assert_eq!(unseen(&origin), 0);
}

#[test]
fn disabled_badge_reset_a_scratch_key_or_an_unknown_pane_keeps_the_badge() {
    let mut disabled = origin(StatusReset::Never, 2);
    let (left, _) = panes(&disabled);
    typed(&mut disabled, &json!(wire(left)), "submit");
    assert_eq!(unseen(&disabled), 2);

    let mut origin = origin(StatusReset::FirstKey, 2);
    typed(&mut origin, &json!("scratch"), "submit");
    typed(&mut origin, &json!(wire(Uuid::new_v4())), "submit");
    assert_eq!(unseen(&origin), 2);
}
