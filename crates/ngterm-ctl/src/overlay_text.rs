//! The text and log page side of `ngtermctl session overlay`: `follow` (`agtermctlKit/Session+OverlayFollow.swift`),
//! `search` (`Session+OverlaySearch.swift`) and `changes` (`Session+OverlayChanges.swift`). Each acts on the page in
//! one overlay slot, scoped to a pane with `--pane`, and prints what the server answers.

use ngterm_proto::command::SessionCmd;
use ngterm_proto::{Args, Request};

use crate::session::session_request;
use crate::session_hud::validate_overlay_pane;
use crate::{BasicOptions, ClientOptions, RequestCommand, TargetOptions};

/// `--pane`'s help on all three (upstream's own wording on each).
const PANE_HELP: &str = "The pane overlay holding the page (primary/left/top or split/right/bottom).";

/// `ngtermctl session overlay follow on|off|toggle`: the mode is required here; only the wire request defaults an
/// absent one to toggle.
#[derive(Debug, clap::Args)]
#[command(
    about = "Follow a log page's end, pause it, or toggle between the two.",
    long_about = "Acts on a page opened with --tail or --follow-file, or a clicked .log or .out file. Prints the state \
                  left behind, following or paused; tree reads it as htmlOverlays[].text.following."
)]
pub struct Follow {
    #[arg(help = "on, off or toggle.")]
    mode: String,

    #[arg(long, help = PANE_HELP)]
    pane: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

impl RequestCommand for Follow {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    fn validate(&self) -> Result<(), String> {
        if !["on", "off", "toggle"].contains(&self.mode.as_str()) {
            return Err("mode must be on, off or toggle".to_owned());
        }
        validate_overlay_pane(self.pane.as_deref())
    }

    fn make_request(&self) -> Request {
        let args = Args {
            mode: Some(self.mode.clone()),
            pane: self.pane.clone(),
            ..Args::default()
        };
        session_request(
            SessionCmd::OverlayFollow,
            Some(self.target.target.clone()),
            Some(args),
            &self.options,
        )
    }
}

/// `ngtermctl session overlay search [TEXT] [--next|--prev|--close]`: exactly one of a query and a step, which rides
/// the wire as `to`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Search a text or log page: set the query, step to the next or previous match, or close it.",
    long_about = "Not session search, which searches a terminal. A step prints the match it reached as N of M and \
                  pauses a log page that follows; tree reads the query and counts as htmlOverlays[].text.search."
)]
pub struct Search {
    #[arg(help = "Text to find, case-insensitive.")]
    text: Option<String>,

    #[arg(long, help = "Step to the next match.")]
    next: bool,

    #[arg(long, help = "Step to the previous match.")]
    prev: bool,

    #[arg(long, help = "Close the search.")]
    close: bool,

    #[arg(long, help = PANE_HELP)]
    pane: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

impl Search {
    /// The step's wire name, absent for a query.
    fn step(&self) -> Option<&'static str> {
        [(self.next, "next"), (self.prev, "prev"), (self.close, "close")]
            .into_iter()
            .find_map(|(set, name)| set.then_some(name))
    }
}

impl RequestCommand for Search {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    // upstream's order: the steps among themselves, then a step beside text, then neither, then an empty query
    fn validate(&self) -> Result<(), String> {
        let steps = [self.next, self.prev, self.close]
            .into_iter()
            .filter(|set| *set)
            .count();
        if steps > 1 {
            return Err("--next, --prev and --close are mutually exclusive".to_owned());
        }
        if steps == 1 && self.text.is_some() {
            return Err("text cannot be combined with --next, --prev or --close".to_owned());
        }
        if steps == 0 && self.text.is_none() {
            return Err("give text, --next, --prev or --close".to_owned());
        }
        if self.text.as_deref() == Some("") {
            return Err("text must not be empty".to_owned());
        }
        validate_overlay_pane(self.pane.as_deref())
    }

    fn make_request(&self) -> Request {
        let args = Args {
            text: self.text.clone(),
            pane: self.pane.clone(),
            to: self.step().map(str::to_owned),
            ..Args::default()
        };
        session_request(
            SessionCmd::OverlaySearch,
            Some(self.target.target.clone()),
            Some(args),
            &self.options,
        )
    }
}

/// `ngtermctl session overlay changes --next|--prev`: the step rides the wire as `to`.
#[derive(Debug, clap::Args)]
#[command(
    about = "Step a diff page to its next or previous change.",
    long_about = "A change is one run of removed and added lines. A step prints the change it reached as N of M; tree \
                  reads the count and the current change as htmlOverlays[].text.changes."
)]
pub struct Changes {
    #[arg(long, help = "Step to the next change.")]
    next: bool,

    #[arg(long, help = "Step to the previous change.")]
    prev: bool,

    #[arg(long, help = PANE_HELP)]
    pane: Option<String>,

    #[command(flatten)]
    target: TargetOptions,

    #[command(flatten)]
    options: ClientOptions,
}

impl RequestCommand for Changes {
    fn options(&self) -> &BasicOptions {
        &self.options.basic
    }

