//! The screen matchers on Task 1's PTY captures, replayed through libghostty-vt at each recorded grid: each agent's
//! permission dialog with its option labels and keys, Claude's picker and its review, the idle prompt and a working
//! turn, and what a prompt holds (its placeholder left out, a draft read); and the screens they must not read: a
//! dialog scrolled away, one half drawn, one quoted by a shell, another agent's.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use ngterm_term::{Frame, GridSize, Outgoing, StreamFeed, StreamOut};

use crate::chat::ChatApproval;
use crate::chat::matchers::{Agent, Dialog, DialogOption, OptionKey, Picker, Screen, SubjectRow, squeeze};

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
        Self::read_term(&term, agent)
    }

    /// What `term`'s screen shows of `agent`.
    fn read_term(term: &Term, agent: Agent) -> Screen {
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

fn empty(working: bool) -> Screen {
    Screen::Prompt {
        working,
        composer: String::new(),
    }
}

const AGENTS: [(Agent, &str); 2] = [(Agent::Claude, "claude"), (Agent::Codex, "codex")];

#[test]
fn the_idle_prompt_and_a_working_turn_are_told_apart_at_every_width() {
    // the placeholder an empty prompt shows (faint) is not read as text in it
    for (agent, name) in AGENTS {
        for (capture_name, capture) in captures(name) {
            assert_eq!(capture.read(agent, "idle"), empty(false), "{capture_name}");
            assert_eq!(capture.read(agent, "working"), empty(true), "{capture_name}");
        }
    }
}

/// The bytes that put `text` in the prompt of `term`'s screen, after its mark, in `style` (an SGR), the rest of the
/// row cleared.
pub(crate) fn into_prompt(term: &Term, agent: Agent, style: &str, text: &str) -> Vec<u8> {
    let mark = if agent == Agent::Claude { '❯' } else { '›' };
    let row = term
        .frame()
        .lines()
        .iter()
        .rposition(|line| line.starts_with(mark))
        .expect("a prompt row");
    format!("\x1b[{};3H\x1b[0m\x1b[K\x1b[{style}m{text}\x1b[0m", row + 1).into_bytes()
}

#[test]
fn a_slash_commands_popup_two_rows_an_entry_on_a_phone_is_still_under_the_prompt() {
    use std::fmt::Write as _;
    // Claude Code 2.1.318 at 52 columns: the pasted `/context` over seven commands, each wrapped onto two rows
    let rule = "─".repeat(52);
    let popup: String = [
        ("/context", "Visualize current context"),
        ("/doctor", "Health-check your setup"),
        ("/autocompact", "Set how full the context"),
        ("/subtask", "Send a subagent off with"),
        ("/clear", "Start a new session with"),
        ("/compact", "Free up context by"),
        ("/skill-doctor", "Show which loaded skills"),
    ]
    .iter()
    .fold(String::new(), |mut rows, (name, about)| {
        let _ = write!(rows, "    {name:<20}{about}\r\n{:24}and what comes after…\r\n", "");
        rows
    });
    let screen = format!("\x1b[2J\x1b[H● Done.\r\n\r\n{rule}\r\n❯ /context\r\n{rule}\r\n{popup}");
    let term = Term::new(52, 40);
    term.feed(screen.as_bytes());
    assert_eq!(
        Agent::Claude.read(&term.frame()),
        Screen::Prompt {
            working: false,
            composer: "/context".to_owned()
        }
    );
}

#[test]
fn a_box_left_over_other_output_or_too_far_up_is_no_prompt() {
    use std::fmt::Write as _;
    let rule = "─".repeat(52);
    let boxed = format!("\x1b[2J\x1b[H● Done.\r\n{rule}\r\n❯ \r\n{rule}\r\n");
    let read = |under: &str| {
        let term = Term::new(52, 40);
        term.feed(format!("{boxed}{under}").as_bytes());
        Agent::Claude.read(&term.frame())
    };
    let idle = Screen::Prompt {
        working: false,
        composer: String::new(),
    };
    assert_eq!(read("  ⏵⏵ auto mode on (shift+tab to cycle)\r\n"), idle, "its footer");
    // Claude exited and the shell went on under the box it left
    assert_eq!(
        read("nick@box ~/project\r\n$ ls\r\nREADME.md\r\n$ \r\n"),
        Screen::Unknown,
        "a shell's rows"
    );
    // indented, but more rows than a footer and a popup take
    let tall = (0..30).fold(String::new(), |mut rows, at| {
        let _ = write!(rows, "    row {at}\r\n");
        rows
    });
    assert_eq!(read(&tall), Screen::Unknown, "too far up");
}

#[test]
fn a_draft_in_the_prompt_is_read_and_a_faint_hint_is_not() {
    for (agent, name) in AGENTS {
        for (capture_name, capture) in captures(name) {
            for (state, working) in [("idle", false), ("working", true)] {
                let term = capture.term();
                term.feed(capture.at(state));
                term.feed(&into_prompt(&term, agent, "0", "draft /comp"));
                assert_eq!(
                    agent.read(&term.frame()),
                    Screen::Prompt {
                        working,
                        composer: "draft /comp".to_owned()
                    },
                    "{capture_name} {state}"
                );
                term.feed(&into_prompt(&term, agent, "2", "a hint"));
                assert_eq!(
                    agent.read(&term.frame()),
                    empty(working),
                    "{capture_name} {state}: faint"
                );
            }
        }
    }
    // Claude's box grown to two input rows: both are read
    let capture = Capture::load("claude-44x36");
    let term = capture.term();
    term.feed(capture.at("idle"));
    let lines = term.frame().lines();
    let input = lines.iter().rposition(|line| line.starts_with('❯')).unwrap();
    let rule = "─".repeat(usize::from(capture.cols));
    // the upper rule one row up, the input on the two rows under it (1-based rows)
    term.feed(
        format!(
            "\x1b[{};1H{rule}\x1b[{};1H\x1b[2K❯ two\x1b[{};1H\x1b[2K  lines",
            input - 1,
            input,
            input + 1
        )
        .as_bytes(),
    );
    assert_eq!(
        Capture::read_term(&term, Agent::Claude),
        Screen::Prompt {
            working: false,
            composer: "two\nlines".to_owned()
        }
    );
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
    const CLAUDE: &str = " Do you want to proceed?\r\n ❯ 1. Yes\r\n   2. No\r\n\r\n Esc to cancel · Tab to amend\r\n";
    const CODEX: &str = "› 1. Yes, proceed (y)\r\n  2. No, and tell Codex what to do differently (esc)\r\n\r\n  Press enter to confirm or esc to cancel\r\n";
    // a dialog recorded at a narrower grid: its rule is not the screen's width
    let narrow = format!("{}\r\n Bash command\r\n\r\n{CLAUDE}", "─".repeat(40));
    let quotes = [
        (Agent::Claude, CLAUDE.to_owned()),
        (Agent::Claude, narrow),
        (Agent::Codex, CODEX.to_owned()),
    ];
    for (agent, quote) in &quotes {
        // a shell prompt after it: the footer is not the last line
        let term = Term::new(80, 24);
        term.feed(format!("user@host:~$ cat dialog.txt\r\n{quote}user@host:~$ ").as_bytes());
        assert_eq!(agent.read(&term.frame()), Screen::Unknown, "{agent:?} {quote:?}");
        // the quote ends the screen (`tail -f`, no prompt yet): Claude's needs the full-width rule over it, Codex's a
        // blank row over its first option
        let term = Term::new(80, 24);
        term.feed(format!("user@host:~$ tail -f dialog.txt\r\n{quote}").as_bytes());
        assert_eq!(
            agent.read(&term.frame()),
            Screen::Unknown,
            "{agent:?} {quote:?} ending the screen"
        );
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
    half_drawn_sweep(None);
}

#[test]
fn a_half_drawn_dialog_over_a_prompt_quoting_a_dialog_is_never_read_as_a_prompt() {
    // the prompt's own rows are left out of the dialog-words search: the dialog's rule and words around them still
    // keep it from being read as a prompt while it is drawn over one that holds those words
    half_drawn_sweep(Some("Explain Esc to cancel (y)"));
}

/// Replays each dialog's draw over the prompt cut by cut: no cut reads as a prompt once the dialog began, nor as any
/// dialog but the whole one. With `draft`, the prompt holds it (in the plain style, the TUI's cursor kept) from
/// before the draw.
fn half_drawn_sweep(draft: Option<&str>) {
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
            let mut start = drawn.saturating_sub(2048);
            let typed = draft.map(|draft| {
                let (at, typed) = typed_before(&capture, agent, start, draft, &capture_name);
                start = at;
                typed
            });
            let step = ((end - start) / 30).max(1);
            for cut in (start..end).step_by(step) {
                let term = capture.term();
                term.feed(&capture.bytes[..start]);
                if let Some(typed) = &typed {
                    term.feed(typed);
                }
                term.feed(&capture.bytes[start..cut]);
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

/// Where and with what bytes to type `draft` into the prompt the capture shows at byte `at`, or before it when an
/// update the program holds (2026) is open there: at its start. The cursor is saved and restored around the bytes, so
/// the TUI's own drawing goes on where it was; the prompt is checked to read as holding it (first, over the prompt's
/// first row: the recorded prompt may run on below).
fn typed_before(capture: &Capture, agent: Agent, at: usize, draft: &str, name: &str) -> (usize, Vec<u8>) {
    const BEGIN: &[u8] = b"\x1b[?2026h";
    let mut at = at;
    loop {
        let term = capture.term();
        term.feed(&capture.bytes[..at]);
        if term.frame().held {
            at = capture.bytes[..at]
                .windows(BEGIN.len())
                .rposition(|window| window == BEGIN)
                .expect("a held update begins");
            continue;
        }
        let mut bytes = b"\x1b7".to_vec();
        bytes.extend(into_prompt(&term, agent, "0", draft));
        bytes.extend(b"\x1b8");
        term.feed(&bytes);
        let screen = agent.read(&term.frame());
        assert!(
            matches!(&screen, Screen::Prompt { composer, .. } if composer.starts_with(draft)),
            "{name}: the draft is read: {screen:?}"
        );
        return (at, bytes);
    }
}

/// Drafts whose rows trip a naive reading of a prompt: a blank line inside, a first line shaped like an option, an
/// option-shaped line (with Codex's cursor) further down, a single line long enough to wrap at every width, and
/// several blank lines with one at the end.
pub(crate) fn tricky_drafts() -> Vec<String> {
    vec![
        "Fix the parser.\n\nThen add tests.".to_owned(),
        "1. Fix the bug\n2. Add tests".to_owned(),
        "Do these in order:\n1. Fix the bug\n\n2. Add tests\n› 3. Ship it".to_owned(),
        "Please refactor the parser module so that it handles nested quotes ".repeat(4),
        "First paragraph.\n\n\nThird, after two blank lines.\n".to_owned(),
    ]
}

/// The rows a TUI lays `text` out on in a prompt `cols` wide: each line wrapped at a word (a word longer than a row
/// cut), an empty line an empty row.
fn wrapped(text: &str, cols: u16) -> Vec<String> {
    let width = usize::from(cols) - 3;
    let mut rows = Vec::new();
    for line in text.split('\n') {
        let mut row = String::new();
        for word in line.split(' ') {
            let mut word = word.to_owned();
            while word.chars().count() > width {
                let rest = word.split_off(word.char_indices().nth(width).unwrap().0);
                rows.push(std::mem::replace(&mut word, rest));
            }
            if !row.is_empty() && row.chars().count() + 1 + word.chars().count() > width {
                rows.push(std::mem::take(&mut row));
            } else if !row.is_empty() {
                row.push(' ');
            }
            row.push_str(&word);
        }
        rows.push(row);
    }
    rows
}

/// Puts `text` on `row` (0-based) of the screen, cleared first, in the plain style.
fn put(bytes: &mut String, row: usize, text: &str) {
    write!(bytes, "\x1b[{};1H\x1b[0m\x1b[2K{text}", row + 1).unwrap();
}

/// A terminal showing the capture's `state` with `text` in `agent`'s prompt ([`draft_drawn`]), and what the prompt
/// then holds as the matcher reads it.
pub(crate) fn with_draft(capture: &Capture, agent: Agent, state: &str, text: &str) -> (Term, String) {
    let term = capture.term();
    term.feed(capture.at(state));
    let (bytes, held) = draft_drawn(capture, agent, state, text);
    term.feed(&bytes);
    (term, held)
}

/// The bytes that draw `text` in `agent`'s prompt over the capture's `state`, laid out as the TUI grows it: Codex's
/// composer upward from its row over the footer (the status line kept as far above it), Claude's box upward from its
/// lower rule (the two rows above it moved up with it). And what the prompt then holds as the matcher reads it: the
/// rows with text, trimmed.
pub(crate) fn draft_drawn(capture: &Capture, agent: Agent, state: &str, text: &str) -> (Vec<u8>, String) {
    let term = capture.term();
    term.feed(capture.at(state));
    let lines = term.frame().lines();
    let rows = wrapped(text, capture.cols);
    let mut bytes = String::new();
    if agent == Agent::Codex {
        let composer = lines.iter().rposition(|line| line.starts_with('›')).unwrap();
        let status = lines[..composer]
            .iter()
            .rposition(|line| line.contains("esc to interrupt"))
            .filter(|_| state == "working");
        let gap = status.map_or(1, |status| composer - status);
        let top = composer + 1 - rows.len();
        for row in top - gap - 1..=composer {
            put(&mut bytes, row, "");
        }
        if let Some(status) = status {
            put(&mut bytes, top - gap, &lines[status]);
        }
        for (at, row) in rows.iter().enumerate() {
            let mark = if at == 0 { "› " } else { "  " };
            put(&mut bytes, top + at, &format!("{mark}{row}"));
        }
    } else {
        let rule = "─".repeat(usize::from(capture.cols));
        let lower = lines.iter().rposition(|line| *line == rule).unwrap();
        let upper = lines[..lower].iter().rposition(|line| *line == rule).unwrap();
        let top = lower - rows.len() - 1;
        for row in top - 2..lower {
            put(&mut bytes, row, "");
        }
        put(&mut bytes, top - 2, &lines[upper - 2]);
        put(&mut bytes, top - 1, &lines[upper - 1]);
        put(&mut bytes, top, &rule);
        for (at, row) in rows.iter().enumerate() {
            let mark = if at == 0 { "❯ " } else { "  " };
            put(&mut bytes, top + 1 + at, &format!("{mark}{row}"));
        }
    }
    let held: Vec<&str> = rows
        .iter()
        .map(|row| row.trim())
        .filter(|row| !row.is_empty())
        .collect();
    (bytes.into_bytes(), held.join("\n"))
}

#[test]
fn a_draft_with_blank_lines_option_shaped_lines_or_long_wrapped_lines_is_read_whole() {
    for (agent, name) in AGENTS {
        for (capture_name, capture) in captures(name) {
            for (state, working) in [("idle", false), ("working", true)] {
                for draft in tricky_drafts() {
                    let (term, held) = with_draft(&capture, agent, state, &draft);
                    assert_eq!(
                        agent.read(&term.frame()),
                        Screen::Prompt {
                            working,
                            composer: held.clone()
                        },
                        "{capture_name} {state} {draft:?}:\n{}",
                        term.frame().lines().join("\n")
                    );
                    assert_eq!(squeeze(&held), squeeze(&draft), "{capture_name} {draft:?}");
                }
            }
        }
    }
}

/// Codex's approval as it could be drawn in the composer's place: option rows naming a dialog's keys, or with a
/// dialog's words under them.
const APPROVAL_SHAPED: [&str; 2] = [
    "1. Yes, proceed (y)\n2. No (esc)",
    "1. Yes, proceed\n2. No\nPress enter to confirm or esc to cancel",
];

#[test]
fn a_codex_composer_shaped_like_its_approval_is_not_read() {
    // an approval drawn into the composer's place, not a prompt, in either state; Claude's box has no such shape
    // (a dialog never reaches its rows while its rules stand), so there the same text is a draft
    for (state, working) in [("idle", false), ("working", true)] {
        for draft in APPROVAL_SHAPED {
            for (capture_name, capture) in captures("codex") {
                let (term, _) = with_draft(&capture, Agent::Codex, state, draft);
                assert_eq!(
                    Agent::Codex.read(&term.frame()),
                    Screen::Unknown,
                    "{capture_name} {state} {draft:?}"
                );
            }
            for (capture_name, capture) in captures("claude") {
                let (term, held) = with_draft(&capture, Agent::Claude, state, draft);
                assert_eq!(
                    Agent::Claude.read(&term.frame()),
                    Screen::Prompt {
                        working,
                        composer: held
                    },
                    "{capture_name} {state} {draft:?}"
                );
            }
        }
    }
}

/// Drafts quoting what the matchers take for a dialog: each of its words (in a sentence and alone on a row), Codex's
/// key hints, option rows with either agent's cursor, and whole dialogs quoted under a first line that is text.
pub(crate) fn marked_drafts() -> Vec<String> {
    let mut drafts: Vec<String> = [
        "Esc to cancel",
        "esc to cancel",
        "Enter to select",
        "Press enter to confirm",
        "Do you want to proceed?",
        "Ready to submit your answers?",
    ]
    .iter()
    .flat_map(|mark| [format!("Explain {mark} here."), format!("What does this mean?\n{mark}")])
    .collect();
    drafts.extend(
        [
            "Explain Esc to cancel.",
            "Should it run? (y)",
            "Answer with (p) or (esc)",
            "Pick one:\n❯ 1. Yes\n2. No",
            "It asked:\nDo you want to proceed?\n❯ 1. Yes\n2. No\nEsc to cancel · Tab to amend",
            "Codex showed:\n› 1. Yes, proceed (y)\n2. No, and tell Codex what to do differently (esc)\nPress enter to confirm or esc to cancel",
            "Then:\nReady to submit your answers?\n❯ 1. Submit answers\n2. Cancel\nEnter to select · ↑/↓ to navigate · Esc to cancel",
        ]
        .map(str::to_owned),
    );
    drafts
}

#[test]
fn a_draft_quoting_a_dialog_is_read_as_the_prompt_it_is() {
    for (agent, name) in AGENTS {
        for (capture_name, capture) in captures(name) {
            for (state, working) in [("idle", false), ("working", true)] {
                for draft in marked_drafts() {
                    let (term, held) = with_draft(&capture, agent, state, &draft);
                    assert_eq!(
                        agent.read(&term.frame()),
                        Screen::Prompt {
                            working,
                            composer: held.clone()
                        },
                        "{capture_name} {state} {draft:?}:\n{}",
                        term.frame().lines().join("\n")
                    );
                    assert_eq!(squeeze(&held), squeeze(&draft), "{capture_name} {draft:?}");
                }
            }
        }
    }
}

#[test]
fn a_dialogs_words_around_the_prompt_still_keep_it_from_being_read() {
    // the prompt quotes a dialog; the same words above it (a dialog's question left over) or under it (its footer)
    // are not its text: not a prompt
    const DRAFT: &str = "Explain Esc to cancel.";
    for (agent, name) in AGENTS {
        for (capture_name, capture) in captures(name) {
            for state in ["idle", "working"] {
                let (term, _) = with_draft(&capture, agent, state, DRAFT);
                let lines = term.frame().lines();
                let mark = if agent == Agent::Claude { '❯' } else { '›' };
                let row = lines.iter().rposition(|line| line.starts_with(mark)).unwrap();
                let last = lines.iter().rposition(|line| !line.trim().is_empty()).unwrap();
                for (at, words) in [
                    (row - 3, " Do you want to proceed?"),
                    (last, " Esc to cancel · Tab to amend"),
                ] {
                    let (marked, _) = with_draft(&capture, agent, state, DRAFT);
                    marked.feed(format!("\x1b[{};1H\x1b[2K{words}", at + 1).as_bytes());
                    assert_eq!(
                        agent.read(&marked.frame()),
                        Screen::Unknown,
                        "{capture_name} {state} {words:?} at {at}:\n{}",
                        marked.frame().lines().join("\n")
                    );
                }
            }
        }
    }
}

/// The command each capture's permission dialog asks to run, as the agent's hook reports it
/// (`crates/ngterm-core/tests/fixtures/agent_chat`: Claude's `33-PermissionRequest`, Codex's `25-PermissionRequest`
/// for the desktop grid; the phone and tablet recordings asked the commands their prompts gave).
fn command_of(capture: &str) -> &'static str {
    match capture {
        "claude-44x36" | "claude-100x40" => "touch picked.txt",
        "claude-160x48" => "touch allowed.txt",
        "codex-44x36" | "codex-100x40" => "curl -sI https://example.net",
        "codex-160x48" => "curl -sI https://example.com",
        other => panic!("no permission dialog in {other}"),
    }
}

/// The approval card for the permission dialog `capture` shows, as the chat has it from the agent's hook.
pub(crate) fn asked(capture: &str) -> ChatApproval {
    approval_of(command_of(capture))
}

/// An approval card for a shell call running `command`.
pub(crate) fn approval_of(command: &str) -> ChatApproval {
    ChatApproval {
        tool: "Bash".to_owned(),
        title: command.to_owned(),
        detail: Some(command.to_owned()),
        tool_use_id: None,
        cwd: None,
    }
}

/// The working directory the synthesized file dialogs' agent runs in.
pub(crate) const PROJECT_DIR: &str = "/home/user/project";

/// An approval card for Claude's `tool` writing the file at `path` (absolute, as the hook reports it), asked from
/// `cwd`.
pub(crate) fn file_approval(tool: &str, path: &str, cwd: Option<&str>) -> ChatApproval {
    ChatApproval {
        tool: tool.to_owned(),
        title: path.to_owned(),
        detail: Some(path.to_owned()),
        tool_use_id: None,
        cwd: cwd.map(str::to_owned),
    }
}

/// Claude's file-edit permission dialog naming the file `shown` (as the dialog words it: relative to the working
/// directory), drawn at 100x40. **Synthesized**, not recorded: Task 1's sessions recorded no Edit or Write dialog. It
/// follows the recorded Bash dialog's frame (a full-width rule over the title, the call between dashed rules, the
/// question, the numbered options, `Esc to cancel · Tab to amend`) with what Claude Code shows for a file tool: the
/// title, the path relative to its working directory over the diff, and a question naming only the file's name.
pub(crate) fn claude_file_dialog(title: &str, shown: &str) -> Screen {
    let (cols, rows) = (100, 40);
    let name = shown.rsplit('/').next().unwrap_or(shown);
    let rule = "─".repeat(usize::from(cols));
    let dashed = "╌".repeat(usize::from(cols));
    let lines = [
        "● Update(hello.py)".to_owned(),
        rule,
        format!(" {title}"),
        format!(" {shown}"),
        dashed.clone(),
        "  1  def greet(name):".to_owned(),
        "  2 -    return \"Hello, \" + name + \"!\"".to_owned(),
        "  2 +    return \"Hi, \" + name + \"!\"".to_owned(),
        dashed,
        format!(" Do you want to make this edit to {name}?"),
        " ❯ 1. Yes".to_owned(),
        "   2. Yes, allow all edits during this session (shift+tab)".to_owned(),
        "   3. No".to_owned(),
        " Esc to cancel · Tab to amend".to_owned(),
    ];
    let term = Term::new(cols, rows);
    term.feed(format!("\x1b[2J\x1b[H{}", lines.join("\r\n")).as_bytes());
    Agent::Claude.read(&term.frame())
}

#[test]
fn a_file_dialog_reads_as_a_dialog_whose_subject_names_the_path() {
    let screen = claude_file_dialog("Edit file", "src/hello.py");
    let dialog = dialog(&screen);
    let labels: Vec<&str> = dialog.options.iter().map(|option| option.label.as_str()).collect();
    assert_eq!(
        labels,
        ["Yes", "Yes, allow all edits during this session (shift+tab)", "No"]
    );
    assert!(
        dialog.subject.iter().any(|row| row.text == "src/hello.py"),
        "{:?}",
        dialog.subject
    );
    assert!(dialog.asks("src/hello.py"));
    // a path the dialog does not name whole: its file name alone, a part of it, the absolute path
    for other in [
        "hello.py",
        "src",
        "/home/user/project/src/hello.py",
        "other/src/hello.py",
    ] {
        assert!(!dialog.asks(other), "{other}");
    }
}

#[test]
fn a_dialog_asks_about_its_own_call_and_no_other() {
    for agent in ["claude", "codex"] {
        for (name, capture) in captures(agent) {
            let screen = capture.read(Agent::of(agent).unwrap(), "permission");
            let dialog = dialog(&screen);
            let command = command_of(&name);
            assert!(dialog.asks(command), "{name}: {:?}", dialog.subject);
            // its ends aside, whitespace counts: spaced otherwise, it is another call
            assert!(dialog.asks(&format!("  {command}\n")), "{name}");
            for spaced in [
                command.replacen(' ', "  ", 1),
                command.replacen(' ', "\n  ", 1),
                command.replacen(' ', "", 1),
            ] {
                assert!(!dialog.asks(&spaced), "{name}: {spaced:?}");
            }
            // another command, one that starts the same or says more, or a part of the command's row
            let others = [
                "touch picked.txt",
                "touch allowed.txt",
                "curl -sI https://example.net",
                "curl -sI https://example.com",
                "rm -rf build",
            ];
            for other in others.iter().filter(|other| **other != command) {
                assert!(!dialog.asks(other), "{name}: {other}");
            }
            let (start, end) = command.split_at(command.len() - 2);
            for part in [
                start,
                end,
                &command[6..],
                &format!("{command} && rm -rf build"),
                "",
                " ",
            ] {
                assert!(!dialog.asks(part), "{name}: {part:?}");
            }
        }
    }
    // the rows above a Claude dialog's top rule or a Codex dialog's `Would you like to …` (the transcript, which may
    // quote any command) are not what it asks about
    let screen = Capture::load("claude-44x36").read(Agent::Claude, "permission");
    assert!(!dialog(&screen).asks("touch picked.txt First call"));
    assert!(dialog(&screen).subject.iter().all(|row| !row.text.starts_with('●')));
    let screen = Capture::load("codex-44x36").read(Agent::Codex, "permission");
    assert!(dialog(&screen).subject[0].text.starts_with("Would you like to run"));
    assert!(!dialog(&screen).asks("Running curl -sI https://example.net"));
}

/// A dialog whose subject is `rows`, each `(text, full)`.
fn subject_of(rows: &[(&str, bool)]) -> Dialog {
    Dialog {
        subject: rows
            .iter()
            .map(|(text, full)| SubjectRow {
                text: (*text).to_owned(),
                full: *full,
            })
            .collect(),
        options: Vec::new(),
    }
}

#[test]
fn a_subject_keeps_its_whitespace_and_allows_only_the_wraps_a_screen_makes() {
    // within a row whitespace counts as it is
    let one = subject_of(&[("Bash command", false), ("touch 'a b'", false)]);
    assert!(one.asks("touch 'a b'"));
    for other in ["touch 'ab'", "touch  'a b'", "touch 'a  b'", "touch\t'a b'"] {
        assert!(!one.asks(other), "{other:?}");
    }
    // wrapped at a space (Claude) or a line of the command: the text holds whitespace there, any of it
    let soft = subject_of(&[("touch 'a", false), ("b'", false)]);
    assert!(soft.asks("touch 'a b'"));
    assert!(soft.asks("touch 'a\n  b'"));
    assert!(
        !soft.asks("touch 'ab'"),
        "a letter short of the edge broke at whitespace"
    );
    let lines = subject_of(&[("cat <<EOF", false), ("x", false), ("EOF", false)]);
    assert!(lines.asks("cat <<EOF\n  x\nEOF"));
    assert!(!lines.asks("cat <<EOFxEOF"));
    // cut at the screen's edge (Claude's long word), or after a mark a line may break at (Codex's `https://`)
    let hard = subject_of(&[("curl https://exa", true), ("mple.net", false)]);
    assert!(hard.asks("curl https://example.net"));
    let mark = subject_of(&[("$ curl -sI https://", false), ("example.net", false)]);
    assert!(mark.asks("curl -sI https://example.net"));
    assert!(mark.asks("curl -sI https:// example.net"));
    // a blank row ends the run: a command is not read across it
    let gap = subject_of(&[("touch", false), ("", false), ("x", false)]);
    assert!(!gap.asks("touch x"));
}
