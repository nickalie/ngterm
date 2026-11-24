//! `ngtermctl catalog ...` (`agtermctlKit/CatalogCommands.swift` `Catalog`): the headless server's catalog stream and
//! what became of an operation id, and [`OperationOptions`], the operation id a structural command carries. Only
//! `ngterm-headless` serves these; an app with windows refuses them.

use ngterm_core::paths::Env;
use ngterm_core::persist::parse_uuid_string;
use ngterm_proto::command::CatalogCmd;
use ngterm_proto::{Args, CatalogReceipt, Command, Request};

use crate::{BasicOptions, Failure, RequestCommand};

/// A structural command's operation id and the catalog epoch it was decided at (upstream's `OperationOptions`), so
/// the server runs it at most once and answers with a receipt.
#[derive(Debug, clap::Args)]
pub struct OperationOptions {
    #[arg(
        long = "op-id",
        help = "Operation id ngterm-headless keeps a receipt under; needs --epoch."
    )]
    op_id: Option<String>,

    #[arg(long, help = "The catalog epoch the operation was decided at; needs --op-id.")]
    epoch: Option<String>,

    #[arg(
        long = "expect-panes",
        help = "Comma-separated pane identities the target session must still have; needs --op-id."
    )]
    expect_panes: Option<String>,
}

impl OperationOptions {
    /// Whether an operation id was given.
    pub const fn is_set(&self) -> bool {
        self.op_id.is_some()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.op_id.is_some() != self.epoch.is_some() {
            return Err("--op-id and --epoch go together".to_owned());
        }
        if self
            .op_id
            .as_deref()
            .is_some_and(|op_id| !CatalogReceipt::is_valid_op_id(op_id))
        {
            return Err("--op-id takes 1 to 128 letters, digits, '.', '_', ':' or '-'".to_owned());
        }
        let Some(expected) = &self.expect_panes else {
            return Ok(());
        };
        if self.op_id.is_none() {
            return Err("--expect-panes needs --op-id".to_owned());
        }
        if panes(expected).iter().any(|pane| parse_uuid_string(pane).is_none()) {
            return Err("--expect-panes takes pane identities separated by commas".to_owned());
        }
        Ok(())
    }

    /// `request` with the operation id, its epoch and the expected panes; unchanged when no id was given.
    pub fn stamp(&self, mut request: Request) -> Request {
        let Some(op_id) = &self.op_id else {
            return request;
        };
        let args = request.args.get_or_insert_with(Args::default);
        args.op_id = Some(op_id.clone());
        args.epoch.clone_from(&self.epoch);
        args.panes = self.expect_panes.as_deref().map(panes);
        request
    }
}

/// A comma-separated list's members, each trimmed; an empty member stays, so it fails the identity check.
fn panes(list: &str) -> Vec<String> {
    list.split(',').map(|pane| pane.trim().to_owned()).collect()
}

/// `ngtermctl catalog <subcommand>`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Carry ngterm-headless's catalog to a remote window on a Mac.",
    long_about = "ngterm-headless keeps one catalog of workspaces, sessions and panes; a remote window on a Mac \
                  mirrors it. Only ngterm-headless serves these commands. A Mac app refuses them.",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Catalog {
    #[command(subcommand)]
    command: CatalogCommand,
}

/// The subcommands, in upstream's `Catalog.configuration.subcommands` order.
#[derive(Debug, clap::Subcommand)]
enum CatalogCommand {
    Subscribe(Subscribe),
    Op(Operation),
}

impl Catalog {
    fn command(&self) -> &dyn RequestCommand {
        match &self.command {
            CatalogCommand::Subscribe(command) => command,
            CatalogCommand::Op(command) => command,
        }
    }

    /// What the parser cannot check, as a usage error's message.
    pub fn check(&self) -> Result<(), String> {
        self.command().validate()
    }

    /// Runs the command.
    pub fn execute(&self, env: &dyn Env) -> Result<(), Failure> {
        self.command().invoke(env)
    }
}

