---
worth: yes
where: mobile/android/app/src/androidTest/kotlin/com/nickalie/ngterm/android/e2e/SessionLinkDriver.kt
added: 2026-11-24
---
# The session pane e2e tests' sign-in can stall after Import

Seen 2026-11-24 on the `ngterm-phone` AVD, twice in about twelve sign-ins of the session pane e2e classes
(`ColorSchemeE2eTest`, `SessionLinkUnlistedE2eTest`), each time in a class that ran after other classes in the same
script run. The test typed the key, clicked Import, and `Done` (`key_done`) did not show in 60 s. The activity was
resumed and the logs had no crash or ANR. The screenshot was black because the import screen sets `FLAG_SECURE`, so
what the screen showed is not known. The next run of the same classes passed.

The driver now logs the screen's semantics texts on a failure. When it occurs again, read the `screen at the failure`
log line: it shows whether the import screen gave a problem, was still busy, or did not get the key.
