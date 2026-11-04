//! The slash-command catalog on temp config trees: skills, commands and plugins found, hidden or skipped, the config
//! dir and cwd taken from the transcript, and duplicates resolved as the agents resolve them.

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;
use tempfile::TempDir;

use crate::agent_chat::AgentKind;
use crate::agent_chat::catalog::{
    CLAUDE_BUILTINS, CODEX_BUILTINS, CommandKind, SlashCommand, catalog, claude, codex, transcript_cwd,
};

fn write(path: &Path, text: impl AsRef<[u8]>) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn skill(dir: &Path, name: &str, text: &str) {
    write(&dir.join(name).join("SKILL.md"), text);
}

/// The entries that are not built-ins.
fn custom(found: &[SlashCommand]) -> Vec<&SlashCommand> {
    found
        .iter()
        .filter(|command| command.kind != CommandKind::Builtin)
        .collect()
}

fn names(found: &[SlashCommand]) -> Vec<&str> {
    custom(found).iter().map(|command| command.name.as_str()).collect()
}

fn entry(found: &[SlashCommand], name: &str) -> SlashCommand {
    found
        .iter()
        .find(|command| command.name == name)
        .cloned()
        .unwrap_or_else(|| panic!("{name} is not in the catalog"))
}

fn command(name: &str, description: &str, kind: CommandKind, hint: Option<&str>) -> SlashCommand {
    SlashCommand {
        name: name.to_owned(),
        description: description.to_owned(),
        kind,
        argument_hint: hint.map(str::to_owned),
    }
}

/// A Claude config dir and a project, side by side in a temp dir.
struct Tree {
    _root: TempDir,
    config: PathBuf,
    project: PathBuf,
}

fn tree() -> Tree {
    let root = tempfile::tempdir().unwrap();
    let config = root.path().join("claude-config");
    let project = root.path().join("project");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&project).unwrap();
    Tree {
        config,
        project,
        _root: root,
    }
}

#[test]
fn user_and_project_skills_are_listed_and_a_hidden_or_incomplete_one_is_not() {
    let tree = tree();
    let user = tree.config.join("skills");
    skill(
        &user,
        "deploy",
        "---\nname: deploy\ndescription: Ship the build\nargument-hint: \"[env]\"\n---\n\nSteps.\n",
    );
    skill(&user, "plain", "# Plain skill\n\nNo frontmatter at all.\n");
    skill(
        &user,
        "quiet",
        "---\nname: quiet\ndescription: Model only\nuser-invocable: false\n---\n",
    );
    write(&user.join("no-skill/README.md"), "a directory without a SKILL.md");
    skill(&user, ".hidden", "---\nname: hidden\n---\n");
    skill(
        &tree.project.join(".claude/skills"),
        "lint",
        "---\nname: lint\ndescription: >-\n  Lint the\n  project\n---\n",
    );

    let found = claude(&tree.config, Some(&tree.project));
    assert_eq!(names(&found), ["deploy", "plain", "lint"]);
    assert_eq!(
        entry(&found, "deploy"),
        command("deploy", "Ship the build", CommandKind::Skill, Some("[env]"))
    );
    assert_eq!(entry(&found, "plain").description, "Plain skill");
    assert_eq!(entry(&found, "lint").description, "Lint the project");

    // without a cwd, only the user's
    assert_eq!(names(&claude(&tree.config, None)), ["deploy", "plain"]);
}

#[test]
fn a_skill_is_named_by_its_frontmatter_else_its_directory() {
    let tree = tree();
    skill(
        &tree.config.join("skills"),
        "dir-name",
        "---\nname: 'front name'\ndescription: \"Says \\\"hi\\\"\"\n---\n",
    );
    skill(
        &tree.config.join("skills"),
        "nameless",
        "---\ndescription: No name\n---\n",
    );
    let found = claude(&tree.config, None);
    assert_eq!(names(&found), ["front name", "nameless"]);
    assert_eq!(entry(&found, "front name").description, "Says \"hi\"");
}

