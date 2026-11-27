---
worth: yes
where: crates/ngterm-core/src/remote/catalog/mod.rs:25
added: 2026-11-27
---
# The catalog stream is written twice, for the desktop remote window and for the mobile apps

`ngterm-core`'s `remote::catalog` has `RETRY_DELAYS`, `SILENCE_LIMIT`, `retry_delay`, `CatalogStream`, `CatalogState`
and `Received`. `ngterm-mobile/src/catalog.rs` has the same constants and its own `Mirror`, with the same fields and
the same frame handling: the revision step, the hello epoch check and the resubscribe. `ngterm-mobile` already uses
`ngterm_core::catalog::frames`. Thus a fix to the revision or epoch handling must go into two places, or the two
copies become different.

Possible fix: make `ngterm-mobile` use the constants and `retry_delay` of `ngterm_core::remote::catalog`, then build
the mobile `Mirror` on `CatalogStream`. Do this in one change with the mobile tests. The `Mirror` is internal to the
mobile actor, but `CatalogState` is in the UniFFI surface of the Android and iOS apps: to use the `CatalogState` of
`ngterm-core` there, add the UniFFI derive to it or map it.

Reported in the revmux review of task #318 (the remote window's core), round 1.
