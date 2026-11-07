# Release pipeline: the GitHub release and the signed APK

## Overview

- Scope (user, 2026-11-07): the result of this plan is a GitHub release that holds
  - the Android APK, signed;
  - the Linux packages (tar.gz, deb, rpm, Arch package, AppImage);
  - the Windows MSI and portable zip, unsigned;
  - the macOS `.app.zip`, `.tar.gz`, DMG and PKG, unsigned. "Unsigned" here means no Developer ID: the app stays
    ad-hoc signed with the hardened runtime and its entitlements, as the work moved to the Apple plan left it,
    because Apple Silicon refuses to run a binary with no signature at all;
  - the iOS app as an unsigned IPA;
  - `SHA256SUMS`, its minisign signature and GitHub build attestations over every asset.
- Moved out (user, 2026-11-07), each into a plan that builds on this one's job graph, signed rehearsals and updater
  trust:
  - **Apple** (`docs/plans/20261107-apple-signing.md`):
    - macOS Developer ID signing, notarization and stapling. Tasks 4 and 5 of this plan were done before the split
      and moved there with their records; their code stays in the repository and runs here with the ad-hoc identity;
    - the updater's team requirement;
    - signed iOS builds for TestFlight and the App Store listing;
    - the Homebrew cask, which needs a notarized app.
  - **Windows** (`docs/plans/20261107-windows-signing.md`): Authenticode through SignPath Foundation, the exes'
    version resources, and winget, which needs a signed MSI.
  - **Google Play** (`docs/plans/20261107-google-play.md`): the App Bundle signed with the upload key, the upload to
    the internal track through Workload Identity Federation, and the store listing. This plan builds and signs the
    APK it ships in the GitHub release; the Play plan adds the bundle beside it.
  - **AUR** (`docs/plans/20261107-aur.md`): the `ngterm-bin` package from the release's Linux tarballs, and the `aur`
    job that updates it.
  - **Flathub** (`docs/plans/20261107-flathub.md`): the Flatpak, panes spawning the host shell from the sandbox, and
    the submission.
- Signing happens inside GitHub Actions, so a release takes a tag push and one approval.
- Prepared ahead of the credentials:
  - until a secret exists, its step is skipped with a workflow notice, and the release stays green and unsigned;
  - signing becomes mandatory, so a missing secret fails the release, in two ways:
    - `NGTERM_RELEASE_SIGNING=required` covers every platform, macOS and Windows included. So the user sets it only
      once the Apple and Windows plans sign too;
    - a committed updater trust anchor covers only its own component: the minisign key forces the `SHA256SUMS`
      signature. (`trust/apple-team-id` forces Developer ID signing, and stays empty until the Apple plan.)
- Keys are reachable from as few places as possible:
  - one `release` environment holds every secret, and only `v*` tags can deploy to it;
  - a single gate job, which the user approves, guards each release;
  - building (cargo, Gradle, Xcode, every `build.rs`) runs in jobs with no secrets;
  - signing jobs run no cargo or Gradle, restore no cache, and **never execute anything the build produced**;
  - smoke tests of the signed output run in secret-free verify jobs afterwards.
- Credentials are requested as their task lands, never batched at the end (user, 2026-11-06): each task ends with a
  **Credentials** list of what the user creates and enters, one step at a time. Secrets are kept in the private
  repository `nickalie/ngterm-release-keys` (age-encrypted, mirrored to Bitwarden) and reach GitHub from there: the
  user runs `./add.sh NAME [FILE]`, `./bitwarden.sh push` and `./sync.sh`, never through the chat. Non-secret ids go
  in its `vars.tsv` (the user may just tell them). A `gh secret set ... --env release` in a Credentials list below
  means `./add.sh` for that name, then `./sync.sh`. Since 2026-11-07 the user is asked for passwords and keys through
  zenity dialogs piped straight into those scripts, which this session then runs.
- Nothing has been released yet (no tags, no GitHub releases), so there are no installed versions to stay compatible
  with.

## Context (from discovery)

- `.github/workflows/release.yml`:
  - jobs: `version`, then `linux` (2 arches) and `desktop` (Windows, macOS x86_64/arm64) build and package, then
    `publish` writes `SHA256SUMS` and the GitHub release (draft first);
  - actions are pinned by tag; caches are used in the build jobs;
  - `workflow_dispatch` has `tag` and `publish` inputs, used by `concurrency.group` and the checkout `ref`.
- `scripts/package-release.sh`:
  - packages a staged tree per OS;
  - macOS: `bundle-macos.sh`, then ad-hoc `codesign --deep`, then `ngterm --build-identity`, then `.tar.gz`,
    `.app.zip`, `.dmg`, `.pkg`;
  - Windows: `build-msi.sh` with `--archive`;
  - Linux: tar.gz, deb/rpm/archlinux, AppImage;
  - checks the font prefix (`fonts_prefix_is_complete`).
- `scripts/build-msi.sh`:
  - needs `rustc -vV` (for the arch), the font prefix and a .NET SDK (`dotnet tool install wix`);
  - `scripts/check-msi.ps1` installs the MSI and runs the installed exes.
