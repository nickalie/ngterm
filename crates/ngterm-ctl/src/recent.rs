//! `ngtermctl recent list|open|clear` (`agtermctlKit/RecentCommands.swift` `Recent`, the output from
//! `SocketClient.swift`'s `formatRecent` and its `recentOpen` branch): recently closed sessions and workspaces
//! (File > Open Recent).

use ngterm_core::NO_RECENT_ITEMS;
use ngterm_proto::command::RecentCmd;
use ngterm_proto::{Args, Command, ControlRecentItem, ControlRecentOpen, Request};

use crate::{BasicOptions, ClientOptions, RequestCommand};

/// `ngtermctl recent <subcommand>`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Recently closed sessions and workspaces (File > Open Recent).",
    subcommand_required = true,
    arg_required_else_help = true
)]
pub struct Recent {
    #[command(subcommand)]
    command: RecentCommand,
}

/// The subcommands, in upstream's `Recent.configuration.subcommands` order.
#[derive(Debug, clap::Subcommand)]
enum RecentCommand {
    List(List),
    Open(Open),
    Clear(Clear),
}

impl Recent {
    /// The subcommand this invocation runs.
    fn command(&self) -> &dyn RequestCommand {
        match &self.command {
            RecentCommand::List(command) => command,
            RecentCommand::Open(command) => command,
            RecentCommand::Clear(command) => command,
        }
    }

    /// What the parser cannot check, as a usage error's message.
    pub fn check(&self) -> Result<(), String> {
        self.command().validate()
    }

    /// Runs the command.
    pub fn execute(&self, env: &dyn ngterm_core::paths::Env) -> Result<(), crate::Failure> {
        self.command().default_run(env)
    }
}

/// A `recent.*` request.
fn recent_request(cmd: RecentCmd, target: Option<String>, args: Option<Args>) -> Request {
    Request {
        target,
        args,
        ..Request::new(Command::Recent(cmd))
    }
}

/// `ngtermctl recent list`.
#[derive(Debug, clap::Args)]
#[command(
    about = "List recently closed sessions and workspaces, newest first.",
    long_about = "One row per entry: its id, kind, title and subtitle. The list keeps the last 20 entries and drops \
                  the oldest silently; --json adds the close time and the session ids each entry restores."
)]
pub struct List {
    #[command(flatten)]
    options: BasicOptions,
}

impl RequestCommand for List {
    fn options(&self) -> &BasicOptions {
        &self.options
    }

    fn make_request(&self) -> Request {
        recent_request(RecentCmd::List, None, None)
    }
}

/// `ngtermctl recent open [ID] [--window W]`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Reopen a recently closed session or workspace (the newest when no id is given).",
    long_about = "The entry comes back with its id, split, flag, context and restore pins. It goes to --window or the \
                  frontmost window, unless another window already holds its session or workspace; --json names the \
                  window it went to. The window is not raised. An entry whose restore was incomplete stays in the \
                  list."
)]
pub struct Open {
    #[arg(help = "Entry id or unique prefix, from 'recent list'.")]
    id: Option<String>,

    #[command(flatten)]
    options: ClientOptions,
}

impl RequestCommand for Open {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    fn make_request(&self) -> Request {
        recent_request(RecentCmd::Open, self.id.clone(), self.options.with_window(None))
    }
}

/// `ngtermctl recent clear`.
#[derive(Debug, clap::Args)]
#[command(about = "Empty the recently closed list.")]
pub struct Clear {
    #[command(flatten)]
    options: BasicOptions,
}

impl RequestCommand for Clear {
    fn options(&self) -> &BasicOptions {
        &self.options
    }

    fn make_request(&self) -> Request {
        recent_request(RecentCmd::Clear, None, None)
    }
}

/// `recent.list`: one entry per row with its full id, which `recent open` takes as is (upstream's `formatRecent`).
pub(crate) fn format_recent(items: &[ControlRecentItem]) -> String {
    if items.is_empty() {
        return NO_RECENT_ITEMS.to_owned();
    }
    items
        .iter()
        .map(|item| {
            let subtitle = item
                .subtitle
                .as_ref()
                .map(|subtitle| format!(" ({subtitle})"))
                .unwrap_or_default();
            format!("{}  {}  {}{subtitle}", item.id, item.kind, item.title)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// `recent.open`: what came back, and whether its entry stays in the list.
pub(crate) fn format_recent_open(open: &ControlRecentOpen) -> String {
    let reopened = format!("reopened {} {}", open.item.kind, open.item.title);
    if open.complete {
        reopened
    } else {
        format!("{reopened} (incomplete: the entry stays in the list)")
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use ngterm_proto::{Args, Command, Request};

    use super::*;
    use crate::Ngtermctl;

    fn request(argv: &[&str]) -> Request {
        let parsed = Ngtermctl::try_parse_from(["ngtermctl"].iter().chain(argv)).unwrap();
        let crate::Command::Recent(group) = parsed.command else {
            panic!("not a recent command: {argv:?}");
        };
        RequestCommand::make_request(group.command())
    }

    #[test]
    fn list_and_clear_carry_no_args() {
        assert_eq!(
            request(&["recent", "list"]),
            Request::new(Command::Recent(RecentCmd::List))
        );
        assert_eq!(
            request(&["recent", "clear"]),
            Request::new(Command::Recent(RecentCmd::Clear))
        );
    }

    #[test]
    fn list_and_clear_reject_a_window_selector() {
        for sub in ["list", "clear"] {
            assert!(
                Ngtermctl::try_parse_from(["ngtermctl", "recent", sub, "--window", "w1"]).is_err(),
                "{sub}"
            );
        }
    }

    #[test]
    fn open_without_an_id_asks_for_the_newest_entry() {
        assert_eq!(
            request(&["recent", "open"]),
            Request::new(Command::Recent(RecentCmd::Open))
        );
    }

    #[test]
    fn open_carries_the_id_and_window() {
        assert_eq!(
            request(&["recent", "open", "ab12", "--window", "w1"]),
            Request {
                target: Some("ab12".to_owned()),
                args: Some(Args {
                    window: Some("w1".to_owned()),
                    ..Args::default()
                }),
                ..Request::new(Command::Recent(RecentCmd::Open))
            }
        );
    }

    fn item(id: &str, kind: &str, title: &str, subtitle: Option<&str>) -> ControlRecentItem {
        ControlRecentItem {
            id: id.to_owned(),
            kind: kind.to_owned(),
            title: title.to_owned(),
            subtitle: subtitle.map(str::to_owned),
            closed_at: 1.0,
            workspace_id: "W1".to_owned(),
            session_ids: vec!["S1".to_owned()],
        }
    }

    #[test]
    fn formats_recent_rows_with_full_ids_and_an_empty_list() {
        let rows = [
            item("R1", "session", "api", Some("project")),
            item("R2", "workspace", "project", None),
        ];
        assert_eq!(
            format_recent(&rows),
            "R1  session  api (project)\nR2  workspace  project"
        );
        assert_eq!(format_recent(&[]), "no recently closed items");
    }

    #[test]
    fn formats_recent_open_and_says_when_the_entry_stays() {
        let open = |complete| ControlRecentOpen {
            item: item("R1", "session", "api", None),
            window: "WIN".to_owned(),
            complete,
        };
        assert_eq!(format_recent_open(&open(true)), "reopened session api");
        assert_eq!(
            format_recent_open(&open(false)),
            "reopened session api (incomplete: the entry stays in the list)"
        );
    }
}