/// `ngtermctl catalog op OPID [--epoch E --rev N]`.
#[derive(Debug, clap::Args)]
#[command(
    name = "op",
    about = "Ask ngterm-headless what became of an operation id.",
    long_about = "Answers from ngterm-headless's operation table: applied (with the revision it reached), no change, \
                  refused, fenced, or unknown. An id it does not hold is recorded so that command is refused if it \
                  arrives later: fenced, or unknown when the table may have dropped its entry since --epoch and --rev."
)]
pub struct Operation {
    #[arg(help = "The operation id a structural command was sent with.")]
    op_id: String,

    #[arg(long, help = "The catalog epoch the operation was sent at; needs --rev.")]
    epoch: Option<String>,

    #[arg(long, help = "The catalog revision the operation was sent at; needs --epoch.")]
    rev: Option<i64>,

    #[command(flatten)]
    options: BasicOptions,
}

impl RequestCommand for Operation {
    fn options(&self) -> &BasicOptions {
        &self.options
    }

    fn validate(&self) -> Result<(), String> {
        if !CatalogReceipt::is_valid_op_id(&self.op_id) {
            return Err("an operation id is 1 to 128 letters, digits, '.', '_', ':' or '-'".to_owned());
        }
        if self.epoch.is_some() != self.rev.is_some() {
            return Err("--epoch and --rev go together".to_owned());
        }
        Ok(())
    }

    fn make_request(&self) -> Request {
        let args = self.epoch.as_ref().map(|epoch| Args {
            epoch: Some(epoch.clone()),
            rev: self.rev,
            ..Args::default()
        });
        Request {
            cmd: Command::Catalog(CatalogCmd::Op),
            target: Some(self.op_id.clone()),
            args,
        }
    }
}

/// `ngtermctl catalog subscribe`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Carry the catalog stream between stdin/stdout and ngterm-headless.",
    long_about = "Run by a Mac's remote window over ssh. Stdout carries newline-delimited JSON frames and nothing \
                  else: the snapshot, then every change as a numbered delta. Stdin takes the other side's frames. It \
                  is not meant to be run by hand. A refusal exits nonzero with the reason on stderr."
)]
pub struct Subscribe {
    #[command(flatten)]
    options: BasicOptions,
}

impl RequestCommand for Subscribe {
    fn options(&self) -> &BasicOptions {
        &self.options
    }

    fn make_request(&self) -> Request {
        Request::new(Command::Catalog(CatalogCmd::Subscribe))
    }