- `crates/ngterm-update`:
  - `download.rs` checks the asset's SHA-256 against `SHA256SUMS`; asset names carry the version;
  - `validate()` checks `app-BUILD` for the archive formats.
- iOS: `scripts/build-ios.sh archive` makes a Release archive, unsigned unless the App Store Connect environment is
  set, and checks it (no fake core, the export compliance key, the privacy manifest). Its build number is the commit
  count of HEAD. `.github/workflows/ios-release.yml` uploads a tag's build to TestFlight and fails on a tag without
  its secrets.
- Android:
  - `build.gradle.kts` has `versionCode = 1`, `versionName = "0.1.0"`, targetSdk 36 and no release `signingConfig`;
  - `scripts/build-android.sh` builds `libngterm_mobile.so` with NDK 27.1 and sets no 16 KB page size;
  - `android.yml` is CI only.
- There was no `LICENSE` file (Cargo.toml says MIT, `NOTICE` refers to it); Task 1 added it.

## Development Approach

- **testing approach**: Regular (code first, then tests); every task's checks are in the same task.
- Complete each task fully before the next; small, focused changes.
- **CRITICAL: every task MUST include new/updated tests** for code changes in that task:
  - Rust: unit tests in `crates/ngterm-update`;
  - Python: a `--self-test` mode the workflow and the lint CI run;
  - shell: `shellcheck`;
  - workflows: `actionlint`, pinned and run from a checksum-verified download.
