---
worth: yes
where: mobile/ios/NgTermUITests/FakeApp.swift:102
added: 2026-11-12
---
# The iOS UI tests' Add machine tap can miss the Host field

`SessionsTests.testNewSessionLandsInItThenEndsFromTheList` failed once in four runs on ngterm-phone on 2026-11-12,
before any session step, in `FakeApp.addMachine`: `hostField.typeText` failed with "Neither element nor any
descendant has keyboard focus". The tap on `machine-host` had been synthesized, then waiting for the app to idle took
22 s (launch took 40 s): the simulator was starved, and the tap gave the field no focus. The form doesn't focus a field
by itself, so the helper should wait for the field's `hasKeyboardFocus` after the tap and tap again only when it stays
unfocused. Every UI test that adds a machine goes through this helper. Surfaced while checking the iOS one-tap New
session change, which doesn't touch the machine form or the helper, so it was left as it was.
