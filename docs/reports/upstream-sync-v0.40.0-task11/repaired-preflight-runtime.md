## rust-linux conformance

- build: a99d1494 (dirty), `/home/nick/develop/projects/my/ngterm/target/conformance-stage`
- mode: full
- suite time: 35s
- ngterm: `/home/nick/develop/projects/my/ngterm/target/conformance-stage/bin/ngterm`
- Vulkan: lavapipe only, `/home/nick/.cache/ngterm/vendor/lavapipe-13261478a29d822b/lvp_icd.json`
- host: Linux 7.3.1-1-cachyos-server, 2026-11-04T08:23:00Z
- layers: protocol cli behaviors ui
- display: headless
- pytest args: -k restart_drops_mail_sent_during_shutdown or session_info_display_picker_persists
- pytest status: 1
- exit status: 0

```
rust: 1 pass, 1 fail, 0 skip(unsupported), 0 skip(not-yet-at-level), 0 skip(other), 0 reruns, parity 50.0%
FAILED conformance/tests/protocol/test_messages.py::test_restart_drops_mail_sent_during_shutdown
=========== 1 failed, 1 passed, 994 deselected, 1 warning in 34.44s ============
```

### ratchet

```
newly passing, not in the list (1):
tests/ui/test_settings.py::test_session_info_display_picker_persists
ratchet holds: 932 listed id(s), partial run
```

### catalog coverage

ui ids exercised by a passing test: 1/129 (0.8%)
