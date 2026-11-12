---
worth: yes
where: crates/ngterm-core/src/agent_chat/follow.rs:77
added: 2026-11-12
---
# A resume whose call lies past the 64 KiB context pairs its result wrongly

`after()` parses only `CONTEXT` (64 KiB) before the resume cursor. In this machine's real transcripts 89 of 3,443
results or task notifications came more than 64 KiB after their call (background `Bash`, `Agent`, `ToolSearch`). A
resume then shows a Claude background task's launch receipt as an untitled finished tool (the task is still running),
and a Codex `CommandExecution` whose call is out of the window as a row of its own, so the call shows twice. Likely fix:
on `--after`, read back further for the ids still running (the backward scan already stops at the main agent's last
prompt for a live approval: `Follower::expect`).
