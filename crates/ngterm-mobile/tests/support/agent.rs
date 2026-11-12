//! The e2e test's fake agent (`fake_agent.py`, a stand-in Claude Code in the pane's foreground) and what it is fed:
//! a transcript made from Task 1's recorded Claude session with generated turns after it, the hook payloads with
//! their transcript path moved into the private home, the config the program reads, and the state the test moves it
//! to ([`State`]).

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use uuid::Uuid;

use crate::machine::{World, quoted};

/// The recorded Claude conversation the transcript starts with, and the record's conversation id.
pub(crate) const CONVERSATION: &str = "f559ab9f-a5bd-4b2f-91e5-72590d662bf4";
/// The recorded conversation's first prompt: the transcript's oldest event a phone reads.
pub(crate) const FIRST_PROMPT: &str = "5d6f056b-68d5-48c9-810e-7530b2c7f6fe";
/// The recorded turn the generated ones copy (`Reply with just: resumed`, `resumed`).
const TEMPLATE_PROMPT: &str = "7b13d7aa-cbc2-449a-a59a-fc4bf2f4d4d8";
const TEMPLATE_REPLY: &str = "89477eac-bfad-46a5-be80-7279c5115356";
/// Generated turns after the recorded ones: with them the transcript holds more events than a backlog
/// (`agent_chat::follow::BACKLOG`), so the phone pages back to the start.
pub(crate) const FILLER_TURNS: u64 = 100;
/// The batch of generated turns the transcript is written with; the test appends later batches.
pub(crate) const FILLER: u64 = 1;
/// The command the phone's approval card is for: no call of the transcript runs it, so nothing answers it there.
pub(crate) const APPROVAL_COMMAND: &str = "touch e2e-approval.txt";
/// The second approval's command: one answered while the phone is away.
pub(crate) const AWAY_COMMAND: &str = "touch e2e-away.txt";
/// The tool use id the fake agent writes its answered `AskUserQuestion` call under.
pub(crate) const QUESTION_CALL: &str = "toolu_e2e_question";

const CLAUDE_FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ngterm-core/tests/fixtures/agent_chat/claude"
);
const SCREENS: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/agent_screens");
const PROGRAM: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/support/fake_agent.py");

/// The fake agent's files, made before it starts.
pub(crate) struct FakeAgent {
    /// The transcript the program writes and the origin follows: `<private home>/.claude/projects/…/<id>.jsonl`.
    pub(crate) transcript: PathBuf,
    /// The ids of the generated turns' events, oldest first.
    pub(crate) filler: Vec<String>,
    turns: Turns,
    config: PathBuf,
    control: PathBuf,
    pid: PathBuf,
}

/// What the fake agent shows, as [`FakeAgent::show`] sets it.
#[derive(Debug, Clone, Copy)]
pub(crate) enum State {
    Idle,
    /// A turn in progress: a prompt sent now is absorbed into it.
    Working,
    /// The `AskUserQuestion` picker of [`FakeAgent::question`].
    Question,
}

impl FakeAgent {
    /// Writes the backlog, the `SessionStart` payload and the config for a fake agent in `session`'s pane `pane`.
    pub(crate) fn prepare(world: &World, session: &str, pane: &str) -> Self {
        let directory = world.work.join("agent");
        fs::create_dir_all(&directory).expect("create the fake agent's directory");
        let transcript = world
            .headless
            .private
            .home
            .join(".claude/projects/-e2e-project")
            .join(format!("{CONVERSATION}.jsonl"));
        let recorded = fs::read_to_string(recorded_transcript()).expect("read the recorded transcript");
        let turns = Turns::from(&recorded);
        let mut backlog = recorded;
        let mut filler = Vec::new();
        for turn in 0..FILLER_TURNS {
            let (lines, ids) = turns.turn(FILLER, turn);
            backlog.push_str(&lines);
            filler.extend(ids);
        }
        let backlog_path = directory.join("backlog.jsonl");
        fs::write(&backlog_path, backlog).expect("write the backlog");
        let payload = directory.join("session-start.json");
        fs::write(&payload, hook_payload("01-SessionStart.json", &transcript)).expect("write the payload");
        let config = directory.join("config.json");
        let control = directory.join("state");
        let pid = directory.join("pid");
        let settings = json!({
            "transcript": transcript,
            "backlog": backlog_path,
            "payload": payload,
            "user_template": turns.user,
            "screens": SCREENS,
            "ngtermctl": world.headless.private.stage.join("bin/ngtermctl"),
            "socket": world.headless.socket(),
            "target": session,
            "pane_id": pane,
            "control": control,
            "pid": pid,
            "question": question_input(),
            "assistant_template": turns.assistant,
        });
        fs::write(&config, settings.to_string()).expect("write the config");
        Self {
            transcript,
            filler,
            turns,
            config,
            control,
            pid,
        }
    }