#[test]
fn a_trailing_comment_is_not_part_of_a_frontmatter_value() {
    let tree = tree();
    let user = tree.config.join("skills");
    skill(
        &user,
        "deploy",
        "---\nname: \"deploy\" # public name\ndescription: Ship it # note\nargument-hint: issue#42\n---\n",
    );
    skill(
        &user,
        "quote",
        "---\nname: 'it''s #1' # c\ndescription: \"say \\\"#2\\\"\"\t# c\n---\n",
    );
    skill(
        &user,
        "quiet",
        "---\nname: quiet\nuser-invocable: false # model only\n---\n",
    );
    skill(&user, "empty", "---\nname: # none\ndescription: |\n  a # kept\n---\n");

    let found = claude(&tree.config, None);
    assert_eq!(names(&found), ["deploy", "empty", "it's #1"]);
    assert_eq!(
        entry(&found, "deploy"),
        command("deploy", "Ship it", CommandKind::Skill, Some("issue#42"))
    );
    assert_eq!(entry(&found, "it's #1").description, "say \"#2\"");
    // a block scalar's `#` is text
    assert_eq!(entry(&found, "empty").description, "a # kept");
}

#[cfg(unix)]
#[test]
fn a_commands_walk_ends_on_link_cycles_and_skips_special_files() {
    use std::os::unix::fs::symlink;
    use std::sync::mpsc;
    use std::time::Duration;

    let tree = tree();
    let user = tree.config.join("commands");
    write(&user.join("x.md"), "X.\n");
    write(&user.join("sub/y.md"), "Y.\n");
    for at in 0..20 {
        symlink(".", user.join(format!("self{at}"))).unwrap();
    }
    // a second way into a directory walked already
    symlink("sub", user.join("twin")).unwrap();
    symlink("..", user.join("sub/up")).unwrap();
    // a linked file is read; a FIFO, a link to one or a device is not
    symlink("x.md", user.join("alias.md")).unwrap();
    let fifo = user.join("wait.md");
    assert!(
        std::process::Command::new("mkfifo")
            .arg(&fifo)
            .status()
            .unwrap()
            .success()
    );
    symlink(&fifo, user.join("fifo-link.md")).unwrap();
    symlink("/dev/null", user.join("null.md")).unwrap();
    let skills = tree.config.join("skills/blocked");
    fs::create_dir_all(&skills).unwrap();
    symlink(&fifo, skills.join("SKILL.md")).unwrap();

    let config = tree.config.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || sender.send(claude(&config, None)).unwrap());
    let found = receiver
        .recv_timeout(Duration::from_secs(20))
        .expect("the catalog never returned");
    assert_eq!(names(&found), ["alias", "sub:y", "x"]);
}

#[test]
fn a_nested_command_keeps_a_name_a_plugin_also_makes() {
    let tree = tree();
    write(&tree.config.join("commands/tools/lint.md"), "My lint.\n");
    let tools = tree.config.join("plugins/cache/market/tools/1.0.0");
    skill(
        &tools.join("skills"),
        "lint",
        "---\nname: lint\ndescription: Plugin lint\n---\n",
    );
    skill(&tools.join("skills"), "fmt", "---\nname: fmt\n---\n");
    write(
        &tree.config.join("settings.json"),
        json!({"enabledPlugins": {"tools@market": true}}).to_string(),
    );
    write(
        &tree.config.join("plugins/installed_plugins.json"),
        json!({"version": 2, "plugins": {"tools@market": [{"scope": "user", "installPath": tools}]}}).to_string(),
    );

    let found = claude(&tree.config, None);
    assert_eq!(names(&found), ["tools:lint", "tools:fmt"]);
    assert_eq!(
        entry(&found, "tools:lint"),
        command("tools:lint", "My lint.", CommandKind::Command, None)
    );
}

#[test]
fn nested_custom_commands_are_named_by_their_path() {
    let tree = tree();
    let user = tree.config.join("commands");
    write(&user.join("review.md"), "Review the diff.\n");
    write(
        &user.join("git/commit.md"),
        "---\ndescription: Commit staged work\nargument-hint: <message>\n---\nCommit.\n",
    );
    write(&user.join("a/b/c.md"), "\n\n## Deep one\n");
    write(&user.join("notes.txt"), "not a command");
    write(&tree.project.join(".claude/commands/team/sync.md"), "Sync the team.\n");

    let found = claude(&tree.config, Some(&tree.project));
    assert_eq!(names(&found), ["a:b:c", "git:commit", "review", "team:sync"]);
    assert_eq!(
        entry(&found, "git:commit"),
        command(
            "git:commit",
            "Commit staged work",
            CommandKind::Command,
            Some("<message>")
        )
    );
    assert_eq!(entry(&found, "a:b:c").description, "Deep one");
    assert_eq!(entry(&found, "review").description, "Review the diff.");
}

