---
worth: yes
where: mobile/ios/NgTermUITests/SessionViewTests.swift:25
added: 2026-11-13
---
# The iOS session-view UI tests' tap on the machine row can leave the app on the Machines list

`SessionViewTests/testEndClosesTheSessionView` fails on the iPhone and passes on its retry, which fails the `ios`
job. Sightings:

- Main run 42909999252 (7188be45, 2026-11-10): an `XCTWaiter` timeout.
- Main run 43319782118 (b4809a35, 2026-11-12): "no the session's row", then the tap on `session-work` found no
  element.

In the second run the failure's UI hierarchy is still the Machines list: `machine-row-split` reads "Connected, 1 idle",
so `openSession` tapped the row, but the app never pushed the machine's sessions. `AskTests.openSession` uses the same
two steps: `expectRow` then `tap()` on `machine-row-split`.

The cause isn't proven. The likely cause is a tap that lands while the list is still settling after the row turned
Connected. A fix to try is to wait for the sessions list after the tap, and tap again only if it never shows. Check the
fix by repeating the test on a simulator, the way `the-ios-ui-tests-add-machine-tap-can-miss-the-host-field.md` handles
its tap.
