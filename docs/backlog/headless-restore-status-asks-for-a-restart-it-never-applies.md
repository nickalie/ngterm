---
worth: yes
where: crates/ngterm-ctl/src/output.rs:172
added: 2026-10-31
---
# ngterm-headless's restore status asks for a restart that would change nothing

`ngterm-headless` runs every launch live whatever the saved restore mode, and leaves the saved mode as the user set
it (so an app later started on the same state dir follows the user's choice). With a saved mode other than `live`,
`ngtermctl restore mode` against it prints `restart ngterm to apply the configured mode`, but a restart of the
headless instance runs live again. Cosmetic: nothing misbehaves, the hint is just wrong there. The fix is a core-side
`restart_required` that knows the host forces the mode (a flag on the launch decision), or a headless line instead.
