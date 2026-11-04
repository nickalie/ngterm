//! The slash commands and skills an agent offers, for the chat composer's `/` dropdown: `ngtermctl agent commands`
//! prints them for the pane's agent.
//!
//! The agent's config directory comes from the record's transcript ([`config_dir`]), so `CLAUDE_CONFIG_DIR` and
//! `CODEX_HOME` from the agent's own environment hold. What is listed:
//!
//! - **Claude Code** ([`claude`]): the curated built-ins ([`CLAUDE_BUILTINS`]); the user's skills
//!   (`<config>/skills/*/SKILL.md`) and the project's (`<cwd>/.claude/skills`), the cwd being the one the transcript
//!   names ([`transcript_cwd`]); custom commands (`<config>/commands/**/*.md`, then `<cwd>/.claude/commands`), a
//!   subdirectory adding a `dir:` prefix; and the skills and commands of each plugin `<config>/settings.json` enables
//!   (`enabledPlugins`), found through `<config>/plugins/installed_plugins.json`, as `plugin:name`.
//! - **Codex** ([`codex`]): the curated built-ins ([`CODEX_BUILTINS`]) and the skills in `<home>/skills/*/SKILL.md`,
//!   named `$name`, the mention Codex invokes a skill with. Codex 0.166 offers no custom prompts.
//!
//! A name is what follows the `/` the user types, except a Codex skill's, which is inserted as it stands (`$name`).
//! A skill's name and description come from its frontmatter, falling back to its directory's name and the body's first
//! line; a command's name is its path (Claude names commands by file, not by frontmatter). A skill whose frontmatter
//! says `user-invocable: false` is left out, and so is a file that cannot be read (not UTF-8, a directory) or whose
//! frontmatter never closes: one bad file costs itself, not the list.
//!
//! **Duplicates** keep the first, in the order Claude Code documents for its skills: a built-in, then a personal
//! skill over a project one, and a skill over a command of the same name. A plugin's names carry its prefix, so they
//! never clash with the others.

use std::collections::HashSet;
use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use serde_with::skip_serializing_none;

use super::hook::{check_transcript, config_dir};
use super::{AgentKind, ClaudeParser, CodexParser, EventBody, TranscriptParser};

/// The Claude Code version [`CLAUDE_BUILTINS`] was checked against (the fixtures' recording).
pub const CLAUDE_VERSION: &str = "2.1.317";

/// The Codex CLI version [`CODEX_BUILTINS`] was checked against (the fixtures' recording).
pub const CODEX_VERSION: &str = "0.166.0";

/// The most of a skill or command file read: its frontmatter and first line are near its start.
const FILE_CAP: u64 = 64 * 1024;

/// The most bytes of a description.
pub const DESCRIPTION_CAP: usize = 1024;

/// How far below `commands/` subdirectories are read; a link cycle ends here.
const COMMAND_DEPTH: usize = 8;

/// The most of a transcript's head read for its cwd.
const HEAD_CAP: u64 = 4 * 1024 * 1024;

/// One entry of the dropdown.
#[skip_serializing_none]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlashCommand {
    /// What follows the `/`; a Codex skill's `$name` is inserted as it stands.
    pub name: String,
    /// One line; empty when the file has none.
    pub description: String,
    pub kind: CommandKind,
    /// What the command takes, e.g. `[instructions]`; absent when it says nothing.
    #[serde(default)]
    pub argument_hint: Option<String>,
}

/// Where an entry comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CommandKind {
    /// The agent's own.
    Builtin,
    /// A user or project skill.
    Skill,
    /// A user or project custom command.
    Command,
    /// A plugin's skill or command.
    Plugin,
}

