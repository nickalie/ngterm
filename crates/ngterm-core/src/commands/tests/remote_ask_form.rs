//! Form asks handed to the viewer presenting a session, on the origin (upstream's `AppStoreRemoteAskTests` form cases):
//! the `ask.form` / `ask.other` capability and the frame fit decide whether one goes, and the presenter's answer is
//! applied only with a payload its form would produce.

use ngterm_proto::command::AskCmd;
use ngterm_proto::{Args, AskButton, AskChoice, Command, Response};
use serde_json::{Value, json};

use super::live::Live;
use super::remote::{Viewer, ask_result, frame, node, presented_listing};
use super::{args, wire};
use crate::model::AskDraft;
use crate::remote::frames::MAX_FRAME_BYTES;
use crate::remote::hub::{ASK_FORM_KIND, ASK_OTHER_KIND};

const BOTH: [&str; 2] = [ASK_FORM_KIND, ASK_OTHER_KIND];

fn button(id: &str, label: &str) -> AskButton {
    AskButton {
        id: id.to_owned(),
        label: label.to_owned(),
        hotkey: None,
    }
}

fn choices() -> Vec<AskChoice> {
    ["a", "b"]
        .into_iter()
        .map(|id| AskChoice {
            id: id.to_owned(),
            label: id.to_uppercase(),
            subtitle: None,
        })
        .collect()
}

/// The form of one case, as `ask.open` arguments.
#[derive(Clone, Copy)]
enum Form {
    Line,
    Multiline,
    Radio,
    RadioOther,
    Checkboxes,
    CheckboxesOther,
    Plain,
}

impl Form {
    fn apply(self, a: &mut Args) {
        match self {
            Self::Line => a.input = Some("line".to_owned()),
            Self::Multiline => a.input = Some("multiline".to_owned()),
            Self::Plain => {}
            Self::Radio | Self::RadioOther | Self::Checkboxes | Self::CheckboxesOther => {
                a.choices = Some(choices());
                a.multiple = Some(matches!(self, Self::Checkboxes | Self::CheckboxesOther));
                if matches!(self, Self::RadioOther | Self::CheckboxesOther) {
                    a.other = Some("Other".to_owned());
                }
            }
        }
    }
}

/// Opens an ask titled `t` on the session with buttons yes/no, `no` the cancel role unless `form` is plain, and `edit`
/// applied last.
fn open(live: &mut Live, form: Form, edit: impl FnOnce(&mut Args)) -> Response {
    let session = live.session;
    let response = live.fixture.core.dispatch(&ngterm_proto::Request {
        cmd: Command::Ask(AskCmd::Open),
        target: Some(wire(session)),
        args: Some(args(|a| {
            a.title = Some("t".to_owned());
            a.buttons = Some(vec![button("yes", "Yes"), button("no", "No")]);
            if !matches!(form, Form::Plain) {
                a.cancel_button = Some("no".to_owned());
            }
            form.apply(a);
            edit(a);
        })),
    });
    live.fixture.core.commit();
    response
}

