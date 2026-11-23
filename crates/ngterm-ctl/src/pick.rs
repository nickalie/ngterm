//! `ngtermctl pick` (`agtermctlKit/MiscCommands.swift` `Pick`): the native fuzzy picker. `open` is the group's default
//! subcommand, so the shell-friendly common case is `printf 'one\ntwo\n' | ngtermctl pick`; `result` and `cancel` make
//! `--no-block` usable without speaking the protocol.

use std::io::{self, Read};

use ngterm_core::modal::{TIMEOUT_ERROR, is_valid_timeout};
use ngterm_core::paths::Env;
use ngterm_proto::command::PickCmd;
use ngterm_proto::{Args, Command, PickItem, Request};

use crate::client::os_message;
use crate::modal::{Family, LiveIo, ModalIo, ModalRunner};
use crate::{BasicOptions, ClientOptions, Failure, RequestCommand};

/// `ngtermctl pick <subcommand>`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Open, poll, or cancel a native fuzzy picker.",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Pick {
    #[command(subcommand)]
    command: PickCommand,
}

/// The subcommands, in upstream's `Pick.configuration.subcommands` order.
#[derive(Debug, clap::Subcommand)]
enum PickCommand {
    Open(Open),
    Result(ResultCommand),
    Cancel(Cancel),
}

impl Pick {
    /// What the parser cannot check (upstream's `validate()`).
    pub fn check(&self) -> Result<(), String> {
        match &self.command {
            PickCommand::Open(command) => command.validate(),
            PickCommand::Result(_) | PickCommand::Cancel(_) => Ok(()),
        }
    }

    /// Runs the command.
    pub fn execute(&self, env: &dyn Env) -> Result<(), Failure> {
        match &self.command {
            PickCommand::Open(command) => {
                let input = read_stdin()?;
                let mut io = LiveIo {
                    socket: command.options.basic.socket_path(env),
                };
                command.execute(&input, &mut io, env)
            }
            PickCommand::Result(command) => {
                let mut io = LiveIo {
                    socket: command.options.basic.socket_path(env),
                };
                command.execute(&mut io)
            }
            PickCommand::Cancel(command) => command.default_run(env),
        }
    }
}

/// Standard input, whole.
fn read_stdin() -> Result<Vec<u8>, Failure> {
    let mut input = Vec::new();
    io::stdin()
        .read_to_end(&mut input)
        .map_err(|error| Failure::Local(format!("could not read stdin: {}", os_message(&error))))?;
    Ok(input)
}

/// What separates bare lines: Foundation's `CharacterSet.newlines`.
fn is_newline(c: char) -> bool {
    matches!(
        c,
        '\n' | '\u{0B}' | '\u{0C}' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'
    )
}

/// The items on stdin. Its first non-whitespace byte decides: `[` is a JSON item array, keeping caller-supplied ids,
/// subtitles and status fields; anything else is lines, each its own id and label, blank ones dropped.
pub fn parse_items(input: &[u8]) -> Result<Vec<PickItem>, Failure> {
    let first = input.iter().find(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r'));
    if first == Some(&b'[') {
        return serde_json::from_slice(input)
            .map_err(|error| Failure::Local(format!("stdin is not a valid JSON item array: {error}")));
    }
    let text = std::str::from_utf8(input)
        .map_err(|_| Failure::Usage("stdin must be UTF-8 text or a JSON item array".to_owned()))?;
    Ok(text
        .split(is_newline)
        .filter(|line| !line.trim().is_empty())
        .map(|line| PickItem {
            id: line.to_owned(),
            label: line.to_owned(),
            subtitle: None,
            ..PickItem::default()
        })
        .collect())
}

/// `ngtermctl pick [open]`: reads the choices from stdin, then waits for the answer unless `--no-block`.
#[derive(Debug, clap::Args)]
#[command(about = "Read choices from stdin and open a native fuzzy picker.")]
pub struct Open {
    #[arg(long, help = "Placeholder text shown in the picker query field.")]
    prompt: Option<String>,

    #[arg(long, help = "Initial text for the picker query field; it opens already filtered.")]
    query: Option<String>,

    #[arg(long, help = "Accept the current query as a custom result.")]
    allow_custom: bool,

    #[arg(
        long,
        help = "Item id to open highlighted; a --query that hides it falls back to the first row."
    )]
    select: Option<String>,

    #[arg(long, help = "Raise the target window when the picker opens.")]
    follow: bool,

    #[arg(long, help = "Cancel the picker unanswered after this many seconds (at most 86400).")]
    timeout: Option<f64>,

    #[arg(long, help = "Print the picker id and return without waiting for a result.")]
    no_block: bool,

    #[arg(
        long,
        help = "Bind the picker to a session: it shows wherever that session is presented."
    )]
    session: Option<String>,

    #[command(flatten)]
    options: ClientOptions,
}

