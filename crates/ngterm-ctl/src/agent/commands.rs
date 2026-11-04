//! `ngtermctl agent commands`: the slash commands and skills of a pane's agent, for the Android chat composer's `/`
//! dropdown, which runs it on the origin under its ssh once per conversation. It reads the pane's record
//! (`agent.get`) and lists what the agent's config dir and project hold ([`catalog`]), the config dir taken from the
//! record's transcript so the agent's own `CLAUDE_CONFIG_DIR` or `CODEX_HOME` holds.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use ngterm_core::agent_chat::catalog::catalog;
use ngterm_core::agent_chat::{AgentKind, NO_AGENT};
use ngterm_core::paths::Env;

use crate::{Failure, TargetOptions, client, socket};

/// `ngtermctl agent commands`: the pane's agent's slash commands and skills, as JSON.
#[derive(Debug, clap::Args)]
#[command(
    about = "Print the slash commands and skills of a pane's agent, as JSON.",
    long_about = "Reads the pane's agent record and prints the agent's built-in slash commands, its user and project \
                  skills and custom commands, and its enabled plugins' ones, as one JSON array of {name, \
                  description, kind, argument_hint}. A Codex skill is named as Codex mentions it ($name)."
)]
pub struct Commands {
    #[arg(long, help = "Which pane the agent runs in: left or right. Defaults to left.")]
    pane: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[arg(long, help = "Override the control socket path.")]
    socket: Option<PathBuf>,
}

impl Commands {
    pub(super) fn validate(&self) -> Result<(), String> {
        super::validate_pane_side(self.pane.as_deref())
    }

    pub(super) fn run(&self, env: &dyn Env) -> Result<(), Failure> {
        let socket = socket::resolve(self.socket.as_deref(), env);
        let Some(record) = super::look(&socket, &self.target.target, self.pane.as_deref())? else {
            return Err(Failure::Local(NO_AGENT.to_owned()));
        };
        let agent = AgentKind::from_name(&record.agent)
            .ok_or_else(|| Failure::Local(format!("unknown agent: {}", record.agent)))?;
        let commands = catalog(agent, Path::new(&record.transcript_path)).map_err(Failure::Local)?;
        // a `SlashCommand` holds only strings and enums: serializing it cannot fail
        let json = serde_json::to_string(&commands).unwrap_or_default();
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "{json}")
            .and_then(|()| stdout.flush())
            .map_err(|error| Failure::Local(format!("could not write the catalog: {}", client::os_message(&error))))
    }
}
