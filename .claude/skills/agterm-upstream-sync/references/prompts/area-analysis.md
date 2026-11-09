# Feature analysis worker

The brief of one analysis worker per feature cluster (a command group, or one changelog entry with its commits), doing
read-only work (`spawn-worker.sh` adds the reporting protocol). Replace every `<...>` before saving it, including the
absolute `SKILL_DIR` and the answer file; the worker sees nothing of this skill.

```
You analyse one upstream agterm change for a port into ngterm (Rust, GPUI; macOS, Linux and Windows are equal
targets, while upstream is macOS only; the Android and iOS apps, remote-only viewers over `crates/ngterm-mobile`, are
targets too). You do NOT implement
anything and you do NOT edit, create or commit repository files: read, grep and run read-only commands only. The one
file you write is your answer, <ANSWER_FILE>. Do not run cargo (build, test, clippy): the gates share target/ and are
running or about to. Your answer is a draft plan task. Nobody is available: never ask, and mark what you cannot settle
as "unconfirmed" in RISKS.

Repository: <ROOT> (ngterm). Upstream clone (read-only): <UPSTREAM>, range <OLD>..<NEW>.
Read <ROOT>/AGENTS.md and any more specific directory instructions first (crate map, reference rule, porting order).
The cluster: <cluster title>
Upstream commits: <hash subject, one per line>
Changelog text: <the matching changelog bullets, verbatim>
Files by area (from the classify report):
<the cluster's lines of the report: path, +/-, tests, strings>

Do this:
1. Read the upstream change: `git -C <UPSTREAM> show <hash>` per commit, or
   `git -C <UPSTREAM> diff <OLD> <NEW> -- <path>` per file. Read the Swift tests in full for the cases and exact
   strings, `site/commands.html` and `.claude/rules/*.md` hunks for the contract. Note the validation order, every
   refusal string, defaults, read-back fields and field order, on-disk format changes, and what changed in the UI.
   Code that only moved (the report marks such strings `(moved)`) is not a behavior change: say so and skip it.
2. Find the Rust side: the files that hold this behavior today (`rg -n` in <ROOT>/crates for the command's wire
   name, its fields, the strings without Swift interpolation, the old behavior). Decide per behavior: already
   ported (cite file:line and the unit test), partly ported (what is missing), or absent.
3. Find the conformance coverage: run
   `cd <ROOT> && .venv/bin/python <SKILL_DIR>/scripts/coverage-map.py <catalog keys of this cluster>` (or
   `--grep <regex>`), and `rg -n` in <ROOT>/conformance/tests for the behavior. List the ids that cover it, which
   are in `conformance/rust-passing.txt`, and the tests still missing (propose file, function name, `covers` keys,
   and what each asserts). A UI-only behavior has no suite test on Linux: propose its UI item
   (the GPUI form) instead.
4. Platforms (AGENTS.md > Platforms): for a behavior upstream builds on a macOS API, name its mechanism on macOS,
   Linux and Windows, or why an OS has no counterpart. Upstream's `SessionHostRuntime` maps to
   `ngterm-session`/`ngterm-pty`.
   Mobile apps (<SKILL_DIR>/references/areas.md > Platforms and apps): decide whether the behavior reaches the
   Android and iOS apps. If it does, find where they get it (`rg -n` in <ROOT>/crates/ngterm-mobile,
   <ROOT>/mobile/android and <ROOT>/mobile/ios), what each lacks, and the screen that should show it; if not, give
   the one-line reason.
5. ngterm's own features: check whether the behavior overlaps something ngterm has that upstream lacks, or does
   differently by choice (<ROOT>/README.md "extension beyond upstream" and "differs from upstream by choice";
   <ROOT>/conformance/docs/rust-build.md, Rust-only extensions and deviations). ngterm's form always wins: propose
   only the part of upstream's change that fits it, adapted, and name what is left out. Never propose reworking an
   ngterm feature toward upstream.
6. Decide STATUS per behavior: `absent` (no Rust code); `partial` (part of it missing, or the code is there but no
   Rust unit test asserts what the Swift tests assert, or no conformance id covers a suite-testable behavior: then the
   task ports the tests); `already-ported` (code and tests both there: cite them); `no-behavior-change` (a move,
   rename or comment). Mark any claim you inferred from reading code without finding the deciding line as
   "unconfirmed" in RISKS.

Write your answer to <ANSWER_FILE> in this exact shape, nothing before or after, and report only that path with one
line per block (STATUS and TITLE). When the cluster holds independent changes, give one block per change, each
starting with STATUS:

STATUS: absent | partial | already-ported | no-behavior-change
TITLE: <task title: group or feature, with PR numbers>
UPSTREAM_FILES: <implementation paths, comma separated>
SWIFT_TESTS: <test file paths with the relevant function names>
CONTRACT: <commands.html section / rules files>
RUST_FILES: <files to create or modify, each with create|modify>
EVIDENCE: <for already-ported or partial: file:line and test ids that show it>
CONFORMANCE_EXISTING: <ids, each with listed|not-listed in rust-passing.txt>
CONFORMANCE_NEW: <proposed new tests, or none>
UI_ITEMS: <proposed UI-only items with their GPUI form, or none>
PLATFORMS: <the mechanism per OS (macOS / Linux / Windows) for each OS-specific behavior, or "no OS-specific code">
NGTERM_OWN: <the ngterm feature or deviation it overlaps, what of upstream's change is kept and what is left out, or
none>
MOBILE: <per app (Android / iOS): absent | partial | already-ported with evidence, the ngterm-mobile, Kotlin and
Swift files to change, or "not reached: <reason>">
ITEMS:
- [ ] <one behavior per line, with the exact upstream string where there is one>
- [ ] write tests: <Rust unit tests: the cases from the Swift tests>
- [ ] write tests: <conformance tests, if any>
RISKS: <ordering dependencies on other clusters, conflicts between docs and code, or none>
```
