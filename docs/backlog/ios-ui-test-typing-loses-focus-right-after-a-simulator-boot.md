---
worth: later
where: mobile/ios/NgTermUITests/ChatTests.swift:29
added: 2026-11-13
---
# an iOS UI test's first typing can lose the keyboard focus right after the simulator boots

Seen once on the Mac, fix #84 of task #13: `ChatTests.testACardAnsweredWhileTheLinkWasDownGoesAndSendWorksAgain` was
the first test of a run on a just-booted `ngterm-phone`. Its tap on `chat-prompt` drew the cursor, but `typeText`
failed three times with "Neither element nor any descendant has keyboard focus", and no keyboard was in the hierarchy.
The simulator's log at that moment shows PosterBoard's extensions (Emoji, Pride, Gradient posters) still registering
keyboard clients with the KeyboardArbiter, and the app reporting `_hardwareKeyboardAvailable = YES`. The same test
passed alone on a cold-booted phone right after, and the whole class passed on the iPad in that run.

CI runs the UI plan after the Unit plan, which has already booted the simulators, so it may never meet this. If it
shows up there: before the UI plan, boot each simulator and wait until it is idle (`xcrun simctl bootstatus -b`), or
have the tests' first tap wait for the field's keyboard focus and tap again.
