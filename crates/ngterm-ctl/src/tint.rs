//! `ngtermctl session tint` and `workspace tint` (`agtermctlKit/TintCommands.swift`), and the creation tint
//! (`TintCreateOptions`) `session new` and `workspace new` share. The CLI spelling's one argument is a color or
//! `clear`; on the wire that is `mode` `set` with `tint`/`tintOn`, or `mode` `clear`. A bad color or placement is
//! refused before anything is sent, in upstream's words; the server checks them again for a raw socket client.

use ngterm_core::model::{TintColor, TintParts};
use ngterm_proto::command::{SessionCmd, WorkspaceCmd};
use ngterm_proto::{Args, Command, Request};

use crate::{BasicOptions, ClientOptions, RequestCommand, TargetOptions};

/// `--on`'s parts as the wire's list: split on every comma, empty parts kept so `validate_parts` refuses them, as
/// upstream's `TintRequest.parts`.
fn parts(on: Option<&str>) -> Option<Vec<String>> {
    on.map(|on| on.split(',').map(str::to_owned).collect())
}

/// Checks `<COLOR|clear> [--on PARTS]` (upstream's `TintRequest.validate`).
fn validate(value: &str, on: Option<&str>) -> Result<(), String> {
    if value == "clear" {
        return match on {
            Some(_) => Err("--on does not apply to clear".to_owned()),
            None => Ok(()),
        };
    }
    if TintColor::from_wire(&value.to_lowercase()).is_none() {
        return Err(format!(
            "tint must be one of: {}, or clear",
            TintColor::valid_names_list()
        ));
    }
    validate_parts(on, "--on")
}

/// Checks a placement list: `icon` and/or `name`, comma-separated (upstream's `TintRequest.validateParts`).
fn validate_parts(on: Option<&str>, flag: &str) -> Result<(), String> {
    match parts(on) {
        Some(parts) if TintParts::parse(&parts).is_none() => {
            Err(format!("{flag} must list icon and/or name, comma-separated"))
        }
        _ => Ok(()),
    }
}

/// The wire form of `<COLOR|clear> [--on PARTS]` (upstream's `TintRequest.args`): the color lower-cased.
fn tint_args(value: &str, on: Option<&str>) -> Args {
    if value == "clear" {
        Args {
            mode: Some("clear".to_owned()),
            ..Args::default()
        }
    } else {
        Args {
            mode: Some("set".to_owned()),
            tint: Some(value.to_lowercase()),
            tint_on: parts(on),
            ..Args::default()
        }
    }
}

/// The creation tint `session new` and `workspace new` take (upstream's `TintCreateOptions`).
#[derive(Debug, clap::Args)]
pub struct TintCreateOptions {
    #[arg(
        long,
        help = "Create it tinted this color (red|orange|yellow|green|blue|purple|gray)."
    )]
    tint: Option<String>,

    #[arg(long = "tint-on", help = "With --tint: icon, name, or icon,name (default icon).")]
    tint_on: Option<String>,
}

impl TintCreateOptions {
    /// Checks the creation tint (upstream's `TintCreateOptions.validate`): a placement needs a color.
    pub fn validate(&self) -> Result<(), String> {
        let Some(tint) = &self.tint else {
            return match self.tint_on {
                Some(_) => Err("--tint-on requires --tint".to_owned()),
                None => Ok(()),
            };
        };
        if TintColor::from_wire(&tint.to_lowercase()).is_none() {
            return Err(format!("--tint must be one of: {}", TintColor::valid_names_list()));
        }
        validate_parts(self.tint_on.as_deref(), "--tint-on")
    }

    /// Copies the creation tint into a `session.new`/`workspace.new` argument bag, the color lower-cased.
    pub fn apply(&self, args: &mut Args) {
        args.tint = self.tint.as_ref().map(|tint| tint.to_lowercase());
        args.tint_on = parts(self.tint_on.as_deref());
    }
}

const COLOR_HELP: &str = "A color (red|orange|yellow|green|blue|purple|gray) or 'clear'.";
const ON_HELP: &str =
    "Where the color shows: icon, name, or icon,name. Omitted keeps the current placement, else icon.";

/// `ngtermctl session tint <COLOR|clear> [--on PARTS]`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Tint a session's sidebar row (a color, or clear).",
    long_about = "session tint red                 tint the icon red (or recolor, keeping the placement)\nsession tint \
                  green --on name     color the name instead\nsession tint blue --on icon,name color both\nsession \
                  tint clear               remove the tint\n\nPersists across a relaunch; read back as the tree's \
                  tint and tintOn."
)]
pub struct SessionTint {
    #[arg(help = COLOR_HELP)]
    value: String,

    #[arg(long, help = ON_HELP)]
    on: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

impl RequestCommand for SessionTint {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    fn validate(&self) -> Result<(), String> {
        validate(&self.value, self.on.as_deref())
    }

