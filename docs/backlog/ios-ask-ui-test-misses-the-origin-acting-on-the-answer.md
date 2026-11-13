---
worth: yes
where: mobile/ios/NgTermUITests/AskTests.swift:33
added: 2026-11-13
---
# The iOS Ask UI test now and then misses the origin acting on the answer

`AskTests/testAnsweringTheAskInTheApp` fails on the iPhone and passes on its retry, which fails the `ios` job. It
failed the same way in main runs 42692831582 (3c6f762b, 2026-11-09) and 43319782118 (b4809a35,
2026-11-12): "no the origin acting on it". The ask's sheet closed after the tap on `ask-button-deploy`, and the session view
was back. But the session's context still read "Split the guide" and never turned to "Deploying to production", the
context the fake core sends for that answer (`FakeSessionWorld.swift`).

The cause isn't proven. Either the fake core's reply to the answer is lost or comes late, or the session title's subtitle
misses the context update. Before changing anything, reproduce it by repeating the test on a simulator. Then check
whether the answer reaches `FakeMobileCore` and whether the context event is delivered.