#[test]
fn only_enabled_plugins_are_listed_with_their_prefix() {
    let tree = tree();
    let cache = tree.config.join("plugins/cache/market");
    let tools = cache.join("tools/1.0.0");
    skill(
        &tools.join("skills"),
        "lint",
        "---\nname: lint\ndescription: Lint it\n---\n",
    );
    write(&tools.join("commands/fmt.md"), "---\ndescription: Format it\n---\n");
    write(&tools.join("commands/deep/check.md"), "Check it.\n");
    let off = cache.join("off/2.0.0");
    skill(&off.join("skills"), "nope", "---\nname: nope\n---\n");
    let elsewhere = cache.join("elsewhere/1.0.0");
    skill(&elsewhere.join("skills"), "far", "---\nname: far\n---\n");
    let legacy = cache.join("legacy/0.1.0");
    write(&legacy.join("commands/old.md"), "Old.\n");
    write(
        &tree.config.join("settings.json"),
        json!({"enabledPlugins": {
            "tools@market": true,
            "off@market": false,
            "elsewhere@market": true,
            "legacy@market": true,
            "uninstalled@market": true,
        }})
        .to_string(),
    );
    write(
        &tree.config.join("plugins/installed_plugins.json"),
        json!({"version": 2, "plugins": {
            "tools@market": [{"scope": "user", "installPath": tools, "version": "1.0.0"}],
            "off@market": [{"scope": "user", "installPath": off}],
            "elsewhere@market": [{"scope": "project", "projectPath": "/some/other/project", "installPath": elsewhere}],
            // version 1 kept one install per plugin
            "legacy@market": {"installPath": legacy},
        }})
        .to_string(),
    );

    let found = claude(&tree.config, Some(&tree.project));
    assert_eq!(
        names(&found),
        ["legacy:old", "tools:lint", "tools:deep:check", "tools:fmt"]
    );
    assert_eq!(
        entry(&found, "tools:lint"),
        command("tools:lint", "Lint it", CommandKind::Plugin, None)
    );
    assert_eq!(entry(&found, "tools:fmt").description, "Format it");
}

#[test]
fn codex_lists_its_skills_as_mentions_and_no_prompts() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("codex-home");
    skill(
        &home.join("skills"),
        "greet-check",
        "---\nname: greet-check\ndescription: Check the greeting\n---\n",
    );
    skill(
        &home.join("skills"),
        "quiet",
        "---\nname: quiet\nuser-invocable: false\n---\n",
    );
    write(
        &home.join("prompts/old.md"),
        "A custom prompt 0.166 no longer offers.\n",
    );

    let found = codex(&home);
    assert_eq!(names(&found), ["$greet-check"]);
    assert_eq!(
        entry(&found, "$greet-check"),
        command("$greet-check", "Check the greeting", CommandKind::Skill, None)
    );
    assert_eq!(entry(&found, "compact").kind, CommandKind::Builtin);
    assert_eq!(found.len(), CODEX_BUILTINS.len() + 1);
}

#[test]
fn the_config_dir_and_cwd_come_from_the_transcript() {
    let root = tempfile::tempdir().unwrap();
    // a custom CLAUDE_CONFIG_DIR
    let config = root.path().join("custom-claude");
    let project = root.path().join("work");
    let transcript = config.join("projects/-work/abc.jsonl");
    let line =
        json!({"type": "user", "sessionId": "abc", "cwd": project, "message": {"role": "user", "content": "hi"}});
    write(
        &transcript,
        format!(
            "{}\n{line}\n",
            json!({"type": "mode", "mode": "normal", "sessionId": "abc"})
        ),
    );
    skill(&config.join("skills"), "mine", "---\nname: mine\n---\n");
    skill(&project.join(".claude/skills"), "ours", "---\nname: ours\n---\n");
    assert_eq!(transcript_cwd(AgentKind::Claude, &transcript), Some(project.clone()));
    assert_eq!(
        names(&catalog(AgentKind::Claude, &transcript).unwrap()),
        ["mine", "ours"]
    );

    // a custom CODEX_HOME
    let home = root.path().join("my-codex");
    let rollout = home.join("sessions/2026/11/03/rollout-x.jsonl");
    let meta = json!({"type": "session_meta", "payload": {"id": "t1", "cwd": project}});
    write(&rollout, format!("{meta}\n"));
    skill(&home.join("skills"), "greet", "---\nname: greet\n---\n");
    assert_eq!(transcript_cwd(AgentKind::Codex, &rollout), Some(project));
    assert_eq!(names(&catalog(AgentKind::Codex, &rollout).unwrap()), ["$greet"]);

    // a path outside such a tree is refused, as the record refuses it
    let stray = root.path().join("stray.jsonl");
    write(&stray, "");
    let refused = catalog(AgentKind::Claude, &stray).unwrap_err();
    assert!(
        refused.starts_with("transcript path is outside a Claude projects/ tree"),
        "{refused}"
    );
}