/// A curated built-in: its name, description and argument hint.
pub type Builtin = (&'static str, &'static str, Option<&'static str>);

/// Claude Code's built-in slash commands a user runs from the prompt, as of [`CLAUDE_VERSION`] (its command table).
/// Left out: commands gated to accounts, platforms or internal builds, and ones that only manage the install.
pub const CLAUDE_BUILTINS: &[Builtin] = &[
    ("add-dir", "Add a new working directory", Some("<path>")),
    (
        "branch",
        "Create a branch of the current conversation at this point",
        Some("[name]"),
    ),
    (
        "btw",
        "Ask a quick side question without interrupting the main conversation",
        Some("[question]"),
    ),
    ("clear", "Start a new session with empty context", Some("[name]")),
    (
        "compact",
        "Free up context by summarizing the conversation so far",
        Some("<optional custom summarization instructions>"),
    ),
    ("config", "Open settings", Some("[key=value]")),
    ("context", "Show current context usage", Some("[all]")),
    ("copy", "Copy Claude's last response to clipboard", None),
    ("diff", "View uncommitted changes and per-turn diffs", None),
    ("effort", "Set effort level for model usage", None),
    ("exit", "Exit the REPL", None),
    (
        "export",
        "Export the current conversation to a file or clipboard",
        Some("[filename]"),
    ),
    (
        "feedback",
        "Send feedback to Anthropic or report a bug",
        Some("[report]"),
    ),
    (
        "fork",
        "Copy this conversation into a new background session and keep working here",
        Some("[prompt]"),
    ),
    (
        "goal",
        "Set a goal Claude checks before stopping",
        Some("[<condition> | clear]"),
    ),
    ("help", "Show help and available commands", None),
    ("hooks", "View hook configurations for tool events", None),
    ("ide", "Manage IDE integrations and show status", Some("[open]")),
    (
        "init",
        "Initialize a new CLAUDE.md file with codebase documentation",
        None,
    ),
    (
        "insights",
        "Generate a report analyzing your Claude Code sessions",
        None,
    ),
    ("login", "Sign in with your Anthropic account", None),
    ("logout", "Sign out from your Anthropic account", None),
    (
        "mcp",
        "Manage MCP servers",
        Some("[reconnect|enable|disable [<server>|all]]"),
    ),
    ("memory", "Edit CLAUDE.md files and memory settings", None),
    ("model", "Set the AI model for Claude Code", Some("<model>")),
    ("output-style", "List output styles or switch to one", Some("[style]")),
    ("permissions", "Manage allow and deny tool permission rules", None),
    (
        "plan",
        "Enable plan mode or view the current session plan",
        Some("[open|<description>]"),
    ),
    ("plugin", "Manage Claude Code plugins", None),
    ("recap", "Generate a one-line session recap now", None),
    (
        "reload-plugins",
        "Activate pending plugin changes in the current session",
        Some("[--force]"),
    ),
    (
        "reload-skills",
        "Pick up skills added or changed on disk during this session",
        None,
    ),
    ("rename", "Rename the current conversation", Some("[name]")),
    ("resume", "Resume a previous conversation", None),
    ("skills", "List available skills", None),
    (
        "status",
        "Show Claude Code status including version, model, account, API connectivity, and tool statuses",
        None,
    ),
    ("tasks", "View and manage everything running in the background", None),
    ("theme", "Change the theme", None),
    ("usage", "Show session cost, plan usage, and activity stats", None),
];

/// Codex's built-in slash commands, as of [`CODEX_VERSION`] (its command list and tips). Left out: experimental,
/// debugging and platform-specific ones.
pub const CODEX_BUILTINS: &[Builtin] = &[
    ("apps", "manage apps", None),
    ("clear", "clear the terminal and start a new chat", None),
    (
        "compact",
        "summarize conversation to prevent hitting the context limit",
        None,
    ),
    ("copy", "copy the last response or part of it", None),
    ("diff", "show git diff (including untracked files)", None),
    ("exit", "exit Codex", None),
    ("experimental", "toggle experimental features", None),
    ("export", "export the conversation as markdown", None),
    ("feedback", "send logs to maintainers", None),
    ("fork", "fork the current chat", None),
    ("goal", "set or view the goal for a long-running task", None),
    ("hooks", "view and manage lifecycle hooks", None),
    ("init", "create an AGENTS.md file with instructions for Codex", None),
    ("mcp", "list MCP tools; use /mcp verbose or /mcp login <name>", None),
    ("mention", "mention a file", None),
    ("model", "choose what model and reasoning effort to use", None),
    ("new", "start a new chat during a conversation", None),
    ("permissions", "choose what Codex is allowed to do", None),
    ("plan", "switch to Plan mode", None),
    ("plugins", "browse plugins", None),
    ("presets", "run or save reusable prompts and slash commands", None),
    ("quit", "exit Codex", None),
    ("rename", "rename the current thread", None),
    ("resume", "resume a saved chat", None),
    ("review", "review my current changes and find issues", None),
    ("side", "start a side conversation in an ephemeral fork", None),
    (
        "skills",
        "use skills to improve how Codex performs specific tasks",
        None,
    ),
    ("status", "show current session configuration and token usage", None),
    ("statusline", "configure which items appear in the status line", None),
    ("theme", "choose a syntax highlighting theme", None),
    ("title", "configure which items appear in the terminal title", None),
    ("usage", "view account usage or use a usage limit reset", None),
];

/// The catalog of the agent whose transcript is `transcript`: its config dir and (for Claude) its cwd come from it.
///
/// # Errors
///
/// The path lies outside the agent's tree ([`check_transcript`]'s refusal).
pub fn catalog(agent: AgentKind, transcript: &Path) -> Result<Vec<SlashCommand>, String> {
    let text = transcript.to_string_lossy();
    check_transcript(agent, &text)?;
    let config = config_dir(agent, transcript).ok_or_else(|| format!("no config dir for {text}"))?;
    Ok(match agent {
        AgentKind::Claude => claude(&config, transcript_cwd(agent, transcript).as_deref()),
        AgentKind::Codex => codex(&config),
    })
}

/// Claude Code's catalog for config dir `config`, with the project's skills and commands when `cwd` is known.
#[must_use]
pub fn claude(config: &Path, cwd: Option<&Path>) -> Vec<SlashCommand> {
    let project = cwd.map(|cwd| cwd.join(".claude"));
    let mut found = builtins(CLAUDE_BUILTINS);
    found.extend(skills(&config.join("skills"), "", CommandKind::Skill));
    if let Some(project) = &project {
        found.extend(skills(&project.join("skills"), "", CommandKind::Skill));
    }
    found.extend(commands(&config.join("commands"), "", CommandKind::Command));
    if let Some(project) = &project {
        found.extend(commands(&project.join("commands"), "", CommandKind::Command));
    }
    for (plugin, root) in plugins(config, cwd) {
        let prefix = format!("{plugin}:");
        found.extend(skills(&root.join("skills"), &prefix, CommandKind::Plugin));
        found.extend(commands(&root.join("commands"), &prefix, CommandKind::Plugin));
    }
    first_of_each(found)
}

/// Codex's catalog for `CODEX_HOME` `home`.
#[must_use]
pub fn codex(home: &Path) -> Vec<SlashCommand> {
    let mut found = builtins(CODEX_BUILTINS);
    found.extend(skills(&home.join("skills"), "$", CommandKind::Skill));
    first_of_each(found)
}

/// The working directory the transcript's conversation names, from the first lines that announce it.
#[must_use]
pub fn transcript_cwd(agent: AgentKind, transcript: &Path) -> Option<PathBuf> {
    let file = fs::File::open(transcript).ok()?;
    let mut reader = BufReader::new(file.take(HEAD_CAP));
    let mut parser: Box<dyn TranscriptParser> = match agent {
        AgentKind::Claude => Box::new(ClaudeParser::new()),
        AgentKind::Codex => Box::new(CodexParser::new()),
    };
    let (mut line, mut cursor) = (Vec::new(), 0_u64);
    loop {
        line.clear();
        let read = reader.read_until(b'\n', &mut line).ok()?;
        if read == 0 {
            return None;
        }
        let body = line.strip_suffix(b"\n").unwrap_or(&line);
        let cwd = parser
            .parse_line(body, cursor)
            .into_iter()
            .find_map(|event| match event.body {
                EventBody::Conversation { cwd, .. } => cwd,
                _ => None,
            });
        if let Some(cwd) = cwd {
            return Some(PathBuf::from(cwd));
        }
        cursor += read as u64;
    }
}

fn builtins(list: &[Builtin]) -> Vec<SlashCommand> {
    list.iter()
        .map(|&(name, description, hint)| SlashCommand {
            name: name.to_owned(),
            description: description.to_owned(),
            kind: CommandKind::Builtin,
            argument_hint: hint.map(str::to_owned),
        })
        .collect()
}

/// Keeps the first entry of each name.
fn first_of_each(found: Vec<SlashCommand>) -> Vec<SlashCommand> {
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter(|command| seen.insert(command.name.clone()))
        .collect()
}

/// The skills in `dir/*/SKILL.md`, by name, each named `prefix` + its frontmatter's name (else its directory's).
fn skills(dir: &Path, prefix: &str, kind: CommandKind) -> Vec<SlashCommand> {
    let mut found: Vec<SlashCommand> = entries(dir)
        .into_iter()
        .filter_map(|(name, path)| {
            let doc = Doc::read(&path.join("SKILL.md"))?;
            if doc
                .field("user-invocable")
                .is_some_and(|value| value.eq_ignore_ascii_case("false"))
            {
                return None;
            }
            let name = doc.field("name").filter(|name| !name.is_empty()).unwrap_or(&name);
            Some(doc.command(format!("{prefix}{name}"), kind))
        })
        .collect();
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

/// The commands in `dir/**/*.md`, by name, each named `prefix` + its path below `dir`, `:` between the parts.
fn commands(dir: &Path, prefix: &str, kind: CommandKind) -> Vec<SlashCommand> {
    let mut found = Vec::new();
    walk_commands(dir, prefix, kind, 0, &mut found);
    found.sort_by(|a, b| a.name.cmp(&b.name));
    found
}

fn walk_commands(dir: &Path, prefix: &str, kind: CommandKind, depth: usize, found: &mut Vec<SlashCommand>) {
    for (name, path) in entries(dir) {
        if path.is_dir() {
            if depth < COMMAND_DEPTH {
                walk_commands(&path, &format!("{prefix}{name}:"), kind, depth + 1, found);
            }
        } else if let Some(stem) = name.strip_suffix(".md").filter(|stem| !stem.is_empty())
            && let Some(doc) = Doc::read(&path)
        {
            found.push(doc.command(format!("{prefix}{stem}"), kind));
        }
    }
}

/// The entries of `dir` whose names are UTF-8 and not hidden, by name; none when it cannot be read.
fn entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut entries: Vec<(String, PathBuf)> = read
        .filter_map(Result::ok)
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
        .filter(|(name, _)| !name.starts_with('.'))
        .collect();
    entries.sort();
    entries
}

/// The enabled plugins' names and install directories: `enabledPlugins` in `<config>/settings.json` (`name@market:
/// true`), looked up in `<config>/plugins/installed_plugins.json`. An install scoped to a project counts only for
/// that project (`cwd`).
fn plugins(config: &Path, cwd: Option<&Path>) -> Vec<(String, PathBuf)> {
    let (Some(settings), Some(installed)) = (
        json(&config.join("settings.json")),
        json(&config.join("plugins/installed_plugins.json")),
    ) else {
        return Vec::new();
    };
    let Some(enabled) = settings.get("enabledPlugins").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut found: Vec<(String, PathBuf)> = enabled
        .iter()
        .filter(|(_, on)| on.as_bool() == Some(true))
        .filter_map(|(key, _)| {
            let name = key.split('@').next().filter(|name| !name.is_empty())?;
            let root = install_path(installed.pointer("/plugins")?.get(key)?, cwd)?;
            Some((name.to_owned(), config.join("plugins").join(root)))
        })
        .collect();
    found.sort();
    found
}

/// An `installed_plugins.json` entry's install directory: one install (version 1) or a list of them (version 2), the
/// first that is not scoped to another project.
fn install_path(entry: &Value, cwd: Option<&Path>) -> Option<PathBuf> {
    let installs = match entry {
        Value::Array(installs) => installs.iter().collect(),
        install => vec![install],
    };
    installs.into_iter().find_map(|install| {
        let project = install.get("projectPath").and_then(Value::as_str);
        if project.is_some_and(|project| cwd != Some(Path::new(project))) {
            return None;
        }
        install
            .get("installPath")
            .and_then(Value::as_str)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
    })
}

fn json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

/// A skill or command file: its frontmatter's fields and its body.
struct Doc {
    fields: Vec<(String, String)>,
    body: String,
}

impl Doc {
    /// The file at `path`, or `None` when it cannot be read, is not UTF-8, or opens a frontmatter it never closes.
    fn read(path: &Path) -> Option<Self> {
        let mut bytes = Vec::new();
        fs::File::open(path).ok()?.take(FILE_CAP).read_to_end(&mut bytes).ok()?;
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            // a cut through a character at the cap is not a bad file
            Err(error) if error.utf8_error().error_len().is_none() => {
                let valid = error.utf8_error().valid_up_to();
                let mut bytes = error.into_bytes();
                bytes.truncate(valid);
                String::from_utf8(bytes).ok()?
            }
            Err(_) => return None,
        };
        Self::parse(&text)
    }

    fn parse(text: &str) -> Option<Self> {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let mut lines = text.lines();
        if lines.clone().next().map(str::trim_end) != Some("---") {
            return Some(Self {
                fields: Vec::new(),
                body: text.to_owned(),
            });
        }
        lines.next();
        let mut head = Vec::new();
        loop {
            let line = lines.next()?;
            if line.trim_end() == "---" {
                break;
            }
            head.push(line);
        }
        Some(Self {
            fields: frontmatter(&head),
            body: lines.collect::<Vec<_>>().join("\n"),
        })
    }

    fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
    }

    /// The entry this file makes as `name`: the frontmatter's description, else the body's first line.
    fn command(&self, name: String, kind: CommandKind) -> SlashCommand {
        let description = self
            .field("description")
            .filter(|text| !text.is_empty())
            .map_or_else(|| first_line(&self.body), str::to_owned);
        SlashCommand {
            name,
            description: super::cap(&description, DESCRIPTION_CAP).0,
            kind,
            argument_hint: self
                .field("argument-hint")
                .filter(|hint| !hint.is_empty())
                .map(str::to_owned),
        }
    }
}