    /// Opens the stream and bridges it to stdin/stdout until either side ends (upstream's `Subscribe.run`).
    fn invoke(&self, env: &dyn Env) -> Result<(), Failure> {
        let socket = crate::client::connect(&self.options.socket_path(env))?;
        let bridge = crate::stream_bridge::StreamBridge::over_stdio(socket).map_err(Failure::Local)?;
        bridge.open(&self.make_request()).map_err(Failure::Local)?;
        bridge.pump();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use serde_json::{Value, json};

    use super::*;
    use crate::Ngtermctl;

    fn outcome(argv: &[&str]) -> Result<Value, String> {
        let command_line = ["ngtermctl", "catalog"].iter().chain(argv).copied();
        let crate::Command::Catalog(group) = Ngtermctl::try_parse_from(command_line).unwrap().command else {
            panic!("not a catalog command")
        };
        group.check()?;
        Ok(serde_json::to_value(group.command().make_request()).unwrap())
    }

    /// Upstream's `CatalogCommandsTests`: the id is the target, and the position rides only when given.
    #[test]
    fn op_sends_its_id_as_the_target_and_the_position_only_when_given() {
        assert_eq!(
            outcome(&["op", "op-1"]),
            Ok(json!({"cmd": "catalog.op", "target": "op-1"}))
        );
        assert_eq!(
            outcome(&["op", "a.b_c:d-1", "--epoch", "E", "--rev", "7"]),
            Ok(json!({"cmd": "catalog.op", "target": "a.b_c:d-1", "args": {"epoch": "E", "rev": 7}}))
        );
    }

    #[test]
    fn op_refuses_a_malformed_id_and_half_a_position() {
        let id = "an operation id is 1 to 128 letters, digits, '.', '_', ':' or '-'";
        let long = "a".repeat(129);
        for bad in ["", "a b", "a/b", "é", long.as_str()] {
            assert_eq!(outcome(&["op", bad]), Err(id.to_owned()), "{bad:?}");
        }
        assert_eq!(outcome(&["op", &"a".repeat(128)]).map(drop), Ok(()));
        for half in [&["--epoch", "E"][..], &["--rev", "1"]] {
            let argv = [&["op", "x"][..], half].concat();
            assert_eq!(outcome(&argv), Err("--epoch and --rev go together".to_owned()));
        }
    }

    #[test]
    fn subscribe_sends_the_bare_command() {
        assert_eq!(outcome(&["subscribe"]), Ok(json!({"cmd": "catalog.subscribe"})));
    }

    #[derive(Debug, Parser)]
    struct Stamped {
        #[command(flatten)]
        operation: OperationOptions,
    }

    fn operation(argv: &[&str]) -> OperationOptions {
        Stamped::try_parse_from(std::iter::once("t").chain(argv.iter().copied()))
            .unwrap()
            .operation
    }

    const PANE: &str = "0B6F5F5E-9A0C-4E0B-8B0A-0A1B2C3D4E5F";

    /// Upstream's `OperationOptions.validate` and `stamp`.
    #[test]
    fn operation_options_go_together_and_stamp_the_request() {
        let plain = Request::new(Command::Tree);
        let none = operation(&[]);
        assert_eq!((none.validate(), none.is_set()), (Ok(()), false));
        assert_eq!(none.stamp(plain.clone()), plain);

        let panes = format!("{PANE}, {PANE}");
        let full = operation(&["--op-id", "op-1", "--epoch", "E", "--expect-panes", &panes]);
        assert_eq!((full.validate(), full.is_set()), (Ok(()), true));
        assert_eq!(
            serde_json::to_value(full.stamp(plain.clone())).unwrap(),
            json!({"cmd": "tree", "args": {"opID": "op-1", "epoch": "E", "panes": [PANE, PANE]}})
        );
        let bare = operation(&["--op-id", "op-1", "--epoch", "E"]);
        assert_eq!(
            serde_json::to_value(bare.stamp(plain)).unwrap(),
            json!({"cmd": "tree", "args": {"opID": "op-1", "epoch": "E"}})
        );

        // upstream's `UUID(uuidString:)` takes only the hyphenated form
        let simple = PANE.replace('-', "");
        let braced = format!("{{{PANE}}}");
        let refusals: [(&[&str], &str); 8] = [
            (&["--op-id", "a"], "--op-id and --epoch go together"),
            (&["--epoch", "E"], "--op-id and --epoch go together"),
            (
                &["--op-id", "a b", "--epoch", "E"],
                "--op-id takes 1 to 128 letters, digits, '.', '_', ':' or '-'",
            ),
            (&["--expect-panes", PANE], "--expect-panes needs --op-id"),
            (
                &["--op-id", "a", "--epoch", "E", "--expect-panes", ""],
                "--expect-panes takes pane identities separated by commas",
            ),
            (
                &["--op-id", "a", "--epoch", "E", "--expect-panes", "x,"],
                "--expect-panes takes pane identities separated by commas",
            ),
            (
                &["--op-id", "a", "--epoch", "E", "--expect-panes", &simple],
                "--expect-panes takes pane identities separated by commas",
            ),
            (
                &["--op-id", "a", "--epoch", "E", "--expect-panes", &braced],
                "--expect-panes takes pane identities separated by commas",
            ),
        ];
        for (argv, message) in refusals {
            assert_eq!(operation(argv).validate(), Err(message.to_owned()), "{argv:?}");
        }
    }
}