- **CRITICAL: all tests must pass before starting next task.**
- **CRITICAL: update this plan file when scope changes during implementation.**
- Checks are sized to the change (AGENTS.md > Choosing checks):
  - no CI dry runs (user's call, 2026-11-06): no `ci/release-signing-<task>` branches, no release workflow runs as a
    check. A workflow task proves itself locally (`scripts/lint.sh`, each script's `--self-test`, `shellcheck`);
    the workflow's first run is a signed rehearsal (Task 4) or a real release;
  - the workflow keeps its dry-run mode (a `publish: false` dispatch) for whoever wants one;
  - macOS and Windows behavior is never checked by `--target *-windows-*` cargo on the Linux host; the existing CI
    workflows still run on the pushed changes.

## Testing Strategy

- **Unit tests:**
  - the updater's minisign verification: valid; wrong key; tampered sums; trusted-comment mismatch; missing
    `.minisig` with a key; no key configured;
  - the `version` job's resolution of the version, Android version code, publishing and rehearsal flags, signing
    mode and platforms, including the minisign trust anchor forcing `required` and dry runs forcing `optional`.
- **Throwaway-key signing on dry runs**, so every signing branch runs before real credentials exist:
  - Android: keystores from `keytool -genkeypair`;
  - minisign: `minisign -G -W`.
- **Verify jobs** (no secrets) on the output:
  - macOS and Windows keep the smoke checks of Tasks 3-4 (the layout checks, `--build-identity`, `check-msi.ps1` with
    the upgrade test);
  - Android:
    - `apksigner verify --print-certs`, `zipalign -c -P 16 -v 4`;
    - `llvm-readelf -lW` on every `lib/*/*.so` in the APK (the core and JNA's `libjnidispatch.so`): LOAD alignment of
      at least `0x4000`;
    - `aapt2 dump badging`: the version name and code match the `version` job's outputs;
  - iOS: the IPA's `Info.plist` version and build number, and no fake core;
  - all: every artifact's digest matches what its producing job reported.
- No UI changes, so no `conformance/tests/ui` work.

## Progress Tracking

- Mark completed items with `[x]` immediately when done.
- Add newly discovered tasks with ➕ prefix; document blockers with ⚠️ prefix.
- Keep the plan in sync with the work.

## Solution Overview

- **Job graph (`release.yml`):**
  1. `version` (no secrets) runs `scripts/release-version.py` and outputs:
     - `version`, `commit`;
     - `android_version_code`;
     - `publishing`: a tag push, or a dispatch on a tag ref with `publish: true`. A `publish: true` dispatch on a
       non-tag ref fails here rather than turning into a dry run;
     - `signed_run`: publishing or a rehearsal (Task 4);
     - `signing_mode`, and per-component `require_minisign` and `require_developer_id` (see Signing mode);
     - `platforms`: the dry-run input, forced to all when publishing.
  2. `approve`: only on a signed run; environment `release-gate`, which has the user as required reviewer and holds no
     secrets. Every signing job needs `approve`, so a release is approved once, at its start.
  3. Build jobs (no environment, no secrets, caches allowed):
     - `linux`: builds, packages and runs its smoke tests as today;
     - `macos-build` (both arches): bundles the `.app` with the version and commit from `version`, ad-hoc signs it
       in the final layout, and checks the layout;
     - `windows-build`: builds the unsigned MSI and zip plus the upgrade-test MSI;
     - `android-build`: builds the unsigned APK with the `version` job's version name and code;
     - `ios-build`: archives the iOS app unsigned and packs it as an IPA (Task 5);
     - each uploads its output as a tarball, so modes and symlinks survive, except finished packages that no later
       job unpacks (`linux`, `windows-build`, `ios-build`), which travel as plain files;
     - each reports a manifest per artifact (artifact id, file digests, release assets) as job outputs, with per-arch
       output names in matrix jobs, because legs writing the same output name overwrite each other;
     - the upgrade-test MSI, the build tarballs and an unsigned APK are never release assets.
  4. Signing and packaging jobs (`environment: release` on a signed run, no cache, no cargo, no Gradle, no build
     output executed):
     - `macos-sign` (one job, both arches): packages the `.app.zip`, `.tar.gz`, DMG and PKG. Here it runs with the
       ad-hoc identity; the Apple plan gives it the Developer ID;
     - `android-sign`: signs the APK (`zipalign -P 16 -f 4`, then `apksigner`, with the APK key);
     - every signing job checks its inputs against the build job's digests before using them. It rejects symlinks
       that are absolute or point outside the extracted tree, so a build can't aim release contents at the keychain
       or key files in `RUNNER_TEMP`. It reports its own outputs' manifests as job outputs.
     - Job condition: `!cancelled() && (needs.approve.result == 'success' || (needs.version.outputs.signed_run !=
       'true' && needs.approve.result == 'skipped'))`, plus the platform gate. This stops a skipped `approve` from
       skipping every dry run, while a signed run still needs the approval.
     - Windows has no signing job here: `windows-verify` and `checksums` take `windows-build`'s files, until the
       Windows plan adds `windows-sign`.
  5. Verify jobs (no secrets): `macos-verify` (per arch), `windows-verify`, `android-verify`, as in Testing Strategy.
  6. `checksums` (read-only; `if: !cancelled() && !contains(needs.*.result, 'failure')`):
     - downloads the release assets by artifact id and checks each against the reported digests;
     - writes `SHA256SUMS` over exactly the reported release assets;
     - outputs the digest of `SHA256SUMS` and its artifact id.
  7. `publish` (signed runs only, `environment: release`; needs `approve`, `checksums` and every verify job to
     succeed):
     - downloads `SHA256SUMS` and the assets by id, checks the `SHA256SUMS` digest, and runs `sha256sum -c` over the
       assets before signing;
     - minisign-signs `SHA256SUMS`;
     - when publishing: attests every asset, uploads to the draft release and undrafts it. A rehearsal stops after
       the signature (Task 7).
  8. Channel jobs (publishing only, after `publish`): none in this plan. The AUR, Apple, Windows and Google Play plans
     add `aur`, `homebrew`, `winget` and `play-upload`; Flathub updates through its own repository.
- **Approval:** `release` has the `v*` tag deployment policy and no reviewer. A run reaching it without `approve` is
  prevented by two things:
  - the job graph: every job that uses `release` needs `approve`;
  - the tag ruleset: only the admin can create `v*` tags, so only the user can start a run that deploys to `release`.
  - `release-gate` leaves `prevent_self_review` off, since the user both pushes the tag and approves.
- **Signing mode** (`version` job):
  - `optional` (the default) or `required`;
  - a dry run is always `optional`;
  - a signed run is `required` when the repository variable `NGTERM_RELEASE_SIGNING=required`. It is a repository
    variable because `version` has no environment.
  - Trust anchors force only their own component, through separate outputs, so committing one doesn't block the
    platforms still waiting for credentials:
    - a non-empty `trust/minisign.pub` sets `require_minisign`: `publish` fails without the minisign key;
    - a non-empty `trust/apple-team-id` sets `require_developer_id` (the Apple plan fills it).
  - A missing trust file counts as empty.
  - With a platform's secrets, the job signs in either mode.
  - Missing secrets: `optional` writes a `::notice::` and ships the unsigned output; `required` fails, naming what's
    missing.
  - The exception is Android: an unsigned APK can't be installed. With no APK key it stays a workflow artifact only
    and is left out of the release.
- **Dry runs, rehearsals and publishing:**
  - A tag push publishes, and so does a dispatch with `publish: true` on a tag ref (`gh workflow run release --ref
    vX.Y.Z`). The `tag` input goes away, and `concurrency.group` and the checkout use `github.ref_name`/`github.ref`.
  - A dispatch with `rehearse: true` on a tag ref is a rehearsal: signed exactly as a release, published nowhere
    (Task 4).
  - A dispatch with `publish: false` on any ref is a dry run:
    - the version comes from that ref's `Cargo.toml`, with no tag check;
    - nothing uses an environment, and signing happens with throwaway keys only;
    - outputs stay as workflow artifacts;
    - its `platforms` input (default all) chooses which build, sign and verify jobs run, and `checksums` covers what's
      present.
- **Updater trust:**
  - Trust anchors are committed in `crates/ngterm-update/trust/`: `minisign.pub` and `apple-team-id`. Both are empty
    until the user fills them, and both are compiled in with `include_str!`. This plan fills `minisign.pub`.
  - A build trusts exactly what its source commit says, so a release can't talk an installed build out of checking.
  - With a key, the updater:
    - requires `SHA256SUMS.minisig` and verifies it;
    - requires the trusted comment to be `ngterm v<version> SHA256SUMS`. This is defense in depth on top of the
      versioned asset names and `validate()`, and it also covers the MSI and AppImage, which skip `validate()`.

## Technical Details

- **Secrets** (environment `release`):
  - Android: `ANDROID_APK_KEYSTORE` (base64), `ANDROID_APK_KEYSTORE_PASSWORD`, `ANDROID_APK_KEY_PASSWORD`, the
    GitHub APK's key. Its alias is the variable `ANDROID_APK_KEY_ALIAS`, not a secret: GitHub masks every secret
    value in the logs and drops any job output containing one, and the alias `ngterm` is in every file name;
  - updater: `MINISIGN_SECRET_KEY`, generated with `minisign -G -W` (no password: the environment already protects
    it).
  - Secrets are read only in the `env:` of the steps that use them, never at job level.
- **Variables:** `NGTERM_RELEASE_SIGNING` is repository-level; every other variable lives in `release`.
  - `NGTERM_RELEASE_SIGNING`;
  - `ANDROID_APK_KEY_ALIAS` (`ngterm`);
  - the Apple, Windows and Google Play plans add their own.
- **iOS (unsigned IPA):**
  - `ios-build` runs on the Xcode runner `ios-release.yml` uses, checks out with the full history (the build number
    is the commit count), and runs `scripts/build-ios.sh archive` with none of the App Store Connect variables, so the
    archive is unsigned and checked as in a dry run of `ios-release.yml`;
  - the IPA is `Payload/NgTerm.app` from the archive, zipped as `ngterm-<version>-ios-unsigned.ipa`. It installs only
    after the user re-signs it (a sideloading tool, or their own Xcode team); the signed app goes through TestFlight
    and the App Store (Apple plan).
- **Android:**
  - `versionName` is the workspace version and `versionCode` is `major * 1_000_000 + minor * 1_000 + patch`. Both
    come from the `version` job and reach Gradle as `-Pngterm.versionName`/`-Pngterm.versionCode`; Gradle keeps its
    defaults when they're absent.
  - `libngterm_mobile.so` is linked with `-z max-page-size=16384`. The APK is aligned with `zipalign -P 16 -f 4`.
    `android-verify` checks it with `zipalign -c -P 16 -v 4` and checks the ELF LOAD alignment of every `.so`.
  - The APK is named `ngterm-<version>-android.apk`. Its key is ngterm's own and permanent (an installed APK takes
    updates only signed with it). It is also Google Play's app signing key (user, 2026-11-07: the Google Play plan
    enrolls Play App Signing with it), so a Play install and the GitHub APK update each other without an
    uninstall.
- **Attestations:** `actions/attest-build-provenance` over every asset in `publish`. Users verify with
  `gh attestation verify <file> --repo nickalie/ngterm`.
- **Permissions:** a job-level `permissions:` replaces the workflow default, so each widened job lists everything it
  needs.
  - workflow level: `contents: read`;
  - `publish`: `contents: write`, `id-token: write`, `attestations: write`.
- **Hardening:**
  - every action in `release.yml` pinned by full commit SHA (tag in a comment). Other workflows hold no secrets and
    keep their tags; Dependabot keeps both kinds current;
  - `persist-credentials: false` on every checkout;
  - no `pull_request_target`;
  - `.github/dependabot.yml` for `github-actions`.
- **Repository protection (`scripts/setup-release-protection.sh`, run by the user once):** idempotent, through
  `gh api`, with `--dry-run` printing the bodies. It sets up:
  - environment `release-gate`: the repo owner as required reviewer, `prevent_self_review: false`, the `v*` tag
    policy;
  - environment `release`: the `v*` tag policy only;
  - a tag ruleset on `refs/tags/v*` that blocks creation, update and deletion except for the repository admin role.

## What Goes Where

- **Implementation Steps:** workflows, scripts, the updater, Android and iOS build changes, LICENSE, docs.
- **Credentials**, asked at the task that needs them: the keys repository flow above.
- **Post-Completion:** what only the user can do (an account's console), and the first release's manual checks.

## Implementation Steps

### Task 1: Groundwork: LICENSE, protection script, workflow lint

**Files:**
- Create: `LICENSE`, `scripts/setup-release-protection.sh`, `.github/dependabot.yml`
- Create: `.github/workflows/lint.yml`
- Modify: `scripts/package-release.sh`, `scripts/build-msi.sh`, `scripts/check-msi.ps1` (ship `LICENSE`)
- Modify: the tool pins file for `actionlint` and `shellcheck`
- ➕ Create: `scripts/lint.sh` (what `lint.yml` runs, and runs the same locally), `.shellcheckrc`,
  `.github/actionlint.yaml`
- ➕ Modify: the scripts and workflow that failed the new lint (`scripts/lib/native.sh`, `scripts/lib/dev-instance.sh`,
  `scripts/fetch-wpe.sh`, the four `scripts/vendor-*.sh`, `cross-platform.yml`), `NOTICE`, README > CI

- [x] `LICENSE` with the MIT text (copyright Nick Gluschenko); ship it beside `NOTICE` in `share/doc/ngterm` and in
      the MSI (as `LICENSE.txt` there, like `NOTICE.txt`)
- [x] `scripts/setup-release-protection.sh` as Technical Details describes; it reads the state back after applying
- [x] `.github/dependabot.yml` for `github-actions` (weekly)
- [x] `.github/workflows/lint.yml` (none exists today):
      - `actionlint` and `shellcheck`, pinned in `packaging/release/pins.env` and checksum-verified, through
        `scripts/lint.sh`;
      - path filters `.github/workflows/**`, `scripts/*.sh`, `scripts/*.py` (plus `scripts/lib/*.sh`, the linters'
        settings and the pins file);
      - later tasks add each `--self-test` here
- [x] `check-msi.ps1` lists `share\doc\ngterm\LICENSE.txt` among the installed files (the MSI harvests `share\**`)
- [x] tests: `shellcheck` on the touched scripts; the `--dry-run` output parses as JSON; `actionlint` passes on all
      current workflows. `LICENSE` in the packaged MSI is confirmed by `windows-installer.yml` on main; in the other
      packages by the first release (no CI dry runs), not by a local Linux package build (that needs the portable WPE
      build)
- [x] run checks; must pass before Task 2

### Task 2: The version job, modes, dry-run semantics and the gate

**Files:**
- Create: `scripts/release-version.py`
- Create: `crates/ngterm-update/trust/minisign.pub`, `crates/ngterm-update/trust/apple-team-id` (both empty)
- Modify: `.github/workflows/release.yml`, `.github/workflows/lint.yml`
- ➕ Modify: `scripts/lint.sh` (runs every `scripts/*.py --self-test`), README > Downloads and releases

- [x] `scripts/release-version.py`:
      - resolves `version`, `commit`, `android_version_code`, `publishing`, `signing_mode`, `require_minisign`,
        `require_developer_id` and `platforms` from the event, the ref, `Cargo.toml`, `NGTERM_RELEASE_SIGNING` and
        the trust files (a missing file counts as empty);
      - checks the tag only when publishing; rejects `publish: true` on a non-tag ref; forces all platforms when
        publishing;
      - `--self-test`, run in `lint.yml` and in the `version` job, covers every combination: the version code's bounds
        (minor and patch < 1000); dry runs always `optional`; each trust anchor forcing only its own component
- [x] drop the `tag` input; `concurrency.group` and checkouts use `github.ref_name`/`github.ref`; add `platforms`
- [x] `approve` job on `release-gate`, only when publishing; `publish` needs it
- [x] pin every action in `release.yml` by SHA; add `persist-credentials: false`; set per-job permissions, listing
      `contents: read` beside any widened scope
- [x] ➕ the platform gate needs a job per platform, so `desktop` is split now into `windows-build` and `macos-build`
      (same steps; Task 3 reshapes them)
- [x] tests: `--self-test`; `scripts/lint.sh`
- [x] run checks; must pass before Task 3

### Task 3: Build/sign/verify split with tarball hand-off and digests

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `scripts/package-release.sh`, `scripts/bundle-macos.sh`, `scripts/build-msi.sh`
- ➕ Create: `scripts/release-artifacts.py` (manifests, digest checks, `SHA256SUMS`, safe extraction; `--self-test`)
- ➕ Modify: `scripts/release-version.py` (a `build_commit` output), README > Downloads and releases

- [x] `bundle-macos.sh` takes the version and commit as arguments (from `version`) instead of running
      `--build-identity`; local use keeps reading them from the staged `app-BUILD`
- [x] `package-release.sh`: split macOS into a bundle step (build job) and a package step (signing job) that executes
      no build output; move `--build-identity` checks into the verify jobs
- [x] `build-msi.sh`: an `--arch` option so it needs no `rustc`, and an option to accept a tree whose font notices
      are already in place, skipping the font prefix check. `windows-build` builds the unsigned MSI, zip and upgrade
      MSI and uploads them as plain files, because SignPath reads that artifact
- [x] the other build jobs upload tarballs. Every build job outputs artifact ids, per-file digests and its release
      asset list, with per-arch output names in matrix jobs
- [x] ➕ `linux` uploads its packages as plain files, like `windows-build`: they are finished packages that no later
      job unpacks, so a tarball would only add a step. The hand-off is one manifest per artifact (id, file digests,
      release assets) rather than separate id/digest outputs
- [x] ➕ the build jobs set `NGTERM_BUILD_COMMIT` to the `version` job's `build_commit`: release binaries recorded
      no commit until now, and the bundle's `GitCommit` and the verify jobs' `--build-identity` check need it
- [x] signing, verify and `checksums` jobs download by id and check digests first; signing jobs reject absolute and
      escaping symlinks in extracted trees. `checksums` writes `SHA256SUMS` over the reported release assets only.
      `publish` (publishing only) checks the `SHA256SUMS` digest and runs `sha256sum -c` before uploading
- [x] job conditions as in Solution Overview: signing jobs accept a skipped `approve` only on dry runs; `checksums`
      skips after any failure; `publish` and `play-upload` need every verify job
      (➕ `windows-sign` arrives with the Windows plan and `play-upload` with the Google Play plan; until then
      `windows-verify` and `checksums` take `windows-build`'s files)
- [x] macOS packaging runs in one `macos-sign` job for both arches from this task on, so the Developer ID steps (the
      Apple plan) only add to it
- [x] fonts: the build jobs copy the font licences into the tree, so the signing jobs need no font prefix
- [x] `macos-verify` and `windows-verify` run the smoke checks moved out of packaging (`--build-identity`,
      `check-msi.ps1` with the upgrade test)
- [x] tests: `scripts/lint.sh`; the release asset names stay as before, and the upgrade MSI stays out of
      `SHA256SUMS`, checked by reading the job graph
- [x] run checks; must pass before Task 4

➕ Tasks 4 and 5 as first planned (macOS hardened runtime and entitlements; Developer ID signing, notarization and
stapling) were done here and moved to `docs/plans/20261107-apple-signing.md` (its Tasks 1-2) when the plan was split
(user, 2026-11-07). Their code stays: `macos-sign` packages with the ad-hoc identity until the Apple plan's
credentials exist. The tasks below are renumbered from there.

### Task 4: Signed rehearsal runs

A dry run never reaches the `release` environment, so credentials could otherwise be checked only by a real release.
A rehearsal signs exactly as a release does and publishes nothing.

**Files:**
- Modify: `scripts/release-version.py`, `.github/workflows/release.yml`, README > Downloads and releases

- [x] a `rehearse` dispatch input (boolean): on a `vX.Y.Z` tag ref with `publish: false` it makes a rehearsal;
      `rehearse` on any other ref, or with `publish: true`, fails in the `version` job
- [x] `version` outputs `signed_run` (publishing or rehearsal) besides `publishing`. `approve`, the signing jobs'
      `environment: release` and their identity choice follow `signed_run`; the release upload and every channel
      job (`aur`, `homebrew`, `winget` and `play-upload`, from the AUR, Apple, Windows and Play plans) follow
      `publishing` only. The signing mode of a rehearsal is that of a release
- [x] signing jobs accept a skipped `approve` only when `signed_run` is false
- [x] `--self-test` covers the rehearsal outputs and its refusals
- [x] ➕ a rehearsal takes the `platforms` input (publishing still builds all), so one platform's credentials can be
      rehearsed without building every platform; `version` also outputs `rehearsal`, and `tag` is set on every signed
      run
- [x] tests: `scripts/lint.sh`
- [x] run checks; must pass before Task 5

**Credentials:** run `scripts/setup-release-protection.sh` (creates `release-gate` and `release`; a job naming an
environment that doesn't exist creates it unprotected, so this comes before any `v*` tag). The first rehearsal runs
when there is a credential to check (the minisign key at Task 7, the Android keys at Task 8): the user pushes the
first release's tag (say `v0.1.0`; the `release` environment admits only `v*` tags) and rejects (or cancels) that
run's `approve`, so nothing is signed or published; then `gh workflow run release --ref v0.1.0 -f rehearse=true`, and
approves that one. A rehearsal tag pins the commit it rehearsed, so it is never published once later tasks have
landed: the release is tagged from the final commit (the patch version bumped in `Cargo.toml` past every rehearsal
tag), rehearsed, and then published with `-f publish=true`.

### Task 5: iOS: the unsigned IPA in the GitHub release

**Files:**
- Modify: `.github/workflows/release.yml`, `scripts/release-version.py` (`ios` among the platforms),
  `.github/workflows/ios-release.yml`, README > Downloads and releases, `docs/ios-release.md`

- [x] `ios-build` (no secrets, platform `ios`) as Technical Details describes: the unsigned Release archive through
      `scripts/build-ios.sh archive`, packed as `ngterm-<version>-ios-unsigned.ipa`, uploaded as a plain file with
      its manifest; the IPA is a release asset
- [x] the job checks the IPA before upload: `CFBundleShortVersionString` equals the `version` job's version, the
      build number is set, and the app carries no code signature (`codesign -dv` reports none)
- [x] `checksums` and `publish` take `ios-build`'s manifest like the other build jobs'
- [x] `ios-release.yml`: a tag push without the App Store Connect secrets skips the TestFlight upload with a notice
      instead of failing, so release tags stay green until the Apple plan sets them up (that plan turns the failure
      back on once they exist)
- [x] ➕ `ios-build` reads `ios.yml`'s cargo and vendor caches without saving them, as `ios-release.yml` does; the
      IPA's checks run in the build job itself (there is no `ios-verify`: an unsigned device build can't run on a
      runner), and `build-ios.sh archive` already refuses the fake core
- [x] the README says what the IPA is: unsigned, for re-signing; the App Store and TestFlight are the signed route
- [x] tests: `scripts/lint.sh`; `--self-test` with `ios` among the platforms
- [x] run checks; must pass before Task 6

### Task 6: Updater: minisign over SHA256SUMS

**Files:**
- Modify: `crates/ngterm-update/src/{download.rs,lib.rs,tests.rs}`, `crates/ngterm-update/Cargo.toml`, `Cargo.toml`

- [x] add `minisign-verify` (the minisign author's verifier, used by the Tauri updater)
- [x] a `Trust` value passed into `select`/`prepare`, which production builds from the `trust/` files (empty or
      whitespace means none). It holds the key here; the Apple plan adds the team id to it
- [x] with a key, require `SHA256SUMS.minisig`, verify the manifest bytes, and check the trusted comment before
      reading any checksum
- [x] ➕ `check` and `prepare` take the `Trust` (the GPUI updater builds it with `Trust::release()`); with a key,
      `select` requires a unique `SHA256SUMS.minisig` at the release's own download URL, at most 4 KiB, and `prepare`
      verifies the manifest before reading any checksum. A test checks the committed `trust/minisign.pub` parses
- [x] tests: the cases in Testing Strategy. Fixtures come from the minisign CLI and a test-only key, with the commands
      in a comment
- [x] run `cargo test -p ngterm-update` and clippy on the crate; must pass before Task 7

**Credentials:** a minisign key pair, made by the user: `minisign -G -W -p minisign.pub -s minisign.key`;
`gh secret set MINISIGN_SECRET_KEY --env release < minisign.key`; the public key is committed to
`trust/minisign.pub` in this task (from then on `publish` requires the signature). Keep an offline backup of the secret
key: an installed build trusts only this key.

➕ Done 2026-11-07: the user asked for credentials through zenity dialogs, so the key pair was generated here (no
password, `minisign -G -W`). The secret key went straight into the keys repository (`add.sh`, then a sign-and-verify
round trip against the committed public key), to the `release` environment (`sync.sh`) and to Bitwarden; the plain
file was shredded. `trust/minisign.pub` is committed with this task, so `publish` must sign from Task 7 on; no
release is cut before it.

### Task 7: Publish: SHA256SUMS signature and attestations

**Files:**
- Modify: `.github/workflows/release.yml`
- ➕ Create: `scripts/sign-sums.sh` (signs and verifies; `--throwaway` for dry runs); Modify: README > Downloads and
  releases

- [x] `publish`:
      - checks the `SHA256SUMS` digest from `checksums` and runs `sha256sum -c` over the downloaded assets first;
      - installs minisign from Ubuntu's archive;
      - writes the key to a `0600` temp file removed on exit;
      - signs `SHA256SUMS` with trusted comment `ngterm v<version> SHA256SUMS`;
      - verifies the signature against `trust/minisign.pub` when that is non-empty
- [x] `actions/attest-build-provenance` over every asset
- [x] ➕ one attestation call over a checksums file (`subject-checksums`): the lines of the checked `SHA256SUMS` plus
      `SHA256SUMS` and `SHA256SUMS.minisig` themselves. `sign-sums.sh` verifies against the committed key, so a secret
      that doesn't match it fails the release; checked locally with the real key (decrypted into the script's
      environment only), a foreign key and a throwaway one
- [x] ➕ `publish` runs on rehearsals too (its condition accepts the platforms a rehearsal skipped), up to the signature and its check: the minisign key is checked before a
      real release, and nothing is attested or uploaded
- [x] mode handling: `require_minisign` (a non-empty `minisign.pub`) or `required` mode with no key → fail; otherwise
      no key → notice and no `.minisig`
- [x] dry runs: `checksums` signs with a throwaway `minisign -G -W` key and verifies it
- [x] tests: `scripts/lint.sh`
- [x] run checks; must pass before Task 8

**Credentials:** none new. A rehearsal (Task 4) checks the minisign key.

### Task 8: Android: the signed APK in the GitHub release

**Files:**
- Modify: `mobile/android/app/build.gradle.kts`, `scripts/build-android.sh`, `.cargo/config.toml` or the cargo-ndk
  invocation, `scripts/release-version.py` (if `android` isn't wired to a job yet)
- Create: `scripts/sign-android.sh`
- Modify: `.github/workflows/release.yml`, README > Downloads and releases
- ➕ Create: `packaging/android/apk-certificate.sha256` (the APK key's certificate digest, a trust anchor)

- [x] Gradle reads `ngterm.versionName`/`ngterm.versionCode`, keeping its defaults
- [x] link `libngterm_mobile.so` with `-z max-page-size=16384`
- [x] `android-build` (platform `android`, no secrets): `scripts/build-android.sh assembleRelease` with the version
      properties; uploads the unsigned APK with its manifest
- [x] `scripts/sign-android.sh`: the keystore to a temp file removed on exit; `zipalign -P 16 -f 4` then
      `apksigner sign`; verified. It takes the key as the Play plan's upload key will (one script, a key per use)
- [x] `android-sign` (`environment: release` on a signed run, needs `approve`): with the APK key,
      `ngterm-<version>-android.apk` joins the release assets; without it the APK stays a workflow artifact (a
      notice in `optional` mode, a failure in `required` mode). Dry runs sign with a `keytool -genkeypair` throwaway
      key
- [x] `android-verify`:
      - `apksigner verify --print-certs`, and the signer's certificate digest equal to the APK key's when signed with
        it;
      - `zipalign -c -P 16 -v 4`;
      - `llvm-readelf -lW` LOAD alignment of at least `0x4000` on every `lib/*/*.so`, including JNA's
        `libjnidispatch.so`;
      - `aapt2 dump badging`: version name and code equal the `version` job's outputs
- [x] `checksums` and `publish` take `android-sign`'s manifest like the other platforms'
- [x] ➕ the key exists before the code (generated here, user 2026-11-07: a random password, no dialog), so its
      certificate digest is committed as `packaging/android/apk-certificate.sha256`: `sign-android.sh` refuses a
      secret that isn't that key, `android-verify` checks the certificate, and a signed run without the secrets fails
      even in `optional` mode once the file names the key. The check half of `sign-android.sh` (`check`) is what
      `android-verify` runs; GNU `readelf` reads the ELF headers, so the verify job needs no NDK
- [x] ➕ without the key (`optional` mode, no committed digest) `android-sign` re-uploads the unsigned APK as a
      non-asset, so `checksums` and `publish` see a manifest either way
- [x] tests: `scripts/lint.sh`; ➕ on a local `assembleRelease` build (version 0.1.0, code 1000): the unsigned APK
      checks as unsigned and aligned, all 8 native libraries (the core and JNA's) have 16 KB LOAD segments, a
      throwaway key and the real key (decrypted into the script's environment only) sign and check, the real key's
      certificate equals the committed digest, and a wrong certificate, version or unsigned expectation is refused
- [x] run checks; must pass before Task 9

**Credentials** (done 2026-11-07): the APK signing key, permanent (it can't be reset; Google Play will hold a copy
as its app signing key). As the user asked, it was generated here with no dialog: `keytool -genkeypair` (PKCS12, RSA
4096, alias `ngterm`, 100 years), with a random 40-character password for the store and key. `ANDROID_APK_KEYSTORE`,
`ANDROID_APK_KEYSTORE_PASSWORD` and `ANDROID_APK_KEY_PASSWORD` are in the keys repository, the `release` environment
and Bitwarden, and the alias is the variable `ANDROID_APK_KEY_ALIAS` (➕ moved out of the secrets after review: a
secret `ngterm` would be masked in every log line and drop every job output naming a release file); a round trip opened the stored keystore with the stored password and matched
the committed certificate. Left: a rehearsal with `platforms=android`, `android-verify` passing.

### Task 9: Verify acceptance criteria

- [ ] every Overview item has its job or step; every secret and variable is read somewhere and documented
- [ ] a release's assets: the signed APK, the Linux packages, the unsigned MSI and zip, the ad-hoc macOS packages, the
      unsigned IPA, `SHA256SUMS` and its `.minisig`
- [ ] `release-version.py --self-test` covers `required` from the variable, the minisign anchor forcing only its own
      component, rehearsals and the `ios` platform
- [ ] grep checks:
      - no job that runs cargo, Gradle or Xcode references the `release` environment;
      - no signing job executes a staged or packaged binary;
      - every job using `release` needs `approve`, and none accepts a skipped `approve` on a signed run;
      - no job references a secret at job level;
      - only `publish` widens the workflow's read-only permissions, besides the jobs the other plans add if they
        have landed (`windows-sign`, `play-upload`), each with only the permissions its plan names;
      - the release upload, and any channel or store job another plan added (`aur`, `homebrew`, `winget`,
        `play-upload`), run only when `publishing`, never on a rehearsal
- [ ] `actionlint`, `shellcheck`; `cargo test -p ngterm-update` and clippy on the crate

### Task 10: [Final] Documentation

- [ ] `docs/release-signing.md` (the Apple, Windows, Google Play and AUR plans add their sections):
      - every secret and variable, with its source and the job that reads it;
      - the APK key (`keytool`, with the warning that it can't be reset; keep an offline backup);
      - minisign (`minisign -G -W`, filling `trust/minisign.pub`);
      - running `setup-release-protection.sh`; rehearsals; switching to `NGTERM_RELEASE_SIGNING=required` once the
        Apple and Windows plans sign too;
      - verifying downloads (`gh attestation verify`, `minisign -Vm`)
- [ ] README: replace the release steps with a summary and a link to the page; say which assets are signed and which
      aren't yet (macOS without Developer ID, Windows without Authenticode, the IPA unsigned)
- [ ] README: installing the APK
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

*Items requiring manual intervention or external systems.*

**Accounts and credentials (user), as their tasks ask:**
- APK signing key (permanent; the keys repository and Bitwarden hold it).
- minisign key pair: the secret key to `release`, the public key committed to `trust/minisign.pub`.

**Repository settings (user, or with the user's go-ahead):**
- Run `scripts/setup-release-protection.sh` before the first `v*` tag (Task 4).
- Set `NGTERM_RELEASE_SIGNING=required` only once every platform signs, after the Apple and Windows plans.

**First release:**
- Android: the APK installs on a phone, and a later release's APK installs over it.
- The updater accepts the signed release and rejects one with a tampered `SHA256SUMS`.
- macOS and Windows install as unsigned apps do (Gatekeeper's and SmartScreen's warnings), until their plans land.
