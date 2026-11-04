## rust-linux conformance

- build: ef023f23 (dirty), `/home/nick/develop/projects/my/ngterm/target/conformance-stage`
- mode: full
- suite time: 28s
- ngterm: `/home/nick/develop/projects/my/ngterm/target/conformance-stage/bin/ngterm`
- Vulkan: lavapipe only, `/home/nick/.cache/ngterm/vendor/lavapipe-13261478a29d822b/lvp_icd.json`
- host: Linux 7.3.1-1-cachyos-server, 2026-11-04T08:59:06Z
- layers: protocol cli behaviors ui
- display: headless
- pytest args: -k test_restart_drops_mail_sent_during_shutdown or test_restart_replaces_the_pane_shell or test_restart_without_a_command_replays_the_foreground_program
- pytest status: 0
- exit status: 0

```
rust: 3 pass, 0 fail, 0 skip(unsupported), 0 skip(not-yet-at-level), 0 skip(other), 0 reruns, parity 100.0%
====================== 3 passed, 993 deselected in 27.13s ======================
```

### ratchet

```
ratchet holds: 988 listed id(s), partial run
```

### catalog coverage

ui ids exercised by a passing test: 0/129 (0.0%)