impl Open {
    /// Upstream's `validate()`.
    fn validate(&self) -> Result<(), String> {
        if self.timeout.is_some_and(|timeout| !is_valid_timeout(timeout)) {
            return Err(TIMEOUT_ERROR.to_owned());
        }
        Ok(())
    }

    /// The open request for the items in `input`, bound to session `bound` when given, else to `--session`.
    pub fn request(&self, input: &[u8], bound: Option<String>) -> Result<Request, Failure> {
        let args = Args {
            follow: self.follow.then_some(true),
            items: Some(parse_items(input)?),
            prompt: self.prompt.clone(),
            query: self.query.clone(),
            allow_custom: self.allow_custom.then_some(true),
            selection: self.select.clone(),
            timeout: self.timeout,
            ..Args::default()
        };
        Ok(Request {
            cmd: Command::Pick(PickCmd::Open),
            target: bound.or_else(|| self.session.clone()),
            args: self.options.with_window(Some(args)),
        })
    }

    /// The session the pick is bound to (upstream's `boundSession`): `--session`, else the pane's own session when the
    /// server is `ngterm-headless`, which has no window of its own to show a pick in. An app keeps the window pick.
    fn bound_session(&self, io: &mut dyn ModalIo, env: &dyn Env) -> Result<Option<String>, Failure> {
        if let Some(session) = &self.session {
            return Ok(Some(session.clone()));
        }
        let Some(own) = env
            .var("NGTERM_SESSION_ID")
            .and_then(|own| own.into_string().ok())
            .filter(|own| !own.is_empty())
        else {
            return Ok(None);
        };
        let version = Request {
            cmd: Command::Version,
            target: None,
            args: None,
        };
        let reply = io.send(&version)?;
        let headless = reply
            .response
            .result
            .as_ref()
            .and_then(|result| result.get("headless"))
            .and_then(serde_json::Value::as_bool);
        Ok((headless == Some(true)).then_some(own))
    }

    /// Opens the picker over `io` and prints its id or, once it resolves, its result.
    pub fn execute(&self, input: &[u8], io: &mut dyn ModalIo, env: &dyn Env) -> Result<(), Failure> {
        let bound = self.bound_session(io, env)?;
        let request = self.request(input, bound)?;
        ModalRunner {
            family: Family::Pick,
            json: self.options.basic.json,
            io,
        }
        .open(&request, self.no_block)
    }
}

/// `ngtermctl pick result <id>`: one read, printed as JSON, `pending` included.
#[derive(Debug, clap::Args)]
#[command(about = "Print a picker's current or terminal result as JSON.")]
pub struct ResultCommand {
    #[arg(help = "Exact picker id returned by pick open.")]
    id: String,

    #[command(flatten)]
    options: ClientOptions,
}

impl ResultCommand {
    fn request(&self) -> Request {
        Request {
            cmd: Command::Pick(PickCmd::Result),
            target: Some(self.id.clone()),
            args: self.options.with_window(None),
        }
    }

    /// Reads the result over `io` and exits with its outcome's status.
    pub fn execute(&self, io: &mut dyn ModalIo) -> Result<(), Failure> {
        ModalRunner {
            family: Family::Pick,
            json: self.options.basic.json,
            io,
        }
        .read(&self.request())
    }
}

/// `ngtermctl pick cancel <id>`.
#[derive(Debug, clap::Args)]
#[command(about = "Cancel a pending picker.")]
pub struct Cancel {
    #[arg(help = "Exact picker id returned by pick open.")]
    id: String,

    #[command(flatten)]
    options: ClientOptions,
}

