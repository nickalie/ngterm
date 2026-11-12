---
worth: yes
where: crates/ngterm-core/src/agent_chat/follow.rs:234
added: 2026-11-12
---
# A long subagent's early steps can't be paged to

A subagent shown in the backlog gets its last `BACKLOG` events, and an older page gives each subagent in it its last
`OLDER_PAGE` events; paging only follows the main transcript's cursors, so nothing ever reads further back into a
subagent's own file. A review or research subagent with more than about 200 events keeps its first steps out of the
chat for good. Reproduce: a subagent file of 300 one-event lines; the backlog plus every older page must cover all
300 (today the first ~100 never come). Likely fix: a per-subagent `--before` (its own cursor) behind the task row's
"earlier steps" affordance.