/// The body's first non-blank line, without a heading's `#`s.
fn first_line(body: &str) -> String {
    body.lines()
        .map(|line| line.trim().trim_start_matches('#').trim())
        .find(|line| !line.is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// The top-level `key: value` fields of a frontmatter, enough YAML for what skills and commands write: plain and
/// quoted scalars, a block scalar (`|`, `>`) or a plain one continued on indented lines, joined with spaces into one
/// line. Anything else (a list, a nested map) is kept as its text or skipped, and so is a comment.
fn frontmatter(lines: &[&str]) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in lines {
        let more = line.trim();
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = fields.last_mut().filter(|_| !more.is_empty()) {
                let gap = if value.is_empty() { "" } else { " " };
                value.push_str(gap);
                value.push_str(more);
            }
            continue;
        }
        if more.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        let value = if value.starts_with(['|', '>']) { "" } else { value };
        fields.push((key.trim().to_owned(), value.to_owned()));
    }
    fields.into_iter().map(|(key, value)| (key, unquote(&value))).collect()
}

/// A scalar without its quotes: `"…"` with `\"` and `\\` unescaped, `'…'` with `''` as one quote.
fn unquote(value: &str) -> String {
    if let Some(inner) = value.strip_prefix('"').and_then(|rest| rest.strip_suffix('"')) {
        return inner.replace("\\\"", "\"").replace("\\\\", "\\");
    }
    if let Some(inner) = value.strip_prefix('\'').and_then(|rest| rest.strip_suffix('\'')) {
        return inner.replace("''", "'");
    }
    value.to_owned()
}
