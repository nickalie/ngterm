use super::*;
use clap::Parser;
use ngterm_core::paths::MapEnv;
use serde_json::json;

fn parse(args: &[&str]) -> Msg {
    let line = [vec!["ngtermctl", "msg"], args.to_vec()].concat();
    let crate::Command::Msg(command) = crate::Ngtermctl::try_parse_from(line).unwrap().command else {
        unreachable!()
    };
    command
}

#[test]
fn requests_keep_recipient_lists_separate_from_navigation_and_include_identity() {
    let command = parse(&["send", "hello", "--to", "s:left", "--to", "s:right", "--as", "s:p"]);
    assert_eq!(
        serde_json::to_value(command.request(&MapEnv::default(), None).unwrap()).unwrap(),
        json!({"cmd":"msg.send", "args":{"body":"hello","recipients":["s:left","s:right"],"identity":"s:p"}})
    );
    let command = parse(&["read", "--ack", "--as", "s:p"]);
    assert_eq!(
        serde_json::to_value(command.request(&MapEnv::default(), None).unwrap()).unwrap(),
        json!({"cmd":"msg.read", "args":{"ack":true,"identity":"s:p"}})
    );
    let command = parse(&["ack", "m1", "m2"]);
    assert_eq!(
        serde_json::to_value(command.request(&MapEnv::default(), None).unwrap()).unwrap(),
        json!({"cmd":"msg.ack", "args":{"ids":["m1","m2"]}})
    );
}

#[test]
fn checks_body_and_wait_forms_before_connecting() {
    assert_eq!(
        parse(&["send"]).check().unwrap_err(),
        "provide the body as TEXT or --stdin, exactly one"
    );
    assert!(parse(&["send", "hi", "--stdin"]).check().is_err());
    assert_eq!(
        parse(&["read", "--timeout", "1"]).check().unwrap_err(),
        "--timeout needs --wait"
    );
    for timeout in ["0", "-1", "NaN", "inf"] {
        assert_eq!(
            parse(&["read", "--wait", "--timeout", timeout]).check().unwrap_err(),
            "--timeout must be a positive number of seconds"
        );
    }
}

fn reply(result: Option<serde_json::Value>, ok: bool) -> Reply {
    let response = ngterm_proto::Response {
        ok,
        result,
        error: (!ok).then(|| "refused".into()),
    };
    Reply {
        raw: serde_json::to_vec(&response).unwrap(),
        response,
    }
}

fn message(id: &str, body: &str, from: Option<&str>) -> serde_json::Value {
    let mut message = json!({"id":id,"body":body,"to":"S:P","ts":1.0});
    if let Some(from) = from {
        message["from"] = json!(from);
    }
    message
}

#[test]
fn identity_comes_only_from_a_complete_environment_and_as_wins() {
    let env = environment([("NGTERM_SESSION_ID", "S"), ("NGTERM_PANE_ID", "P")]);
    let command = parse(&["send", "hi", "--all"]);
    assert_eq!(
        command.request(&env, None).unwrap().args.unwrap().identity.as_deref(),
        Some("S:P")
    );
    for pairs in [
        vec![("NGTERM_SESSION_ID", "S")],
        vec![("NGTERM_PANE_ID", "P")],
        vec![("NGTERM_SESSION_ID", "")],
        vec![("NGTERM_PANE_ID", "")],
        vec![("NGTERM_SESSION_ID", "S"), ("NGTERM_PANE_ID", "")],
        vec![("NGTERM_SESSION_ID", ""), ("NGTERM_PANE_ID", "")],
    ] {
        let env = environment(pairs);
        for command in [parse(&["send", "hi", "--all"]), parse(&["read"]), parse(&["ack", "id"])] {
            assert!(command.request(&env, None).is_err());
        }
        assert_eq!(
            parse(&["send", "hi", "--all", "--as", "X:Y"])
                .request(&env, None)
                .unwrap()
                .args
                .unwrap()
                .identity
                .as_deref(),
            Some("X:Y")
        );
    }
    for command in [parse(&["send", "hi", "--all"]), parse(&["read"]), parse(&["ack", "id"])] {
        assert!(
            command
                .request(&MapEnv::default(), None)
                .unwrap()
                .args
                .unwrap()
                .identity
                .is_none()
        );
    }
}

