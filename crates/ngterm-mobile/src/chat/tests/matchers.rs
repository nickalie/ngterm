//! The screen matchers on Task 1's PTY captures, replayed through libghostty-vt at each recorded grid: each agent's
//! permission dialog with its option labels and keys, Claude's picker and its review, the idle prompt and a working
//! turn; and the screens they must not read: a dialog scrolled away, one half drawn, one quoted by a shell, another
//! agent's.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ngterm_term::{Frame, GridSize, Outgoing, StreamFeed, StreamOut};

use crate::chat::matchers::{Agent, Dialog, DialogOption, OptionKey, Picker, Screen, squeeze};

const DEADLINE: Duration = Duration::from_secs(10);

/// One recorded capture: every byte the agent wrote at a grid, and where each state was on screen.
pub(crate) struct Capture {
    pub(crate) bytes: Vec<u8>,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    marks: BTreeMap<String, usize>,
}

impl Capture {
    /// `<agent>-<cols>x<rows>` from `tests/fixtures/agent_screens`.
    pub(crate) fn load(name: &str) -> Self {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/agent_screens");
        let bytes = std::fs::read(format!("{dir}/{name}.bin")).unwrap();
        let marks: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(format!("{dir}/{name}.marks.json")).unwrap()).unwrap();
        let number = |key: &str| u16::try_from(marks[key].as_u64().unwrap()).unwrap();
        Self {
            cols: number("cols"),
            rows: number("rows"),
            marks: marks["marks"]
                .as_object()
                .unwrap()
                .iter()
                .map(|(state, at)| (state.clone(), usize::try_from(at.as_u64().unwrap()).unwrap()))
                .collect(),
            bytes,
        }
    }

    /// The bytes up to the mark `state`.
    pub(crate) fn at(&self, state: &str) -> &[u8] {
        &self.bytes[..self.marks[state]]
    }

    pub(crate) fn has(&self, state: &str) -> bool {
        self.marks.contains_key(state)
    }

    /// A fresh terminal at the capture's grid.
    pub(crate) fn term(&self) -> Term {
        Term::new(self.cols, self.rows)
    }

    /// The screen the capture's `state` leaves, as `agent` reads it.
    fn read(&self, agent: Agent, state: &str) -> Screen {
        let term = self.term();
        term.feed(self.at(state));
        agent.read(&term.frame())
    }
}

/// A stream terminal fed by hand.
pub(crate) struct Term {
    pub(crate) pane: ngterm_term::Pane,
    feed: StreamFeed,
    out: StreamOut,
}

impl Term {
    pub(crate) fn new(cols: u16, rows: u16) -> Self {
        let size = GridSize {
            cols,
            rows,
            cell_width: 8,
            cell_height: 16,
        };
        let (pane, feed, out) = ngterm_term::Pane::stream(&crate::pane::config(size, Arc::new(|| {}))).unwrap();
        Self { pane, feed, out }
    }

    /// Feeds `bytes` and waits until the terminal took them all (a cursor report asked after them comes back);
    /// what the terminal answered the program meanwhile is dropped.
    pub(crate) fn feed(&self, bytes: &[u8]) {
        assert!(self.feed.deliver(bytes));
        assert!(self.feed.deliver(b"\x1b[6n"));
        let deadline = Instant::now() + DEADLINE;
        let mut sent = Vec::new();
        while !sent.ends_with(b"R") {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(!left.is_zero(), "no cursor report");
            if let Ok(Some(Outgoing::Bytes(bytes))) = self.out.next(Some(left)) {
                sent.extend(bytes);
            }
        }
    }

    /// What the terminal sent the program since, until it has been quiet a moment.
    pub(crate) fn sent(&self) -> Vec<u8> {
        let mut sent = Vec::new();
        while let Ok(Some(Outgoing::Bytes(bytes))) = self.out.next(Some(Duration::from_millis(100))) {
            sent.extend(bytes);
        }
        sent
    }