    fn make_request(&self) -> Request {
        Request {
            cmd: Command::Session(SessionCmd::Tint),
            target: Some(self.target.target.clone()),
            args: self
                .options
                .with_window(Some(tint_args(&self.value, self.on.as_deref()))),
        }
    }
}

/// `ngtermctl workspace tint <COLOR|clear> [--on PARTS]`: the workspace row's own tint; its sessions keep theirs.
#[derive(Debug, clap::Args)]
#[command(
    about = "Tint a workspace's own sidebar row (a color, or clear); its sessions keep their own.",
    long_about = "workspace tint red --on icon,name\nworkspace tint clear\n\nPersists across a relaunch; read back as \
                  the workspace node's tint and tintOn."
)]
pub struct WorkspaceTint {
    #[arg(help = COLOR_HELP)]
    value: String,

    #[arg(long, help = ON_HELP)]
    on: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

impl RequestCommand for WorkspaceTint {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    fn validate(&self) -> Result<(), String> {
        validate(&self.value, self.on.as_deref())
    }

    fn make_request(&self) -> Request {
        Request {
            cmd: Command::Workspace(WorkspaceCmd::Tint),
            target: Some(self.target.target.clone()),
            args: self
                .options
                .with_window(Some(tint_args(&self.value, self.on.as_deref()))),
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use serde_json::{Value, json};

    use crate::Ngtermctl;

    /// The request `ngtermctl <argv>` sends, as JSON, or the refusal its validation answers.
    fn outcome(argv: &[&str]) -> Result<Value, String> {
        let command_line = std::iter::once("ngtermctl").chain(argv.iter().copied());
        let command = Ngtermctl::try_parse_from(command_line).unwrap().command;
        command.validate()?;
        let request = match command {
            crate::Command::Session(group) => serde_json::to_value(group.command().make_request()),
            crate::Command::Workspace(group) => serde_json::to_value(group.command().make_request()),
            other => panic!("not a session or workspace command: {other:?}"),
        };
        Ok(request.unwrap())
    }

    fn request(argv: &[&str]) -> Value {
        outcome(argv).unwrap()
    }

    /// `tintRejectsBadInputAtParseTime`.
    #[test]
    fn a_bad_tint_is_refused_before_anything_is_sent() {
        for (argv, error) in [
            (
                &["session", "tint", "teal"][..],
                "tint must be one of: red|orange|yellow|green|blue|purple|gray, or clear",
            ),
            (
                &["session", "tint", "clear", "--on", "icon"],
                "--on does not apply to clear",
            ),
            (
                &["session", "tint", "red", "--on", "icon,strip"],
                "--on must list icon and/or name, comma-separated",
            ),
            (
                &["workspace", "tint", "red", "--on", ""],
                "--on must list icon and/or name, comma-separated",
            ),
        ] {
            assert_eq!(outcome(argv), Err(error.to_owned()), "{argv:?}");
        }
    }

    /// `creationTintRejectsBadInputAtParseTime`, ahead of `session new`'s own checks.
    #[test]
    fn a_bad_creation_tint_is_refused_before_anything_is_sent() {
        for (argv, error) in [
            (
                &["session", "new", "--tint-on", "icon"][..],
                "--tint-on requires --tint",
            ),
            (
                &["workspace", "new", "--tint", "teal"],
                "--tint must be one of: red|orange|yellow|green|blue|purple|gray",
            ),
            (
                &["workspace", "new", "--tint", "red", "--tint-on", "both"],
                "--tint-on must list icon and/or name, comma-separated",
            ),
            (
                &["session", "new", "--wait", "--tint", "teal"],
                "--tint must be one of: red|orange|yellow|green|blue|purple|gray",
            ),
        ] {
            assert_eq!(outcome(argv), Err(error.to_owned()), "{argv:?}");
        }
        assert_eq!(
            request(&["workspace", "new", "prod", "--tint", "red"]),
            json!({"cmd": "workspace.new", "args": {"name": "prod", "tint": "red"}})
        );
    }

    #[test]
    fn tint_sends_set_with_the_lowercased_color_or_clear() {
        for (group, cmd) in [("session", "session.tint"), ("workspace", "workspace.tint")] {
            assert_eq!(
                request(&[group, "tint", "Red"]),
                json!({"cmd": cmd, "target": "active", "args": {"mode": "set", "tint": "red"}}),
                "{group}"
            );
            assert_eq!(
                request(&[
                    group,
                    "tint",
                    "blue",
                    "--on",
                    "icon,name",
                    "--target",
                    "t1",
                    "--window",
                    "w1"
                ]),
                json!({"cmd": cmd, "target": "t1",
                       "args": {"mode": "set", "tint": "blue", "tintOn": ["icon", "name"], "window": "w1"}}),
                "{group}"
            );
            assert_eq!(
                request(&[group, "tint", "clear"]),
                json!({"cmd": cmd, "target": "active", "args": {"mode": "clear"}}),
                "{group}"
            );
        }
    }

    #[test]
    fn the_creation_tint_rides_on_both_news() {
        assert_eq!(
            request(&["session", "new", "--tint", "Green", "--tint-on", "icon,name"]),
            json!({"cmd": "session.new", "args": {"tint": "green", "tintOn": ["icon", "name"]}})
        );
        assert_eq!(
            request(&["workspace", "new", "w", "--tint", "gray"]),
            json!({"cmd": "workspace.new", "args": {"name": "w", "tint": "gray"}})
        );
    }
}
