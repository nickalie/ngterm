#!/usr/bin/env bash
# ngterm-agent-mail — hand the pane's mail to the Claude Code that runs in it.
#
#   ngterm-agent-mail.sh claude          # Claude Code PostToolUse: the mail as added context
#   ngterm-agent-mail.sh claude --wait   # Claude Code SessionStart and Stop, asyncRewake: wake an idle agent
#
# Installed by ngterm's Help ▸ Install Agent Status Hooks command. An ngterm extension beyond upstream, whose agents
# read their mailbox (`ngtermctl msg read`) only when they decide to.
#
# Another program leaves a message in this pane's mailbox with `ngtermctl msg send --to SESSION:PANE`; this hands it to
# the pane's agent, so a dispatcher can steer a worker session, or a worker report back, without typing into a
# composer. The work is `ngtermctl agent mail claude`, which reads the hook payload on stdin, takes the mailbox's
# messages in one read+ack and prints the hook's answer: added context on PostToolUse, and with --wait it waits in the
# background, then prints the mail on stderr and exits 2, which wakes an idle Claude Code with it. Only the pane's
# interactive agent takes mail: a subagent's hook and a `claude -p` worker (which inherits the pane's NGTERM_*) take
# none. A wake also turns the pane's status back to active, since a woken turn fires no UserPromptSubmit.
#
# It execs ngtermctl, so a waiter's parent is the Claude Code that started it: an agent that exits or dies without
# ending its hooks leaves the waiter re-parented, and it stops before taking mail nobody would read. Its stdout and
# stderr are the hook's answer, so it adds nothing of its own; outside ngterm (NGTERM_SESSION_ID or NGTERM_PANE_ID
# unset) it does nothing and exits 0. Codex's mail goes through its lifecycle adapter instead, whose hook lines Codex
# has already trusted.
#
# ngtermctl resolution: $NGTERMCTL, else the path the installer bakes in, else `ngtermctl` on PATH.
set -u

# A background job of Claude Code's daemon (CLAUDE_JOB_DIR is set: Claude Code moves a conversation there on
# /background or on a Left arrow at its empty prompt) has the daemon's NGTERM_*, which name the pane the daemon was
# started in: a session long gone, or another one's. `ngtermctl agent rerun` finds the pane that holds this
# conversation from the payload and runs this hook again as that pane; with none it runs nothing, silently.
if [ -n "${CLAUDE_JOB_DIR:-}" ] && [ -z "${NGTERM_AGENT_LOCATED:-}" ]; then
  command -v "${NGTERMCTL:-ngtermctl}" >/dev/null 2>&1 || exit 0
  rerun=(agent rerun claude)
  [ -n "${NGTERM_SOCKET:-}" ] && rerun+=(--socket "$NGTERM_SOCKET")
  exec "${NGTERMCTL:-ngtermctl}" "${rerun[@]}" -- "${BASH:-bash}" "$0" "$@"
fi

[ -n "${NGTERM_SESSION_ID:-}" ] && [ -n "${NGTERM_PANE_ID:-}" ] || exit 0

args=(agent mail "$@")
[ -n "${NGTERM_SOCKET:-}" ] && args+=(--socket "$NGTERM_SOCKET")

exec "${NGTERMCTL:-ngtermctl}" "${args[@]}"