fn opened_id(response: &Response) -> String {
    response.result.as_ref().unwrap_or_else(|| panic!("{response:?}"))["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Whether the ask stayed here: drawn on the session, nothing sent to the presenter.
fn stayed_here(live: &mut Live, presenter: &Viewer, id: &str) -> bool {
    let session = live.session;
    presenter.last("ask.request").is_none() && node(&mut live.fixture, session)["ask"] == json!({"id": id})
}

/// The presenter answers ask `id` with `answer`'s fields beside the id and its owner.
fn resolve(live: &mut Live, serial: u64, presenter: &Viewer, id: &str, mut answer: Value) {
    answer["id"] = json!(id);
    answer["owner"] = json!(1);
    let resolve = frame(presenter, &json!({"kind": "ask.resolve", "ask": answer}));
    live.fixture.core.presentation_line(serial, &resolve);
    live.fixture.core.commit();
}

fn cancel(live: &mut Live, id: &str) {
    let response = live.fixture.core.dispatch(&ngterm_proto::Request {
        cmd: Command::Ask(AskCmd::Cancel),
        target: Some(id.to_owned()),
        args: None,
    });
    assert!(response.ok, "{response:?}");
    live.fixture.core.commit();
}

/// Upstream's `aFormOrCancelRoleAskStaysHereForAPresenterWithoutTheCapability`.
#[test]
fn a_form_or_cancel_role_ask_stays_here_for_a_presenter_without_the_capability() {
    for form in [Form::Line, Form::Radio, Form::Plain] {
        let (mut live, _, presenter) = presented_listing(&[ASK_OTHER_KIND]);
        let id = opened_id(&open(&mut live, form, |a| {
            a.cancel_button = Some("no".to_owned());
        }));
        assert!(stayed_here(&mut live, &presenter, &id));
    }
}

/// Upstream's `aFormAskGoesToAPresenterListingTheCapability`.
#[test]
fn a_form_ask_goes_to_a_presenter_listing_the_capability() {
    let (mut live, _, presenter) = presented_listing(&[ASK_FORM_KIND]);
    let id = opened_id(&open(&mut live, Form::Multiline, |a| a.value = Some("v".to_owned())));
    let request = presenter.last("ask.request").unwrap()["ask"].clone();
    assert_eq!(request["id"], json!(id));
    assert_eq!(request["input"], json!({"mode": "multiline", "value": "v"}));
    assert_eq!(request["cancelID"], "no");
    assert!(request.get("choices").is_none());
    let session = live.session;
    assert_eq!(
        node(&mut live.fixture, session)["ask"],
        json!({"id": id, "remote": true})
    );
}

/// Upstream's `aFormAskWhoseFrameIsTooLargeStaysHere` and
/// `anInputAskStaysHereWhenItsLongestButtonAndTheReservedTextCannotFitAnAnswer`.
#[test]
fn a_form_ask_whose_request_or_answer_cannot_fit_a_frame_stays_here() {
    let (mut live, _, presenter) = presented_listing(&BOTH);
    let id = opened_id(&open(&mut live, Form::Line, |a| {
        a.message = Some("m".repeat(MAX_FRAME_BYTES));
    }));
    assert!(stayed_here(&mut live, &presenter, &id));
    cancel(&mut live, &id);
    let id = opened_id(&open(&mut live, Form::Line, |a| {
        a.buttons = Some(vec![button(&"b".repeat(200 * 1024), "Long")]);
        a.cancel_button = None;
    }));
    assert!(stayed_here(&mut live, &presenter, &id));
    cancel(&mut live, &id);
    // the same button on a list without a free-form row answers with ids alone, so it goes
    open(&mut live, Form::Radio, |a| {
        a.buttons = Some(vec![button(&"b".repeat(200 * 1024), "Long")]);
        a.cancel_button = None;
    });
    assert!(presenter.last("ask.request").is_some());
}

/// Upstream's `anOtherRowAskStaysHereUnlessThePresenterListsBothCapabilities`.
#[test]
fn an_other_row_ask_stays_here_unless_the_presenter_lists_both_capabilities() {
    for kind in BOTH {
        let (mut live, _, presenter) = presented_listing(&[kind]);
        let id = opened_id(&open(&mut live, Form::RadioOther, |_| {}));
        assert!(stayed_here(&mut live, &presenter, &id), "{kind}");
    }
}

/// Upstream's `anOtherRowAskGoesToAPresenterListingBothCapabilities`.
#[test]
fn an_other_row_ask_goes_to_a_presenter_listing_both_capabilities() {
    let (mut live, _, presenter) = presented_listing(&BOTH);
    let id = opened_id(&open(&mut live, Form::RadioOther, |_| {}));
    let request = presenter.last("ask.request").unwrap()["ask"].clone();
    assert_eq!(request["id"], json!(id));
    assert_eq!(
        request["choices"],
        json!({"choices": [{"id": "a", "label": "A"}, {"id": "b", "label": "B"}],
            "multiple": false, "checked": [], "other": "Other"})
    );
    assert_eq!(request["cancelID"], "no");
}

/// Upstream's `aFormAnswerIsAppliedWithItsPayload`.
#[test]
fn a_form_answer_is_applied_with_its_payload() {
    let answered = |extra: Value| {
        let mut result = json!({"result": "answered", "id": "yes", "label": "Yes", "index": 0});
        for (key, value) in extra.as_object().unwrap() {
            result[key] = value.clone();
        }
        result
    };
    let cases = [
        (
            Form::Multiline,
            json!({"button": "yes", "text": "a \"q\"\nb\tc"}),
            answered(json!({"text": "a \"q\"\nb\tc"})),
        ),
        (
            Form::Checkboxes,
            json!({"button": "yes", "selected": ["a", "b"]}),
            answered(json!({"selected": ["a", "b"]})),
        ),
        (
            Form::Checkboxes,
            json!({"button": "yes", "selected": []}),
            answered(json!({"selected": []})),
        ),
        (Form::Line, json!({"button": "no"}), json!({"result": "escaped"})),
        (
            Form::RadioOther,
            json!({"button": "yes", "text": " mine ", "selected": []}),
            answered(json!({"text": " mine ", "selected": []})),
        ),
        (
            Form::CheckboxesOther,
            json!({"button": "yes", "text": "mine", "selected": ["b"]}),
            answered(json!({"text": "mine", "selected": ["b"]})),
        ),
    ];
    let (mut live, serial, presenter) = presented_listing(&BOTH);
    for (form, answer, expected) in cases {
        let id = opened_id(&open(&mut live, form, |_| {}));
        assert!(
            presenter
                .last("ask.request")
                .is_some_and(|request| request["ask"]["id"] == json!(id))
        );
        resolve(&mut live, serial, &presenter, &id, answer.clone());
        assert_eq!(ask_result(&mut live.fixture.core, &id), expected, "{answer}");
    }
}

/// Upstream's `aFormAnswerWithAPayloadItsFormWouldNotProduceIsRefused` and `aPlainAskAnswerCarryingAPayloadIsRefused`.
#[test]
fn a_form_answer_with_a_payload_its_form_would_not_produce_is_refused() {
    let cases = [
        (Form::Line, json!({"button": "yes", "selected": ["a"]})),
        (Form::Line, json!({"button": "yes"})),
        (Form::Line, json!({"button": "yes", "text": "x".repeat(65_537)})),
        (Form::Line, json!({"button": "yes", "text": "a\u{1}b"})),
        (Form::Line, json!({"button": "yes", "text": "a\tb"})),
        (Form::Multiline, json!({"button": "yes", "text": "a\u{7f}\nb"})),
        (Form::Line, json!({"button": "no", "text": "typed"})),
        (Form::Line, json!({"text": "typed"})),
        (Form::Radio, json!({"button": "yes", "selected": ["c"]})),
        (Form::Radio, json!({"button": "yes", "selected": []})),
        (Form::Radio, json!({"button": "yes", "selected": ["a", "b"]})),
        (Form::Radio, json!({"button": "yes", "text": "text", "selected": ["a"]})),
        (Form::Checkboxes, json!({"button": "yes", "selected": ["b", "a"]})),
        (Form::Checkboxes, json!({"button": "yes", "selected": ["a", "a"]})),
        (Form::Checkboxes, json!({"button": "yes"})),
        (
            Form::Checkboxes,
            json!({"button": "yes", "text": "mine", "selected": ["a"]}),
        ),
        (
            Form::RadioOther,
            json!({"button": "yes", "text": "mine", "selected": ["a"]}),
        ),
        (Form::RadioOther, json!({"button": "yes", "text": "  ", "selected": []})),
        (
            Form::CheckboxesOther,
            json!({"button": "yes", "text": "a\tb", "selected": ["a"]}),
        ),
        (Form::Plain, json!({"button": "yes", "text": "x"})),
    ];
    let (mut live, serial, presenter) = presented_listing(&BOTH);
    let session = live.session;
    for (form, answer) in cases {
        let id = opened_id(&open(&mut live, form, |_| {}));
        assert!(
            presenter
                .last("ask.request")
                .is_some_and(|request| request["ask"]["id"] == json!(id))
        );
        resolve(&mut live, serial, &presenter, &id, answer.clone());
        assert_eq!(ask_result(&mut live.fixture.core, &id)["result"], "pending", "{answer}");
        assert_eq!(
            node(&mut live.fixture, session)["ask"],
            json!({"id": id, "remote": true}),
            "{answer}"
        );
        cancel(&mut live, &id);
    }
}

/// A form ask taken back after its presenter went away is drawn here from the form's initial values.
#[test]
fn a_handback_restarts_the_form_from_its_initial_values() {
    let (mut live, serial, presenter) = presented_listing(&BOTH);
    let session = live.session;
    let id = opened_id(&open(&mut live, Form::Multiline, |a| {
        a.value = Some("draft".to_owned());
    }));
    assert!(presenter.last("ask.request").is_some());
    live.fixture
        .core
        .update_session_ask_draft(session, &id, |draft| draft.set_text("typed elsewhere"));
    live.fixture.core.presentation_closed(serial);
    live.fixture.core.commit();
    assert_eq!(
        node(&mut live.fixture, session)["ask"],
        json!({"id": id}),
        "drawn here now"
    );
    let pending = live.fixture.store().session(session).unwrap().ask().unwrap();
    assert_eq!(pending.draft, AskDraft::new(pending.ask.form.as_ref()));
    assert_eq!(pending.draft.text(), "draft");
    live.fixture.core.answer_session_ask_button(session, &id, 0);
    live.fixture.core.commit();
    assert_eq!(
        ask_result(&mut live.fixture.core, &id),
        json!({"result": "answered", "id": "yes", "label": "Yes", "index": 0, "text": "draft"})
    );
}
