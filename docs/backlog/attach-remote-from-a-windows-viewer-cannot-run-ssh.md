---
worth: yes
where: crates/ngterm-core/src/remote/tree.rs:173
added: 2026-11-10
---
# Attach Remote cannot start from a Windows viewer

The viewer side runs ssh through Unix-only forms: `ProcessRemoteRunner` execs `/usr/bin/env`, and `attach_pane_command`
builds a `/usr/bin/env /bin/sh -c '…'` line for the pane. On Windows neither exists, so a Windows machine cannot run
Attach Remote against any origin, and the mobile apps and Unix desktops are the only viewers. Windows origins do work
from those (`crates/ngterm-core/src/remote/origin.rs`). The fix is a Windows viewer form for each: `ssh` run directly,
and a pane command with the keep-alive probe and reconnect bar in Windows terms.
