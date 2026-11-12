---
worth: yes
where: mobile/ios/Packages/NgtermFeatures/Tests/NgtermFeaturesTests/ScreenSnapshot.swift:20
added: 2026-11-12
---
# iOS spinner goldens catch the spinner mid-turn

`StatePanel.loading`'s `ProgressView` (`NgtermDesign/Components/StatePanel.swift:41`) is drawn at whatever phase its
animation has reached when `ScreenSnapshot.check` renders the screen. Its goldens fail now and then on the phone
simulator. On a Mac run of `SessionsSnapshotTests` at 4214cea9 (ngterm-phone, iOS 27.0), two goldens failed:
`creating(isEmpty: true)` (`sessions-empty-creating-light-on-phone`) and `problems(.reading)`
(`sessions-reading-light-on-phone`). Each differed by 0.005% of pixels, with a perceptual precision of 0.908 against
the required 0.98, and only the spinner's spokes showed in the diff image. The tablet passed, and an earlier run of
the same suite at da1f79b9 passed on both simulators.

Fix: render spinners frozen in snapshots, for example with a static stand-in for `ProgressView` under a snapshot
environment value, or with the animation stopped before the render. Don't loosen the precision. Every golden that
shows a `ProgressView` (`Cover`, `SessionRow`, `StatePanel`, `NewSessionSheet`) has the same exposure.
