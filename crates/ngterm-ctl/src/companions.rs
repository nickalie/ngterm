//! The companion requests' run (upstream's `runCompanionRequest` and `ControlResponse.refusesUnknown` in
//! `agtermctlKit/Session+Companions.swift` and `ControlResponse.swift`): an app predating companions cannot decode the
//! wire command and refuses it as an invalid request, which the human output names as an app too old for companions.
//! `--json` keeps the raw reply.

use std::process::ExitCode;

use ngterm_core::paths::Env;
use ngterm_proto::{Command, Response};

use crate::{Failure, RequestCommand, client, output};

/// What a human run prints on stderr for an app that does not know a companion command.
pub const TOO_OLD: &str = "error: the running app is too old for companions";

/// Whether `response` is the refusal an app predating `command` gives: it could not decode the command, so it refused
/// the request before doing anything. Every other error is the server's own.
#[must_use]
pub fn refuses_unknown(response: &Response, command: Command) -> bool {
    !response.ok
        && response
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("invalid request:") && error.contains(command.wire_name()))
}

/// Sends a companion request once and prints the reply as [`RequestCommand::default_run`] does, except that an app
/// too old for it is named, exit 1, outside `--json`.
pub fn run(command: &(impl RequestCommand + ?Sized), env: &dyn Env) -> Result<(), Failure> {
    let options = command.options();
    let request = command.run_request()?;
    let reply = client::send(&options.socket_path(env), &request)?;
    if !options.json && refuses_unknown(&reply.response, request.cmd) {
        eprintln!("{TOO_OLD}");
        return Err(Failure::Exit(ExitCode::FAILURE));
    }
    output::print_response(&reply, options.json, command.echoes_result_id());
    if reply.response.ok {
        Ok(())
    } else {
        Err(Failure::Exit(ExitCode::FAILURE))
    }
}

#[cfg(test)]
mod tests {
    use ngterm_proto::command::SessionCmd;

    use super::*;

    /// Upstream's `anUndecodableCommandIsTheOnlyRefusalOfAnUnknownCommand`.
    #[test]
    fn an_undecodable_command_is_the_only_refusal_of_an_unknown_command() {
        let companion = Command::Session(SessionCmd::CompanionNew);
        let decode = "invalid request: Cannot initialize Command from invalid String value session.companion.new";
        assert!(refuses_unknown(&Response::failure(decode), companion));
        for error in [
            "no session matching p1",
            "companions of a remote session are not supported yet",
            "invalid request: Cannot initialize Command from invalid String value session.nope",
        ] {
            assert!(!refuses_unknown(&Response::failure(error), companion), "{error}");
        }
        assert!(!refuses_unknown(&Response::success(), companion));
    }
}
