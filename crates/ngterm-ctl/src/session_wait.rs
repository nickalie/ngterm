//! `ngtermctl session wait` (`agtermctlKit/Session+Wait.swift` `Session.Wait`): a client-side composite over
//! `events.read`, `window.list` and `tree` that blocks until a session's agent status matches. Not ported yet: it
//! parses and fails with a local error, sending nothing.

use ngterm_core::paths::Env;
use ngterm_proto::command::EventsCmd;
use ngterm_proto::{Command, Request};

use crate::{BasicOptions, Failure, RequestCommand, TargetOptions};

/// What a run reports until the composite is ported.
pub const NOT_IMPLEMENTED: &str = "session wait is not implemented yet";

/// `ngtermctl session wait`.
#[derive(Debug, clap::Args)]
#[command(about = "Block until a session's agent status is one of the given values.")]
pub struct Wait {
    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: BasicOptions,
}

impl RequestCommand for Wait {
    fn options(&self) -> &BasicOptions {
        &self.options
    }

    /// The composite's first read, upstream's `events.read` that anchors the event cursor before `window.list` and
    /// `tree` read the current status. Never sent yet: `invoke` refuses before any request.
    fn make_request(&self) -> Request {
        Request::new(Command::Events(EventsCmd::Read))
    }

    fn invoke(&self, _env: &dyn Env) -> Result<(), Failure> {
        Err(Failure::Local(NOT_IMPLEMENTED.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use ngterm_core::paths::ProcessEnv;

    use super::NOT_IMPLEMENTED;
    use crate::{Failure, Ngtermctl};

    #[test]
    fn a_run_fails_locally_without_sending() {
        let parsed = Ngtermctl::try_parse_from(["ngtermctl", "session", "wait", "--socket", "/nonexistent"]).unwrap();
        let result = parsed.command.run(&ProcessEnv);
        assert!(matches!(result, Err(Failure::Local(message)) if message == NOT_IMPLEMENTED));
    }
}
