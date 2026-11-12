---
worth: yes
where: mobile/ios/NgTermUITests/SessionViewTests.swift:92
added: 2026-11-09
---
# the iOS audit of the machines list now and then flags a session row's text as not scaling

`SessionViewTests/testTheMachinesListTheAttachedSessionAndOpenIt` failed its accessibility audit on the iPhone and
passed on its retry, which fails the `ios` job (run 42623785962, commit 373d48e5, seen once). The audit reported
"Dynamic Type font sizes are partially unsupported" for the `work` and `working` labels of the attached session's row
(`SidebarSessionRow`), and nothing for the machine's own row above it.

The cause isn't proven. The design's text styles are Dynamic Type styles, so it doesn't look like a font that really
doesn't scale. The audit runs on a list reached by going back twice, with no `FakeApp.settle` first, and that is the
first thing to try. Reproduce it by repeating the test on a simulator before changing anything.
