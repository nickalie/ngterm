//! New v0.40.0 shapes are optional on older peers and retain their exact wire keys.
use ngterm_proto::{Args, ControlEvent, ControlResult, ControlSessionNode, Request};
use serde_json::json;

#[test]
fn message_requests_keep_scalar_navigation_separate_from_recipient_arrays() {
    let wire = json!({"cmd":"msg.send","args":{"body":"hello","to":"down","recipients":["s:left","s:right"],"identity":"s:p","ack":false,"ids":["m"]}});
    let request: Request = serde_json::from_value(wire.clone()).unwrap();
    let args = request.args.as_ref().unwrap();
    assert_eq!(args.to.as_deref(), Some("down"));
    assert_eq!(args.recipients.as_ref().unwrap(), &["s:left", "s:right"]);
    assert_eq!(serde_json::to_value(request).unwrap(), wire);
    assert!(serde_json::from_value::<Args>(json!({"recipients":"s:left"})).is_err());
    assert!(serde_json::from_value::<Args>(json!({"to":["down"]})).is_err());
}

#[test]
fn message_receipts_and_remote_group_members_round_trip() {
    let wire = json!({"messageID":"M", "delivered":["S:P"], "rejected":[{"address":"missing:left","reason":"unknown pane"},{"address":"full:right","reason":"mailbox full"}], "messages":[{"id":"M","from":"A:B","to":"S:P","ts":1.25,"body":"hi"}], "remaining":0, "acked":[], "remoteGroup":{"membershipKnown":false,"companions":[{"origin":"ok","id":"local"},{"origin":"bad","error":"unavailable"}]}});
    let result: ControlResult = serde_json::from_value(wire.clone()).unwrap();
    assert_eq!(serde_json::to_value(result).unwrap(), wire);
    let old: ControlResult = serde_json::from_value(json!({"id":"old"})).unwrap();
    assert!(old.message_id.is_none() && old.remote_group.is_none() && old.messages.is_none());
}

#[test]
fn info_fields_round_trip_without_becoming_required() {
    let mut node = ControlSessionNode::new("S".into(), "name".into(), "/".into());
    node.info = Some("**busy**".into());
    node.info_markdown = Some(true);
    node.info_hide_after = Some(1.5);
    let wire = serde_json::to_value(&node).unwrap();
    assert_eq!(wire["info"], "**busy**");
    assert_eq!(wire["infoMarkdown"], true);
    assert_eq!(wire["infoHideAfter"], 1.5);
    assert_eq!(serde_json::from_value::<ControlSessionNode>(wire).unwrap(), node);
    let old = serde_json::to_value(ControlSessionNode::new("S".into(), "name".into(), "/".into())).unwrap();
    for field in ["info", "infoMarkdown", "infoHideAfter"] {
        assert!(old.get(field).is_none());
    }
}

#[test]
fn message_events_use_message_id_and_omit_an_absent_sender() {
    let wire = json!({"seq":1,"ts":1.0,"kind":"message","session":"S","payload":{"paneID":"P","messageID":"M"}});
    let event: ControlEvent = serde_json::from_value(wire.clone()).unwrap();
    assert!(event.payload.body.is_none() && event.payload.from.is_none());
    assert_eq!(serde_json::to_value(event).unwrap(), wire);
}
