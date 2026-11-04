//! Group attach's one-request wire contract and human/raw output (`ZmxCommandsTests`/`SocketClientTests`).

use std::sync::Mutex;

use super::*;

fn group_run(response: Response, json: bool) -> TestResult<Output> {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let requests = Arc::new(Mutex::new(Vec::new()));
    let received = Arc::clone(&requests);
    let _server = serve(&socket, (), move |(), request| {
        received
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(request);
        response.clone().into()
    })?;
    let mut arguments = vec![
        "live",
        "attach",
        "buildbox",
        "s1",
        "--companions",
        "--window",
        "local-window",
    ];
    if json {
        arguments.push("--json");
    }
    let output = run_on(dir.path(), &socket, &arguments)?;
    assert_eq!(
        *requests.lock().unwrap_or_else(std::sync::PoisonError::into_inner),
        vec![serde_json::from_value::<Request>(serde_json::json!({
            "cmd": "live.attach.group", "target": "s1", "args": {"host": "buildbox", "window": "local-window"}
        }))?],
        "one group request, with no fallback"
    );
    Ok(output)
}

#[test]
fn group_attach_prints_the_parent_then_each_companion_origin_to_local() -> TestResult {
    let result = serde_json::json!({
        "id": "P",
        "remoteGroup": {"membershipKnown": true, "companions": [
            {"origin": "c1", "id": "L1"}, {"origin": "pg", "id": "L2"}
        ]}
    });
    assert_eq!(
        outcome(&group_run(Response::with_result(result), false)?),
        (Some(0), "P\n  c1 -> L1\n  pg -> L2\n".to_owned(), String::new())
    );
    Ok(())
}

#[test]
fn group_attach_success_preserves_empty_and_absent_receipt_fallbacks() -> TestResult {
    for (result, expected) in [
        (
            serde_json::json!({"id": "P", "remoteGroup": {"membershipKnown": true, "companions": []}}),
            "P\n",
        ),
        (serde_json::json!({}), "ok\n"),
        (
            serde_json::json!({"remoteGroup": {"membershipKnown": true, "companions": [{"origin": "c1"}]}}),
            "ok\n  c1 -> -\n",
        ),
    ] {
        assert_eq!(
            outcome(&group_run(Response::with_result(result), false)?),
            (Some(0), expected.to_owned(), String::new())
        );
    }
    Ok(())
}

#[test]
fn group_attach_unknown_membership_note_names_the_host_on_success() -> TestResult {
    let result = serde_json::json!({"id": "P", "remoteGroup": {"membershipKnown": false, "companions": []}});
    assert_eq!(
        outcome(&group_run(Response::with_result(result), false)?),
        (
            Some(0),
            "P\n".to_owned(),
            "note: buildbox does not report group membership; unlisted companions cannot be detected\n".to_owned()
        )
    );
    Ok(())
}

#[test]
fn group_attach_partial_failure_prints_error_and_unknown_membership_then_fails() -> TestResult {
    let response = Response::failure_with_result(
        "kept build (P) and 0 companion(s); not attached: logs (boom)",
        serde_json::json!({"id": "P", "remoteGroup": {
            "membershipKnown": false, "companions": [{"origin": "c1", "error": "boom"}]
        }}),
    );
    assert_eq!(
        outcome(&group_run(response, false)?),
        (
            Some(1),
            String::new(),
            "error: kept build (P) and 0 companion(s); not attached: logs (boom)\n\
             note: buildbox does not report group membership; unlisted companions cannot be detected\n"
                .to_owned()
        )
    );
    Ok(())
}

#[test]
fn group_attach_unknown_command_is_named_and_nothing_else_is_sent() -> TestResult {
    let response =
        Response::failure("invalid request: Cannot initialize Command from invalid String value live.attach.group");
    assert_eq!(
        outcome(&group_run(response.clone(), false)?),
        (
            Some(1),
            String::new(),
            "error: the running app is too old for group attach\n".to_owned()
        )
    );
    assert_eq!(
        outcome(&group_run(response.clone(), true)?),
        (
            Some(1),
            format!("{}\n", serde_json::to_string(&response)?),
            String::new()
        )
    );
    Ok(())
}

#[test]
fn group_attach_other_errors_keep_the_apps_message() -> TestResult {
    assert_eq!(
        outcome(&group_run(
            Response::failure("no attachable session s1 on buildbox"),
            false
        )?),
        (
            Some(1),
            String::new(),
            "error: no attachable session s1 on buildbox\n".to_owned()
        )
    );
    Ok(())
}

#[test]
fn group_attach_json_keeps_unknown_fields_and_has_no_note_on_success_or_failure() -> TestResult {
    for ok in [true, false] {
        let response: Response = serde_json::from_value(serde_json::json!({
            "ok": ok,
            "error": (!ok).then_some("partial failure"),
            "result": {"id": "P", "remoteGroup": {"membershipKnown": false, "companions": []}, "futureField": 1}
        }))?;
        assert_eq!(
            outcome(&group_run(response.clone(), true)?),
            (
                Some(i32::from(!ok)),
                format!("{}\n", serde_json::to_string(&response)?),
                String::new()
            )
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn group_attach_json_prints_the_original_response_whitespace() -> TestResult {
    let dir = short_dir()?;
    let socket = dir.path().join("ngterm.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket)?;
    let line = " {\"ok\": true, \"result\": {\"id\": \"P\", \"remoteGroup\": {\"membershipKnown\": false, \
                \"companions\": []}, \"futureField\": 1}} ";
    let server = std::thread::spawn(move || -> io::Result<Request> {
        let (mut connection, _) = listener.accept()?;
        connection.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        connection.set_write_timeout(Some(std::time::Duration::from_secs(5)))?;
        let mut request = String::new();
        BufReader::new(&connection).read_line(&mut request)?;
        connection.write_all(format!("{line}\n").as_bytes())?;
        Ok(serde_json::from_str(&request)?)
    });
    let output = run_on(
        dir.path(),
        &socket,
        &["live", "attach", "buildbox", "s1", "--companions", "--json"],
    )?;
    assert_eq!(server.join().unwrap()?.cmd.wire_name(), "live.attach.group");
    assert_eq!(outcome(&output), (Some(0), format!("{line}\n"), String::new()));
    Ok(())
}
