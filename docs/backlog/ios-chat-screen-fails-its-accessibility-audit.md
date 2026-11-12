---
worth: yes
where: mobile/ios/Packages/NgtermFeatures/Sources/NgtermFeatures/Chat/Views/ChatComposerView.swift:38
added: 2026-11-12
---
# the iOS chat screen fails its accessibility audit with a question card up

Task #13 ran `FakeApp.audit` over the chat on the iPhone simulator with Claude's question card up (the fake core's
`-ngterm-fake-chat`, prompt "ask me"). It failed, on the unfixed and the fixed code alike, so `ChatTests` doesn't audit
yet:

- Contrast: the composer's "Answer the card above first: a prompt now would go to its dialog." (`secondaryText`
  footnote on the bar material) and the question card's **Send answers** while it is disabled (nothing picked yet).
- Dynamic Type "partially unsupported": the session title's `work` and `working` in the toolbar. On the phone that
  title also wraps the state word letter by letter ("wor / kin / g") beside the pane tabs, which the recordings show.
  It may share a cause with `ios-machines-list-audit-flags-session-rows-as-not-scaling.md`.

The chat screen had no audit before, so these are older than #13. Fix them, then add `try FakeApp.audit(app)` back to
`ChatTests.testClaudesQuestionIsAQuestionCardAnsweredThereAndAskedAgain` after the card shows.