    /// Moves the running program to `state`, which it draws at once.
    pub(crate) fn show(&self, state: State) {
        let name = match state {
            State::Idle => "idle",
            State::Working => "working",
            State::Question => "question",
        };
        fs::write(&self.control, name).expect("write the fake agent's state");
        let pid = fs::read_to_string(&self.pid).expect("the fake agent's pid");
        let signalled = std::process::Command::new("kill")
            .args(["-USR1", pid.trim()])
            .status()
            .expect("signal the fake agent");
        assert!(signalled.success(), "the fake agent is not running");
    }

    /// The `PermissionRequest` Claude Code fires for [`FakeAgent::question`] as its picker opens.
    pub(crate) fn question_payload(&self) -> String {
        let mut payload: Value =
            serde_json::from_str(&hook_payload("27-PermissionRequest.json", &self.transcript)).expect("a payload");
        payload["tool_input"] = question_input();
        payload.to_string()
    }

    /// The line that starts the program in a pane's shell: the system's `python3`, by its path.
    pub(crate) fn command_line(&self) -> String {
        let python = ["/usr/bin/python3", "/bin/python3"]
            .into_iter()
            .map(Path::new)
            .find(|path| path.exists())
            .expect("the fake agent runs on python3, in /usr/bin or /bin (the pane's PATH)");
        format!(
            "{} {} {}",
            quoted(&python.display().to_string()),
            quoted(PROGRAM),
            quoted(&self.config.display().to_string())
        )
    }

    /// Appends `count` generated turns of `batch` to the transcript, as the agent writes them; their events' ids.
    pub(crate) fn append(&self, batch: u64, count: u64) -> Vec<String> {
        let mut lines = String::new();
        let mut ids = Vec::new();
        for turn in 0..count {
            let (text, turn_ids) = self.turns.turn(batch, turn);
            lines.push_str(&text);
            ids.extend(turn_ids);
        }
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.transcript)
            .expect("open the transcript");
        // one write, so the follower never reads a turn in part
        file.write_all(lines.as_bytes()).expect("append to the transcript");
        ids
    }

    /// A `Bash` `PermissionRequest` for [`APPROVAL_COMMAND`], as Claude Code's hook gets it.
    pub(crate) fn permission_payload(&self, command: &str) -> String {
        let mut payload: Value =
            serde_json::from_str(&hook_payload("33-PermissionRequest.json", &self.transcript)).expect("a payload");
        payload["tool_input"] = json!({ "command": command, "description": "An e2e command" });
        payload.to_string()
    }
}

/// The `AskUserQuestion` input the phone's capture draws as its picker (Task 1's recorded `Pick a color`).
pub(crate) fn question_input() -> Value {
    json!({ "questions": [{
        "question": "Pick a color",
        "header": "Color",
        "multiSelect": false,
        "options": [{ "label": "Red", "description": "Red" }, { "label": "Green", "description": "Green" }],
    }] })
}

fn recorded_transcript() -> PathBuf {
    Path::new(CLAUDE_FIXTURES).join(format!("projects/-home-user-project/{CONVERSATION}.jsonl"))
}

/// The recorded hook payload `name` with its transcript path moved to `transcript`.
fn hook_payload(name: &str, transcript: &Path) -> String {
    let text = fs::read_to_string(Path::new(CLAUDE_FIXTURES).join("hooks").join(name)).expect("read a hook payload");
    let mut payload: Value = serde_json::from_str(&text).expect("a hook payload");
    payload["transcript_path"] = json!(transcript);
    payload.to_string()
}

/// A recorded turn's two lines, copied into generated turns.
struct Turns {
    user: Value,
    assistant: Value,
}

impl Turns {
    fn from(recorded: &str) -> Self {
        let line = |id: &str| {
            recorded
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).expect("a transcript line"))
                .find(|line| line["uuid"] == id)
                .unwrap_or_else(|| panic!("no line {id} in the recorded transcript"))
        };
        Self {
            user: line(TEMPLATE_PROMPT),
            assistant: line(TEMPLATE_REPLY),
        }
    }

    /// Turn `turn` of `batch`: its prompt and reply lines, and the ids of the events they make.
    fn turn(&self, batch: u64, turn: u64) -> (String, [String; 2]) {
        let id = |half: u64| Uuid::from_u128(u128::from(batch) << 64 | u128::from(turn * 2 + half)).to_string();
        let ids = [id(0), id(1)];
        let mut user = self.user.clone();
        user["uuid"] = json!(ids[0]);
        user["promptId"] = json!(ids[0]);
        user["message"]["content"] = json!(format!("e2e prompt {batch}.{turn}"));
        let mut assistant = self.assistant.clone();
        assistant["uuid"] = json!(ids[1]);
        assistant["parentUuid"] = json!(ids[0]);
        assistant["message"]["id"] = json!(format!("msg_e2e_{batch}_{turn}"));
        assistant["message"]["content"][0]["text"] = json!(format!("e2e reply {batch}.{turn}"));
        (format!("{user}\n{assistant}\n"), ids)
    }
}