#[test]
fn stdin_and_broadcast_request_and_utf8_refusal() {
    let command = parse(&["send", "--stdin", "--all", "--workspace", "w"]);
    let args = command
        .request(&MapEnv::default(), Some(b"body\n".to_vec()))
        .unwrap()
        .args
        .unwrap();
    assert_eq!(
        (
            args.body.as_deref(),
            args.workspace.as_deref(),
            args.all,
            args.recipients
        ),
        (Some("body\n"), Some("w"), Some(true), None)
    );
    assert!(command.request(&MapEnv::default(), Some(vec![255])).is_err());
}

#[test]
fn send_reports_partial_rejections_without_losing_raw_json() {
    let response = reply(
        Some(
            json!({"messageID":"id","delivered":["S:P"],"rejected":[{"address":"a:b","reason":"mailbox full"},{"address":"c:d","reason":"unknown pane"}]}),
        ),
        true,
    );
    assert_eq!(
        parse(&["send", "hi", "--all"]).output(&response).unwrap(),
        MessageOutput {
            stdout: "id\n".into(),
            stderr: "rejected a:b: mailbox full\nrejected c:d: unknown pane\n".into(),
            failed: true
        }
    );
    assert_eq!(
        parse(&["send", "hi", "--all", "--json"]).output(&response).unwrap(),
        MessageOutput {
            stdout: response.line() + "\n",
            failed: true,
            ..MessageOutput::default()
        }
    );
    let none = reply(Some(json!({"messageID":"id","delivered":[],"rejected":[]})), true);
    assert!(!parse(&["send", "hi", "--all"]).output(&none).unwrap().failed);
}

#[test]
fn server_errors_fail_every_command_with_json_or_human_error() {
    let response = reply(None, false);
    for argv in [vec!["send", "hi", "--all"], vec!["read"], vec!["ack", "id"]] {
        assert_eq!(
            parse(&argv).output(&response).unwrap(),
            MessageOutput {
                stderr: "error: refused\n".into(),
                failed: true,
                ..MessageOutput::default()
            }
        );
        let argv = [argv, vec!["--json"]].concat();
        assert_eq!(parse(&argv).output(&response).unwrap().stdout, response.line() + "\n");
    }
}

#[test]
fn read_records_remaining_and_empty_and_raw_json() {
    let response = reply(
        Some(json!({"messages":[message("a","one",Some("S:Q")),message("b","two\n",None)],"remaining":2})),
        true,
    );
    let read = parse(&["read"]);
    assert_eq!(
        read.output(&response).unwrap(),
        MessageOutput {
            stdout: "a from S:Q\none\nb from outside ngterm\ntwo\n".into(),
            stderr: "2 more pending\n".into(),
            failed: false
        }
    );
    assert_eq!(
        parse(&["read", "--json"]).output(&response).unwrap().stdout,
        response.line() + "\n"
    );
    assert_eq!(
        read.output(&reply(Some(json!({"messages":[],"remaining":0})), true))
            .unwrap(),
        MessageOutput::default()
    );
}

#[test]
fn read_ack_prints_only_when_exact_ordered_ids_are_confirmed_even_under_json() {
    for acked in [
        None,
        Some(json!([])),
        Some(json!(["other"])),
        Some(json!(["b", "a"])),
        Some(json!(["a", "b", "a"])),
    ] {
        let mut result = json!({"messages":[message("a","secret",None),message("b","other",None)]});
        if let Some(acked) = acked {
            result["acked"] = acked;
        }
        let response = reply(Some(result), true);
        for argv in [vec!["read", "--ack"], vec!["read", "--ack", "--json"]] {
            assert_eq!(
                parse(&argv).output(&response).unwrap(),
                MessageOutput {
                    stderr: "error: the app did not confirm which messages it removed; nothing was printed\n".into(),
                    failed: true,
                    ..MessageOutput::default()
                }
            );
        }
    }
    let response = reply(Some(json!({"messages":[message("a","one",None)],"acked":["a"]})), true);
    assert_eq!(
        parse(&["read", "--ack"]).output(&response).unwrap().stdout,
        "a from outside ngterm\none\n"
    );
    assert_eq!(
        parse(&["read", "--ack"])
            .output(&reply(Some(json!({"messages":[],"acked":[]})), true))
            .unwrap(),
        MessageOutput::default()
    );
    assert_eq!(
        parse(&["ack", "id"])
            .output(&reply(Some(json!({"acked":["id"]})), true))
            .unwrap()
            .stdout,
        "acked 1\n"
    );
}

