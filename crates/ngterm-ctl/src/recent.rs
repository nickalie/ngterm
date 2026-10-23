//! `ngtermctl recent list|open|clear` (`agtermctlKit/RecentCommands.swift` `Recent`): recently closed sessions and
//! workspaces. The subcommands send their requests; their arguments and output are not ported yet.

use ngterm_proto::command::RecentCmd;
use ngterm_proto::{Command, Request};

use crate::{BasicOptions, RequestCommand};

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

/// A `recent.*` request with no target and no args.
fn recent_request(cmd: RecentCmd) -> Request {
    Request::new(Command::Recent(cmd))
}

/// Declares a `recent` subcommand taking only [`BasicOptions`] and sending `$cmd`.
macro_rules! recent_command {
    ($(#[$meta:meta])* $name:ident, $cmd:expr, $about:literal) => {
        $(#[$meta])*
        #[derive(Debug, clap::Args)]
        #[command(about = $about)]
        pub struct $name {
            #[command(flatten)]
            options: BasicOptions,
        }

        impl RequestCommand for $name {
            fn options(&self) -> &BasicOptions {
                &self.options
            }

            fn make_request(&self) -> Request {
                recent_request($cmd)
            }
        }
    };
}

recent_command!(
    /// `ngtermctl recent list`.
    List,
    RecentCmd::List,
    "List recently closed sessions and workspaces, newest first."
);
recent_command!(
    /// `ngtermctl recent open`.
    Open,
    RecentCmd::Open,
    "Reopen a recently closed session or workspace (the newest when no id is given)."
);
recent_command!(
    /// `ngtermctl recent clear`.
    Clear,
    RecentCmd::Clear,
    "Empty the recently closed list."
);

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::Ngtermctl;

    #[test]
    fn each_subcommand_sends_its_wire_name() {
        for (sub, wire) in [
            ("list", "recent.list"),
            ("open", "recent.open"),
            ("clear", "recent.clear"),
        ] {
            let parsed = Ngtermctl::try_parse_from(["ngtermctl", "recent", sub]).unwrap();
            let crate::Command::Recent(group) = parsed.command else {
                panic!("not a recent command: {sub}");
            };
            let request = super::RequestCommand::make_request(group.command());
            assert_eq!(request.cmd.wire_name(), wire);
            assert_eq!((request.target, request.args), (None, None));
        }
    }
}
