---
worth: yes
where: conformance/tests/ui/test_kitty_clipboard_prompt.py
added: 2026-11-12
---
# Closing a Live session right after its Kitty read was denied at the prompt once stalled the app for 18 s

`test_an_asked_read_waits_for_the_prompt_and_escape_denies_it[live]` failed once in about ten local runs on
2026-11-12 (instrumented build, ui layer on Xvfb, load average 44–65 from other sessions' builds): the read was denied
at its prompt (the probe echoed `type=read:status=EPERM`), then the test's `session.close` got no answer within 10 s.
The app logged nothing for 18 s after the prompt's focus change (19:59:22 to 19:59:40), then answered again. Five
reruns of that case alone at load 38–61 passed. Find what the main thread waits on once a Live pane's read prompt
closes and its session is closed (the bridge's answer to the session server, the alert's focus handoff through
AT-SPI, or the close of the Live daemon), reproduce it with a test that shows the stall on the unfixed code, and fix it.