#[test]
fn the_recorded_transcripts_name_their_cwd() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/agent_chat");
    let claude = fixtures.join("claude/projects/-home-user-project/f559ab9f-a5bd-4b2f-91e5-72590d662bf4.jsonl");
    let codex = fixtures
        .join("codex/sessions/2026/11/03/rollout-2026-11-03T23-01-46-01a1a300-e2e3-76a1-abcf-961cbbb2eb3c.jsonl");
    let project = Some(PathBuf::from("/home/user/project"));
    assert_eq!(transcript_cwd(AgentKind::Claude, &claude), project);
    assert_eq!(transcript_cwd(AgentKind::Codex, &codex), project);
    // a transcript not written yet
    assert_eq!(transcript_cwd(AgentKind::Claude, &fixtures.join("missing.jsonl")), None);
}

#[test]
fn an_unreadable_or_malformed_file_is_skipped() {
    let tree = tree();
    let user = tree.config.join("skills");
    skill(&user, "good", "---\nname: good\n---\n");
    write(&user.join("binary/SKILL.md"), b"---\nname: bin\xff\xfe\n---\n");
    skill(&user, "unclosed", "---\nname: unclosed\ndescription: never ends\n");
    // a directory where the file should be
    fs::create_dir_all(user.join("dir/SKILL.md")).unwrap();
    write(&tree.config.join("commands/bad.md"), b"\xff not text");
    write(&tree.config.join("commands/fine.md"), "Fine.\n");
    write(&tree.config.join("settings.json"), "{ not json");
    write(&tree.config.join("plugins/installed_plugins.json"), "[]");

    assert_eq!(names(&claude(&tree.config, None)), ["good", "fine"]);
}

#[test]
fn a_duplicate_name_keeps_the_builtin_then_the_personal_then_the_skill() {
    let tree = tree();
    let user = tree.config.join("skills");
    let project = tree.project.join(".claude");
    skill(&user, "dup", "---\nname: dup\ndescription: personal\n---\n");
    skill(
        &project.join("skills"),
        "dup",
        "---\nname: dup\ndescription: project\n---\n",
    );
    skill(
        &user,
        "compact",
        "---\nname: compact\ndescription: shadowing a built-in\n---\n",
    );
    write(&tree.config.join("commands/same.md"), "the command\n");
    skill(
        &project.join("skills"),
        "same",
        "---\nname: same\ndescription: the skill\n---\n",
    );
    write(&tree.config.join("commands/both.md"), "personal command\n");
    write(&project.join("commands/both.md"), "project command\n");

    let found = claude(&tree.config, Some(&tree.project));
    assert_eq!(names(&found), ["dup", "same", "both"]);
    assert_eq!(entry(&found, "dup").description, "personal");
    assert_eq!(entry(&found, "same").kind, CommandKind::Skill);
    assert_eq!(entry(&found, "both").description, "personal command");
    assert_eq!(entry(&found, "compact").kind, CommandKind::Builtin);
}

#[test]
fn the_builtins_lead_once_each_and_serialize_as_the_phone_reads_them() {
    let tree = tree();
    let found = claude(&tree.config, None);
    assert_eq!(found.len(), CLAUDE_BUILTINS.len());
    for list in [CLAUDE_BUILTINS, CODEX_BUILTINS] {
        let mut names: Vec<&str> = list.iter().map(|builtin| builtin.0).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), list.len(), "a built-in is listed twice");
    }
    assert_eq!(
        serde_json::to_value(entry(&found, "compact")).unwrap(),
        json!({
            "name": "compact",
            "description": "Free up context by summarizing the conversation so far",
            "kind": "builtin",
            "argument_hint": "<optional custom summarization instructions>",
        })
    );
    assert_eq!(
        serde_json::to_value(entry(&found, "help")).unwrap(),
        json!({"name": "help", "description": "Show help and available commands", "kind": "builtin"})
    );
}