impl RequestCommand for Cancel {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    fn make_request(&self) -> Request {
        Request {
            cmd: Command::Pick(PickCmd::Cancel),
            target: Some(self.id.clone()),
            args: self.options.with_window(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use clap::{CommandFactory, Parser};
    use serde_json::{Value, json};

    use ngterm_core::paths::MapEnv;

    use super::*;
    use crate::Ngtermctl;
    use crate::modal::tests::{FakeIo, answer};
    use crate::usage::name_default_subcommands;

    fn parse(argv: &[&str]) -> Result<PickCommand, String> {
        let command_line: Vec<OsString> = ["ngtermctl", "pick"].iter().chain(argv).map(OsString::from).collect();
        let named = name_default_subcommands(&Ngtermctl::command(), &command_line);
        let parsed = Ngtermctl::try_parse_from(named).map_err(|error| error.to_string())?;
        let crate::Command::Pick(group) = parsed.command else {
            panic!("not a pick command")
        };
        Ok(group.command)
    }

    fn open(argv: &[&str]) -> Open {
        let Ok(PickCommand::Open(open)) = parse(argv) else {
            panic!("not pick open")
        };
        open
    }

    fn open_request(argv: &[&str], input: &str) -> Value {
        serde_json::to_value(open(argv).request(input.as_bytes(), None).ok().unwrap()).unwrap()
    }

    fn items(input: &[u8]) -> Vec<PickItem> {
        parse_items(input).ok().unwrap()
    }

    fn item(id: &str, label: &str) -> PickItem {
        PickItem {
            id: id.to_owned(),
            label: label.to_owned(),
            subtitle: None,
            ..PickItem::default()
        }
    }

    #[test]
    fn open_is_the_default() {
        assert!(matches!(parse(&[]), Ok(PickCommand::Open(_))));
        assert!(matches!(parse(&["--no-block"]), Ok(PickCommand::Open(_))));
    }

    /// Upstream's `MiscCommandsTests` pick cases: `--session` is the request's target and `--timeout` its `timeout`.
    #[test]
    fn session_and_timeout_reach_the_request() {
        let request = open_request(&["--session", "S1", "--timeout", "2.5"], "a\n");
        assert_eq!(request["target"], json!("S1"));
        assert_eq!(request["args"]["timeout"], json!(2.5));
        let plain = open_request(&[], "a\n");
        assert_eq!(plain.get("target"), None);
        assert_eq!(plain["args"].get("timeout"), None);
    }

    #[test]
    fn a_timeout_out_of_range_is_refused_before_sending() {
        for timeout in ["0", "86400.5", "inf", "NaN"] {
            let Ok(PickCommand::Open(open)) = parse(&["--timeout", timeout]) else {
                panic!("{timeout} does not parse")
            };
            assert_eq!(open.validate(), Err(TIMEOUT_ERROR.to_owned()), "{timeout}");
        }
        assert_eq!(open(&["--timeout", "86400"]).validate(), Ok(()));
    }

    #[test]
    fn a_json_array_keeps_ids_subtitles_and_status_fields() {
        let input = concat!(
            "  [\n {\"id\":\"one\",\"label\":\"One\",\"subtitle\":\"First\"},\n",
            " {\"id\":\"two\",\"label\":\"Two\",\"status\":\"blocked\",\"statusColor\":\"#ff8800\",",
            "\"statusShape\":\"star\"}\n]"
        );
        let first = PickItem {
            subtitle: Some("First".to_owned()),
            ..item("one", "One")
        };
        let second = PickItem {
            status: Some("blocked".to_owned()),
            status_color: Some("#ff8800".to_owned()),
            status_shape: Some("star".to_owned()),
            ..item("two", "Two")
        };
        assert_eq!(items(input.as_bytes()), [first, second]);
        assert!(matches!(parse_items(b"[{\"id\":\"one\""), Err(Failure::Local(_))));
    }

    #[test]
    fn bare_lines_are_their_own_ids_and_blank_ones_are_dropped() {
        assert_eq!(items(b"One\nTwo\n"), [item("One", "One"), item("Two", "Two")]);
        assert_eq!(
            items(b"\nOne\n \t \n\nTwo\r\n"),
            [item("One", "One"), item("Two", "Two")]
        );
        // the whole line is the id: `=` is not a separator
        assert_eq!(items(b"a=Alpha\n"), [item("a=Alpha", "a=Alpha")]);
        assert!(items(b"").is_empty());
        assert!(matches!(
            parse_items(&[0xFF, b'\n']),
            Err(Failure::Usage(message)) if message == "stdin must be UTF-8 text or a JSON item array"
        ));
    }

    #[test]
    fn open_maps_every_option() {
        let argv = [
            "--prompt",
            "Choose one",
            "--query",
            "on",
            "--allow-custom",
            "--select",
            "One",
            "--follow",
            "--window",
            "w1",
            "--no-block",
        ];
        assert_eq!(
            open_request(&argv, "One\n"),
            json!({"cmd": "pick.open", "args": {
                "items": [{"id": "One", "label": "One"}], "prompt": "Choose one", "query": "on",
                "allowCustom": true, "selection": "One", "follow": true, "window": "w1"}})
        );
        assert!(open(&argv).no_block);
        assert!(!open(&[]).no_block);
        assert_eq!(
            open_request(&[], "One\n"),
            json!({"cmd": "pick.open", "args": {"items": [{"id": "One", "label": "One"}]}})
        );
        assert_eq!(
            open_request(&["--allow-custom", "--query", "old name"], ""),
            json!({"cmd": "pick.open", "args": {"items": [], "query": "old name", "allowCustom": true}})
        );
    }

    #[test]
    fn result_and_cancel_send_the_id_and_window() {
        let Ok(PickCommand::Result(result)) = parse(&["result", "pick-1", "--window", "w1"]) else {
            panic!("not pick result")
        };
        assert_eq!(
            serde_json::to_value(result.request()).unwrap(),
            json!({"cmd": "pick.result", "target": "pick-1", "args": {"window": "w1"}})
        );
        let Ok(PickCommand::Cancel(cancel)) = parse(&["cancel", "pick-1", "--window", "w1"]) else {
            panic!("not pick cancel")
        };
        assert_eq!(
            serde_json::to_value(cancel.make_request()).unwrap(),
            json!({"cmd": "pick.cancel", "target": "pick-1", "args": {"window": "w1"}})
        );
    }

    #[test]
    fn a_blocking_open_sends_the_items_then_polls_without_the_window() {
        let mut io = FakeIo::new(|request: &Request| {
            if request.cmd == Command::Pick(PickCmd::Open) {
                Ok(answer(&json!({"id": "P1"})))
            } else {
                Ok(answer(&json!({"pick": {"result": "custom", "query": "typed"}})))
            }
        });
        let outcome = open(&["--window", "w1"]).execute(b"a\nb\n", &mut io, &MapEnv::new());
        assert!(outcome.is_ok());
        assert_eq!(
            io.requests,
            [
                json!({"cmd": "pick.open", "args": {"items": [{"id": "a", "label": "a"}, {"id": "b", "label": "b"}],
                    "window": "w1"}}),
                json!({"cmd": "pick.result", "target": "P1"}),
            ]
        );
        assert_eq!(io.lines, [r#"{"result":"custom","query":"typed"}"#]);
    }

    /// Opens `argv` with `--no-block` against a server whose `version` answers `version`, in a pane of session `own`
    /// when given; the requests it sent.
    fn bound_requests(argv: &[&str], version: &Value, own: Option<&str>) -> Vec<Value> {
        let version = version.clone();
        let mut io = FakeIo::new(move |request: &Request| {
            if request.cmd == Command::Version {
                Ok(answer(&version))
            } else {
                Ok(answer(&json!({"id": "P1"})))
            }
        });
        let env = own.map_or_else(MapEnv::new, |own| MapEnv::new().with("NGTERM_SESSION_ID", own));
        let argv: Vec<&str> = argv.iter().copied().chain(["--no-block"]).collect();
        assert!(open(&argv).execute(b"a\n", &mut io, &env).is_ok());
        io.requests
    }

    /// A pane's own untargeted pick binds to its session only on `ngterm-headless`, which has no window to show it in;
    /// an app keeps the window pick, and `--session` or a caller outside a pane never asks the server.
    #[test]
    fn an_untargeted_pick_binds_to_the_panes_session_only_on_a_headless_server() {
        let headless = json!({"app": {"version": "1"}, "headless": true});
        let app = json!({"app": {"version": "1"}});
        let sent = bound_requests(&[], &headless, Some("S1"));
        assert_eq!(sent[0], json!({"cmd": "version"}));
        assert_eq!(sent[1]["target"], json!("S1"));
        let sent = bound_requests(&[], &app, Some("S1"));
        assert_eq!(sent.len(), 2);
        assert_eq!(sent[1].get("target"), None);
        let sent = bound_requests(&["--session", "S2"], &headless, Some("S1"));
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["target"], json!("S2"));
        for own in [None, Some("")] {
            let sent = bound_requests(&[], &headless, own);
            assert_eq!(sent.len(), 1, "{own:?}");
            assert_eq!(sent[0].get("target"), None);
        }
    }
}
