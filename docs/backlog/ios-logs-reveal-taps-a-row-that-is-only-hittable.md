---
worth: yes
where: mobile/ios/NgTermUITests/LogsPage.swift:174
added: 2026-11-26
---
# The iOS Logs tests tap a Settings row that is only hittable

`LogsPage.reveal` swipes Settings' form until the row is `exists && isHittable`, and its callers then tap the row. On
an iPad sheet a row with only a sliver above the foot of the sheet is hittable, and a tap at its hit point lands
outside the sheet and closes it (`FormScroll` tells why). The other Settings scrolls of the UI tests use
`FormScroll.scroll`, which brings the row wholly into view. `reveal` also scrolls back up after `swipes` swipes,
which `FormScroll.scroll` does not do.

Suggested fix: give `FormScroll` a scroll that goes down to the foot and back up, with drags at the end, and use it
in `reveal`. Look at the separate LogsTests flake (Settings closed after the taps on the password offer's Not Now)
in the same change.
