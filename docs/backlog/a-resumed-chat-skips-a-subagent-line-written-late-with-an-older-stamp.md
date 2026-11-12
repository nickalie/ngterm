---
worth: yes
where: crates/ngterm-core/src/agent_chat/follow.rs:296
added: 2026-11-12
---
# A resumed chat skips a subagent line written late with an older stamp

`Follower::after` sends a subagent's lines again from `since`, the stamp of the last main line before the resume
cursor, to the minute, when the subagent's task call is not in the `CONTEXT` (64 KiB) read back before the cursor.
Claude Code writes some lines late with older stamps (a permission-gated call, the lines of a turn whose
`AskUserQuestion` waits). So a subagent line stamped more than a minute before `since`, written after the phone's last
`live`, is never sent on a resume: the phone shows the call's result without its call, or neither. One main API
request (`api-request-shape` lines average 87 KB) pushes the task call out of the context, so a background agent
working while the main agent goes on is the usual case.

Reproduce at the follower: main line stamped `12:03:30`, its task call more than `CONTEXT` before the cursor, take the
`live`, append a subagent line stamped `12:00:00`, then a fresh `Follower::after(live)` must include it (today it
doesn't). Likely fix: resume a subagent by its own file offset kept per subagent, not by stamps.