struct FakeRead {
    replies: std::collections::VecDeque<Result<Reply, ClientError>>,
    elapsed: Duration,
    sleeps: Vec<Duration>,
    requests: Vec<Request>,
}
impl FakeRead {
    fn new(replies: Vec<Reply>) -> Self {
        Self {
            replies: replies.into_iter().map(Ok).collect(),
            elapsed: Duration::ZERO,
            sleeps: vec![],
            requests: vec![],
        }
    }
}
impl ReadDependencies for FakeRead {
    fn send(&mut self, request: &Request) -> Result<Reply, ClientError> {
        self.requests.push(request.clone());
        self.replies.pop_front().expect("unexpected extra read")
    }
    fn sleep(&mut self, delay: Duration) {
        self.sleeps.push(delay);
        self.elapsed += delay;
    }
    fn now(&mut self) -> Duration {
        self.elapsed
    }
}
fn poll(command: Msg, fake: &mut FakeRead) -> Result<Option<Reply>, ClientError> {
    let request = command.request(&MapEnv::default(), None).unwrap();
    let MsgCommand::Read(read) = command.command else {
        unreachable!()
    };
    read.poll(&request, fake)
}
fn empty() -> Reply {
    reply(Some(json!({"messages":[],"acked":[],"remaining":0})), true)
}

#[test]
fn read_without_wait_asks_once_and_wait_polls_the_real_ack_request() {
    let mut fake = FakeRead::new(vec![empty()]);
    assert!(poll(parse(&["read"]), &mut fake).unwrap().is_some());
    assert!(fake.sleeps.is_empty());
    let ready = reply(Some(json!({"messages":[message("a","one",None)],"acked":["a"]})), true);
    let mut fake = FakeRead::new(vec![empty(), empty(), ready.clone()]);
    assert_eq!(
        poll(parse(&["read", "--wait", "--ack"]), &mut fake)
            .unwrap()
            .unwrap()
            .raw,
        ready.raw
    );
    assert_eq!(fake.sleeps, vec![Duration::from_millis(250); 2]);
    assert_eq!(fake.requests.len(), 3);
    assert!(
        fake.requests
            .iter()
            .all(|r| r.cmd == Command::Msg(MsgCmd::Read) && r.args.as_ref().unwrap().ack == Some(true))
    );
}

#[test]
fn wait_stops_on_server_error_unconfirmed_ack_and_client_error() {
    for response in [reply(None, false), reply(Some(json!({"messages":[]})), true)] {
        let mut fake = FakeRead::new(vec![response]);
        assert!(poll(parse(&["read", "--wait", "--ack"]), &mut fake).unwrap().is_some());
        assert!(fake.sleeps.is_empty());
    }
    let mut fake = FakeRead::new(vec![]);
    fake.replies.push_back(Err(ClientError::new("gone")));
    assert!(poll(parse(&["read", "--wait"]), &mut fake).is_err());
}

#[test]
fn timeout_is_monotonic_and_clips_the_last_poll_delay() {
    let mut fake = FakeRead::new(vec![empty(), empty(), empty()]);
    assert!(
        poll(parse(&["read", "--wait", "--timeout", "0.3"]), &mut fake)
            .unwrap()
            .is_none()
    );
    assert_eq!(fake.sleeps, vec![Duration::from_millis(250), Duration::from_millis(50)]);
    let MsgCommand::Read(read) = parse(&["read", "--wait", "--timeout", "1e300"]).command else {
        unreachable!()
    };
    assert!(read.timeout_duration().is_none());
}

fn environment<'a>(pairs: impl IntoIterator<Item = (&'a str, &'a str)>) -> MapEnv {
    pairs
        .into_iter()
        .fold(MapEnv::default(), |env, (key, value)| env.with(key, value))
}