    /// The screen once it holds still: two frames a moment apart the same, neither stale.
    pub(crate) fn frame(&self) -> Arc<Frame> {
        let deadline = Instant::now() + DEADLINE;
        let mut last: Option<Arc<Frame>> = None;
        loop {
            let snapshot = self.pane.frame();
            if !snapshot.stale {
                if last.as_ref().is_some_and(|last| last.seq == snapshot.frame.seq) {
                    return snapshot.frame;
                }
                last = Some(snapshot.frame);
            }
            assert!(Instant::now() < deadline, "the screen never held still");
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

const GRIDS: [&str; 3] = ["44x36", "100x40", "160x48"];

fn captures(agent: &str) -> Vec<(String, Capture)> {
    GRIDS
        .iter()
        .map(|grid| {
            let name = format!("{agent}-{grid}");
            let capture = Capture::load(&name);
            (name, capture)
        })
        .collect()
}

fn dialog(screen: &Screen) -> &Dialog {
    match screen {
        Screen::Permission(dialog) => dialog,
        other => panic!("not a dialog: {other:?}"),
    }
}

fn picker(screen: &Screen) -> &Picker {
    match screen {
        Screen::Question(picker) => picker,
        other => panic!("not the picker: {other:?}"),
    }
}

/// The recorded project path, as Claude's second option names it.
const PROJECT: &str = "/tmp/claude-1000/-home-user-projects-example-workspace/00000000-0000-0000-0000-000000000000/scratchpad/rec/project";

#[test]
fn claudes_permission_dialog_yields_its_labels_and_digits_at_every_width() {
    for (name, capture) in captures("claude") {
        let screen = capture.read(Agent::Claude, "permission");
        let options = &dialog(&screen).options;
        let labels: Vec<String> = options.iter().map(|option| squeeze(&option.label)).collect();
        assert_eq!(
            labels,
            [
                "Yes".to_owned(),
                squeeze(&format!("Yes, and always allow access to {PROJECT} from this project")),
                squeeze("Yes, and switch to auto mode · auto mode handles these prompts for you"),
                "No".to_owned(),
            ],
            "{name}"
        );
        let keys: Vec<OptionKey> = options.iter().map(|option| option.key).collect();
        assert_eq!(keys, [1, 2, 3, 4].map(OptionKey::Digit), "{name}: digits pick");
    }
    // wrapped rows join as the dialog words them where the rows tell (a row cut at the edge breaks a word, a short
    // one breaks between words)
    for name in ["claude-100x40", "claude-160x48"] {
        let screen = Capture::load(name).read(Agent::Claude, "permission");
        assert_eq!(
            dialog(&screen).options[1].label,
            format!("Yes, and always allow access to {PROJECT} from this project"),
            "{name}"
        );
    }
}

#[test]
fn codexs_approval_yields_its_labels_and_its_keys_at_every_width() {
    for (name, capture) in captures("codex") {
        let screen = capture.read(Agent::Codex, "permission");
        assert_eq!(
            dialog(&screen).options,
            [
                DialogOption {
                    label: "Yes, proceed".to_owned(),
                    key: OptionKey::Letter('y'),
                },
                DialogOption {
                    label: format!(
                        "Yes, and don't ask again for commands that start with `{}`",
                        if name.ends_with("160x48") {
                            "curl -sI https://example.com"
                        } else {
                            "curl -sI"
                        }
                    ),
                    key: OptionKey::Letter('p'),
                },
                DialogOption {
                    label: "No, and tell Codex what to do differently".to_owned(),
                    key: OptionKey::Escape,
                },
            ],
            "{name}"
        );
    }
}

#[test]
fn a_dialog_finds_its_option_by_label_whatever_the_wrap() {
    let screen = Capture::load("claude-44x36").read(Agent::Claude, "permission");
    let dialog = dialog(&screen);
    let wide = format!("Yes, and always allow access to {PROJECT} from this project");
    assert_eq!(dialog.option(&wide).unwrap().key, OptionKey::Digit(2));
    assert_eq!(dialog.option("  No ").unwrap().key, OptionKey::Digit(4));
    assert!(dialog.option("Maybe").is_none());
    assert!(dialog.option("Ye").is_none(), "a label is matched whole");
}

#[test]
fn claudes_picker_yields_the_question_and_its_options_at_every_width() {
    for (name, capture) in captures("claude") {
        let screen = capture.read(Agent::Claude, "question");
        let picker = picker(&screen);
        let options: Vec<(u8, String, bool)> = picker
            .options
            .iter()
            .map(|option| (option.number, squeeze(&option.text), option.cursor))
            .collect();
        assert!(!picker.multi(), "{name}");
        if name.ends_with("160x48") {
            assert_eq!(picker.question, "Which greeting style?");
            assert_eq!(
                options,
                [
                    (1, squeeze("Formal Polite, professional greeting"), true),
                    (2, squeeze("Casual Relaxed, friendly greeting"), false),
                    (3, squeeze("Pirate Arr, matey-style greeting"), false),
                    (4, "Typesomething.".to_owned(), false),
                ]
            );
        } else {
            assert_eq!(picker.question, "Pick a color", "{name}");
            assert_eq!(
                options,
                [
                    (1, "RedRed".to_owned(), true),
                    (2, "GreenGreen".to_owned(), false),
                    (3, "Typesomething.".to_owned(), false),
                ],
                "{name}"
            );
        }
    }
}

#[test]
fn a_multi_select_question_shows_its_boxes_and_the_review_its_submit() {
    let capture = Capture::load("claude-160x48");
    let screen = capture.read(Agent::Claude, "question_multi");
    let picker = picker(&screen);
    assert!(picker.multi());
    assert_eq!(picker.question, "Which languages should greet() support?");
    let options: Vec<(String, Option<bool>)> = picker
        .options
        .iter()
        .map(|option| (squeeze(&option.text), option.checked))
        .collect();
    assert_eq!(
        options[..3],
        [
            (squeeze("Spanish Support Spanish greetings"), Some(false)),
            (squeeze("French Support French greetings"), Some(false)),
            (squeeze("German Support German greetings"), Some(false)),
        ]
    );
    assert!(options[3].0.starts_with("Typesomething"));

    // ticked: the same screen with Spanish's box ticked
    let term = capture.term();
    term.feed(capture.at("question_multi"));
    term.feed("\x1b[37;7H✔".as_bytes());
    let screen = Agent::Claude.read(&term.frame());
    assert_eq!(picker_checked(&screen), [Some(true), Some(false), Some(false)]);

    assert_eq!(
        capture.read(Agent::Claude, "question_submit"),
        Screen::Review { on_submit: true }
    );
}

fn picker_checked(screen: &Screen) -> Vec<Option<bool>> {
    picker(screen).options[..3]
        .iter()
        .map(|option| option.checked)
        .collect()
}

#[test]
fn the_idle_prompt_and_a_working_turn_are_told_apart_at_every_width() {
    for agent in [Agent::Claude, Agent::Codex] {
        let name = if agent == Agent::Claude { "claude" } else { "codex" };
        for (capture_name, capture) in captures(name) {
            assert_eq!(
                capture.read(agent, "idle"),
                Screen::Prompt { working: false },
                "{capture_name}"
            );
            assert_eq!(
                capture.read(agent, "working"),
                Screen::Prompt { working: true },
                "{capture_name}"
            );
        }
    }
}

#[test]
fn another_agents_screens_read_as_nothing() {
    for (agent, other) in [(Agent::Claude, "codex"), (Agent::Codex, "claude")] {
        for (name, capture) in captures(other) {
            for state in ["idle", "working", "permission", "question"] {
                if capture.has(state) {
                    assert_eq!(
                        capture.read(agent, state),
                        Screen::Unknown,
                        "{agent:?} on {name} {state}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_dialog_that_scrolled_away_is_not_read() {
    for (agent, name) in [(Agent::Claude, "claude"), (Agent::Codex, "codex")] {
        for (capture_name, capture) in captures(name) {
            let term = capture.term();
            term.feed(capture.at("permission"));
            // the program goes on below it: the dialog moves up, the bottom is a shell's
            let rows = capture.rows;
            term.feed(format!("\x1b[{rows};1H{}user@host:~/project$ ", "\r\n".repeat(6)).as_bytes());
            assert_eq!(agent.read(&term.frame()), Screen::Unknown, "{capture_name}");
        }
    }
}

#[test]
fn a_shell_that_quotes_a_dialog_is_not_read() {
    let quotes = [
        (
            Agent::Claude,
            " Do you want to proceed?\r\n ❯ 1. Yes\r\n   2. No\r\n\r\n Esc to cancel · Tab to amend\r\n",
        ),
        (
            Agent::Codex,
            "› 1. Yes, proceed (y)\r\n  2. No, and tell Codex what to do differently (esc)\r\n\r\n  Press enter to confirm or esc to cancel\r\n",
        ),
    ];
    for (agent, quote) in quotes {
        let term = Term::new(80, 24);
        term.feed(format!("user@host:~$ cat dialog.txt\r\n{quote}user@host:~$ ").as_bytes());
        assert_eq!(agent.read(&term.frame()), Screen::Unknown, "{agent:?}");
    }
}

/// The last `marker` before the mark `state`: a word the dialog draws early.
fn last_before(capture: &Capture, state: &str, marker: &[u8]) -> usize {
    capture
        .at(state)
        .windows(marker.len())
        .rposition(|window| window == marker)
        .unwrap_or_else(|| panic!("no {marker:?} before the mark"))
}

#[test]
fn a_half_drawn_dialog_is_never_read_as_a_prompt_or_a_partial_dialog() {
    let cases = [
        (Agent::Claude, "claude", "permission", "proceed"),
        (Agent::Claude, "claude", "question", "☐"),
        (Agent::Codex, "codex", "permission", "Would"),
    ];
    for (agent, name, state, marker) in cases {
        for (capture_name, capture) in captures(name) {
            let whole = capture.read(agent, state);
            let end = capture.at(state).len();
            // from before the dialog is drawn to its end: Claude draws it from the top over what was there, so the
            // old prompt box stays under it until the dialog reaches it
            let drawn = last_before(&capture, state, marker.as_bytes());
            let start = drawn.saturating_sub(2048);
            let step = ((end - start) / 30).max(1);
            for cut in (start..end).step_by(step) {
                let term = capture.term();
                term.feed(&capture.bytes[..cut]);
                let frame = term.frame();
                if frame.held {
                    // the screen before the program's update: what a pane shows until the update ends
                    continue;
                }
                let screen = agent.read(&frame);
                let shown = || frame.lines().join("\n");
                match &screen {
                    Screen::Prompt { .. } => {
                        assert!(
                            cut <= drawn,
                            "{capture_name} {state} at {cut}: read as a prompt:\n{}",
                            shown()
                        );
                    }
                    other if other.is_dialog() => {
                        assert_eq!(
                            *other,
                            whole,
                            "{capture_name} {state} at {cut}: a partial dialog:\n{}",
                            shown()
                        );
                    }
                    _ => {}
                }
            }
        }
    }
}