    fn validate(&self) -> Result<(), String> {
        if self.next == self.prev {
            return Err("give exactly one of --next or --prev".to_owned());
        }
        validate_overlay_pane(self.pane.as_deref())
    }

    fn make_request(&self) -> Request {
        let args = Args {
            pane: self.pane.clone(),
            to: Some(if self.next { "next" } else { "prev" }.to_owned()),
            ..Args::default()
        };
        session_request(
            SessionCmd::OverlayChanges,
            Some(self.target.target.clone()),
            Some(args),
            &self.options,
        )
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use clap::Parser;
    use serde_json::{Value, json};

    use crate::{Command as Cli, Ngtermctl};

    /// `ngtermctl session overlay <argv>` parsed and validated: the request it builds as JSON, or the refusal.
    fn outcome(argv: &[&str]) -> Result<Value, String> {
        let command_line: Vec<OsString> = ["ngtermctl", "session", "overlay"]
            .iter()
            .chain(argv)
            .map(OsString::from)
            .collect();
        let parsed = Ngtermctl::try_parse_from(command_line).map_err(|error| error.to_string())?;
        let Cli::Session(group) = parsed.command else {
            panic!("not a session command")
        };
        let command = group.command();
        command.validate()?;
        Ok(serde_json::to_value(command.make_request()).unwrap())
    }

    fn request(argv: &[&str]) -> Value {
        outcome(argv).unwrap()
    }

    /// Upstream's `followSendsItsModePaneAndTarget`.
    #[test]
    fn follow_sends_its_mode_pane_and_target() {
        for mode in ["on", "off", "toggle"] {
            assert_eq!(
                request(&["follow", mode, "--pane", "right", "--target", "s"]),
                json!({"cmd": "session.overlay.follow", "target": "s", "args": {"mode": mode, "pane": "right"}})
            );
        }
        assert_eq!(
            request(&["follow", "on"]),
            json!({"cmd": "session.overlay.follow", "target": "active", "args": {"mode": "on"}})
        );
    }

    /// Upstream's `followRefusesABadModeOrPane`.
    #[test]
    fn follow_refuses_a_bad_mode_or_pane() {
        assert_eq!(
            outcome(&["follow", "maybe"]),
            Err("mode must be on, off or toggle".to_owned())
        );
        assert!(outcome(&["follow"]).is_err(), "the mode is required");
        assert_eq!(
            outcome(&["follow", "on", "--pane", "scratch"]),
            Err("--pane must be left or right".to_owned())
        );
    }

    /// Upstream's `searchSendsItsQueryOrStepPaneAndTarget`.
    #[test]
    fn search_sends_its_query_or_step_pane_and_target() {
        assert_eq!(
            request(&["search", "main()", "--pane", "right", "--target", "s"]),
            json!({"cmd": "session.overlay.search", "target": "s", "args": {"text": "main()", "pane": "right"}})
        );
        for step in ["next", "prev", "close"] {
            assert_eq!(
                request(&["search", &format!("--{step}"), "--pane", "right", "--target", "s"]),
                json!({"cmd": "session.overlay.search", "target": "s", "args": {"pane": "right", "to": step}})
            );
        }
    }

    /// Upstream's `searchRefusesAMissingOrMixedRequest`, with each refusal's text.
    #[test]
    fn search_refuses_a_missing_or_mixed_request() {
        for (argv, error) in [
            (&["search"][..], "give text, --next, --prev or --close"),
            (&["search", ""], "text must not be empty"),
            (
                &["search", "a", "--next"],
                "text cannot be combined with --next, --prev or --close",
            ),
            (
                &["search", "--next", "--prev"],
                "--next, --prev and --close are mutually exclusive",
            ),
            (
                &["search", "a", "--close", "--prev"],
                "--next, --prev and --close are mutually exclusive",
            ),
            (&["search", "a", "--pane", "scratch"], "--pane must be left or right"),
        ] {
            assert_eq!(outcome(argv), Err(error.to_owned()), "{argv:?}");
        }
    }

    /// Upstream's `changesSendsItsStepPaneAndTarget`.
    #[test]
    fn changes_sends_its_step_pane_and_target() {
        for step in ["next", "prev"] {
            assert_eq!(
                request(&["changes", &format!("--{step}"), "--pane", "right", "--target", "s"]),
                json!({"cmd": "session.overlay.changes", "target": "s", "args": {"pane": "right", "to": step}})
            );
        }
    }

    /// Upstream's `changesRefusesAMissingOrDoubleStep`.
    #[test]
    fn changes_refuses_a_missing_or_double_step() {
        for (argv, error) in [
            (&["changes"][..], "give exactly one of --next or --prev"),
            (&["changes", "--next", "--prev"], "give exactly one of --next or --prev"),
            (
                &["changes", "--next", "--pane", "scratch"],
                "--pane must be left or right",
            ),
        ] {
            assert_eq!(outcome(argv), Err(error.to_owned()), "{argv:?}");
        }
    }
}