/// Runs the command in a test subprocess so stdin and stdout belong only to this invocation.
#[test]
fn command_probe() {
    let Ok(arguments) = std::env::var("CTL_COVERAGE_ARGUMENTS") else {
        return;
    };
    let arguments: Vec<String> = serde_json::from_str(&arguments).unwrap();
    let parsed = crate::Ngtermctl::try_parse_from(arguments).unwrap();
    eprintln!("PROBE_RESULT:{:?}", parsed.command.run(&ngterm_core::paths::ProcessEnv));
}

#[cfg(unix)]
pub(crate) fn run_command(
    arguments: &[&str],
    input: &[u8],
    environment: &[(&str, &str)],
    replies: &[&str],
) -> (std::process::Output, Vec<serde_json::Value>) {
    use std::io::{BufRead as _, Write as _};
    use std::os::unix::net::UnixListener;
    use std::process::Stdio;
    use std::thread;

    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("ctl.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    let replies: Vec<String> = replies.iter().map(|reply| format!("{reply}\n")).collect();
    let server = thread::spawn(move || {
        let mut requests = Vec::new();
        for reply in replies {
            let deadline = Instant::now() + Duration::from_secs(10);
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "command never connected");
                        thread::yield_now();
                    }
                    Err(error) => panic!("accept: {error}"),
                }
            };
            // On macOS an accepted socket inherits the listener's non-blocking mode; the request is read blocking.
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut line = String::new();
            std::io::BufReader::new(&stream).read_line(&mut line).unwrap();
            requests.push(serde_json::from_str(&line).unwrap());
            stream.write_all(reply.as_bytes()).unwrap();
        }
        requests
    });
    let mut argv = vec!["ngtermctl".to_owned()];
    argv.extend(arguments.iter().map(|argument| (*argument).to_owned()));
    argv.extend(["--socket".to_owned(), socket.to_string_lossy().into_owned()]);
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "messages::tests::command_probe",
            "--exact",
            "--nocapture",
            "--test-threads=1",
        ])
        .env_clear()
        .envs(std::env::vars_os().filter(|(key, _)| !key.to_string_lossy().starts_with("NGTERM_")))
        .envs(environment.iter().copied())
        .env("CTL_COVERAGE_ARGUMENTS", serde_json::to_string(&argv).unwrap())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    let requests = server.join().unwrap();
    assert!(output.status.success(), "{output:?}");
    (output, requests)
}

#[cfg(unix)]
#[test]
fn commands_emit_the_servers_output_and_send_stdin_bytes_as_the_body() {
    let (output, requests) = run_command(
        &["msg", "send", "--stdin", "--to", "S:P"],
        "héllo\n".as_bytes(),
        &[],
        &[r#"{"ok":true,"result":{"messageID":"m1"}}"#],
    );
    assert!(String::from_utf8(output.stdout).unwrap().contains("m1\n"));
    assert_eq!(
        requests,
        [json!({"cmd":"msg.send","args":{"body":"héllo\n","recipients":["S:P"]}})]
    );
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("PROBE_RESULT:Ok(())")
    );
    let (output, requests) = run_command(
        &["msg", "ack", "m1", "--as", "S:P"],
        b"",
        &[],
        &[r#"{"ok":false,"error":"unknown pane"}"#],
    );
    assert_eq!(requests[0]["cmd"], "msg.ack");
    let error = String::from_utf8(output.stderr).unwrap();
    assert!(error.contains("error: unknown pane\n"), "{error}");
    assert!(error.contains("PROBE_RESULT:Err(Exit("), "{error}");
}

#[cfg(unix)]
#[test]
fn a_live_wait_reads_again_and_cancels_its_timeout_when_mail_arrives() {
    let (output, requests) = run_command(
        &["msg", "read", "--wait", "--timeout", "5", "--ack", "--as", "S:P"],
        b"",
        &[],
        &[
            r#"{"ok":true,"result":{"messages":[],"acked":[]}}"#,
            r#"{"ok":true,"result":{"messages":[{"id":"m1","body":"ready","to":"S:P","ts":1}],"acked":["m1"]}}"#,
        ],
    );
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0], requests[1]);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("m1 from outside ngterm\nready\n")
    );
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("PROBE_RESULT:Ok(())")
    );
}
