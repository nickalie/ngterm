---
worth: yes
where: crates/ngterm-core/src/remote/tree.rs:81
added: 2026-10-31
---
# Attach Remote cannot reach a Windows origin

Every command Attach Remote runs on the origin over ssh (`live tree`, the pane's attach, `live present`, the overlay
job, `page-serve`) is wrapped as `/bin/sh -c '<PATH chain> && ngtermctl …'` (`remote_cli_chain`, and `ssh.rs`'s
note "A Windows origin needs its own remote form"). Windows' OpenSSH server runs `cmd.exe` (or PowerShell) and has no
`/bin/sh`, so a Windows machine running the app or `ngterm-headless` builds, serves its control pipe and live
sessions, and still cannot be attached from another machine. Surfaced while merging `ngterm-headless` (a display-less
Attach Remote origin, built and tested on all three OSes) onto main: its Windows form has no viewer yet. The fix is a
Windows remote form for each of those invocations, chosen by probing the origin (or a `remotes.conf` hint), proven on
a Windows runner with its OpenSSH server.
