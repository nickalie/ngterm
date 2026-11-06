# Release signing in GitHub Actions

## Overview

- Sign every release artifact inside GitHub Actions, so a release takes a tag push and one approval:
  - macOS: Developer ID signing, notarization and stapling;
  - Windows: Authenticode through SignPath Foundation;
  - Android: the Play upload key, plus a self-managed key for the GitHub APK;
  - the updater: a minisign signature over `SHA256SUMS`;
  - every artifact: GitHub build attestations.
- Prepared ahead of the credentials:
  - until a secret exists, its step is skipped with a workflow notice, and the release stays green and unsigned;
  - signing becomes mandatory, so a missing secret fails the release, in two ways:
    - `NGTERM_RELEASE_SIGNING=required`, which the user sets later, covers every platform;
    - a committed updater trust anchor covers only its own component: the minisign key forces the `SHA256SUMS`
      signature, and the Apple team id forces Developer ID signing.
- Keys are reachable from as few places as possible:
  - one `release` environment holds every secret, and only `v*` tags can deploy to it;
  - a single gate job, which the user approves, guards each release;
  - building (cargo, Gradle, every `build.rs`) runs in jobs with no secrets;
  - signing jobs run no cargo or Gradle, restore no cache, and **never execute anything the build produced**;
  - smoke tests of the signed output run in secret-free verify jobs afterwards.
- iOS is out of scope: `docs/plans/20261103-ios-app.md` and the iOS branch are left untouched.
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
- `scripts/bundle-macos.sh`:
  - runs the bundled `ngterm --build-identity` to fill `Info.plist`, which has no usage descriptions;
  - ad-hoc `--deep` sign, no hardened runtime, no entitlements.
- `scripts/build-msi.sh`:
  - needs `rustc -vV` (for the arch), the font prefix and a .NET SDK (`dotnet tool install wix`);
  - `scripts/check-msi.ps1` installs the MSI and runs the installed exes.
- `crates/ngterm-gpui/windows/ngterm.rc` holds only the icon. The other three exes have no resources. The workspace
  already depends on `embed-resource`.
- Upstream (`.upstream/agterm/scripts/release.sh`, `agterm/agterm.entitlements`, `agterm/agterm-debug.entitlements`,
  `agterm/Info.plist`):
  - signs helpers first with no entitlements, then the app with its TCC entitlements;
  - `--options runtime --timestamp`, never `--deep` on the Developer ID sign;
  - order: sign, zip for the notary, notarize, staple the app, then the DMG from the stapled app, then
    codesign/notarize/staple the DMG;
  - `NS*UsageDescription` strings for every TCC resource.
- `crates/ngterm-update`:
  - `download.rs` checks the asset's SHA-256 against `SHA256SUMS`; asset names carry the version;
  - `install.rs` runs `codesign --verify --deep --strict` on a `MacBundle` and nothing on a `MacArchive`;
  - `validate()` checks `app-BUILD` for the archive formats.
- Android:
  - `build.gradle.kts` has `versionCode = 1`, `versionName = "0.1.0"`, targetSdk 36 and no release `signingConfig`;
  - `scripts/build-android.sh` builds `libngterm_mobile.so` with NDK 27.1 and sets no 16 KB page size;
  - `android.yml` is CI only.
- There is no `LICENSE` file (Cargo.toml says MIT, `NOTICE` refers to it). SignPath Foundation requires one.

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
    the workflow's first run is a real release;
  - the workflow keeps its dry-run mode (a `publish: false` dispatch) for whoever wants one;
  - macOS and Windows behavior is never checked by `--target *-windows-*` cargo on the Linux host; the existing CI
    workflows still run on the pushed changes.

## Testing Strategy

- **Unit tests:**
  - the updater's minisign verification: valid; wrong key; tampered sums; trusted-comment mismatch; missing
    `.minisig` with a key; no key configured;
  - the macOS requirement string, with and without a team id, applied to `MacBundle` and `MacArchive`;
  - the `version` job's resolution of the version, Android version code, publishing flag and signing mode, including
    trust anchors forcing `required` and dry runs forcing `optional`.
- **Throwaway-key signing on dry runs**, so every signing branch runs before real credentials exist:
  - Android: keystores from `keytool -genkeypair`;
  - minisign: `minisign -G -W`;
  - macOS: a self-signed codesigning identity in the temporary keychain, which covers import, partition list,
    helper/app order and `productsign`, but not notarization;
  - SignPath: none. Its token lives only in `release`, which dry runs never reach, so Windows dry runs take the
    unsigned path.
- **Verify jobs** (no secrets) on the signed output:
  - macOS: hardened runtime on every Mach-O; helpers carry no entitlements; the app's entitlements match the file
    exactly; `--build-identity`; `spctl` when Developer ID signed;
  - Windows: `Get-AuthenticodeSignature`; `check-msi.ps1` with the upgrade test;
  - Android:
    - `apksigner verify --print-certs`, `jarsigner -verify`, `zipalign -c -P 16 -v 4`;
    - `llvm-readelf -lW` on every `lib/*/*.so` in the APK (the core and JNA's `libjnidispatch.so`): LOAD alignment of
      at least `0x4000`;
    - `aapt2 dump badging`: the version name and code match the `version` job's outputs;
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
     - `signing_mode`, and per-component `require_minisign` and `require_developer_id` (see Signing mode);
     - `platforms`: the dry-run input, forced to all when publishing.
  2. `approve`: only when publishing; environment `release-gate`, which has the user as required reviewer and holds no
     secrets. Every signing job needs `approve`, so a release is approved once, at its start.
  3. Build jobs (no environment, no secrets, caches allowed):
     - `linux`: builds, packages and runs its smoke tests as today;
     - `macos-build` (both arches): bundles the `.app` with the version and commit from `version`, ad-hoc signs it
       in the final layout, and checks the layout;
     - `windows-build`: builds the unsigned MSI and zip plus the upgrade-test MSI;
     - `android-build`: builds the unsigned AAB and APK with the `version` job's version name and code;
     - each uploads its output as a tarball, so modes and symlinks survive. The exception is `windows-build`: it
       uploads the MSI, the zip and the upgrade MSI as plain files, because SignPath opens the artifact itself and
       can't read tar;
     - each reports its artifact id and file digests as job outputs, using `upload-artifact`'s `artifact-id` and
       `artifact-digest`. Matrix legs use per-arch output names (`artifact_id_<arch>`, `digests_<arch>`), because
       legs writing the same output name overwrite each other;
     - each reports the list of its files that are release assets. The upgrade-test MSI, the build tarballs, the AAB
       and an unsigned APK are never release assets.
  4. Signing jobs (`environment: release` when publishing, no cache, no cargo, no Gradle, no build output executed):
     - `macos-sign` (one job, both arches):
       - imports the keychain and re-signs helpers then app;
       - notarizes and staples the app, then makes the `.app.zip`, `.tar.gz`, DMG and PKG;
       - signs, notarizes and staples the DMG and PKG;
     - `windows-sign`: one SignPath request signs the PE files inside the MSI and zip, and the MSI itself. Two
       requests are the fallback (Task 6);
     - `android-sign`: signs the APK (`zipalign -P 16 -f 4`, then `apksigner`, with the APK key) and the AAB
       (`jarsigner`, with the upload key);
     - every signing job checks its inputs against the build job's digests before using them. It rejects symlinks
       that are absolute or point outside the extracted tree, so a build can't aim release contents at the keychain
       or key files in `RUNNER_TEMP`. It reports its own outputs' digests, artifact ids and release-asset list as job
       outputs.
     - Job condition: `!cancelled() && (needs.approve.result == 'success' || (needs.version.outputs.publishing !=
       'true' && needs.approve.result == 'skipped'))`, plus the platform gate. This stops a skipped `approve` from
       skipping every dry run, while a publishing run still needs the approval.
  5. Verify jobs (no secrets): `macos-verify` (per arch), `windows-verify`, `android-verify`, as in Testing Strategy.
  6. `checksums` (read-only; `if: !cancelled() && !contains(needs.*.result, 'failure')`):
     - downloads the release assets by artifact id and checks each against the reported digests;
     - writes `SHA256SUMS` over exactly the reported release assets;
     - outputs the digest of `SHA256SUMS` and its artifact id.
  7. `publish` (publishing only, `environment: release`; needs `approve`, `checksums` and every verify job to
     succeed):
     - downloads `SHA256SUMS` and the assets by id, checks the `SHA256SUMS` digest, and runs `sha256sum -c` over the
       assets before signing;
     - minisign-signs `SHA256SUMS`;
     - attests every asset;
     - uploads to the draft release and undrafts it.
  8. `play-upload` (publishing only, `environment: release`; needs `approve` and `android-verify` to succeed):
     - logs in with Workload Identity Federation;
     - sends the AAB to the internal track.
- **Approval:** `release` has the `v*` tag deployment policy and no reviewer. A run reaching it without `approve` is
  prevented by two things:
  - the job graph: every job that uses `release` needs `approve`;
  - the tag ruleset: only the admin can create `v*` tags, so only the user can start a run that deploys to `release`.
  - `release-gate` leaves `prevent_self_review` off, since the user both pushes the tag and approves.
- **Signing mode** (`version` job):
  - `optional` (the default) or `required`;
  - a dry run is always `optional`;
  - a publishing run is `required` when the repository variable `NGTERM_RELEASE_SIGNING=required`. It is a
    repository variable because `version` has no environment.
  - Trust anchors force only their own component, through separate outputs, so committing one doesn't block the
    platforms still waiting for credentials:
    - a non-empty `trust/minisign.pub` sets `require_minisign`: `publish` fails without the minisign key;
    - a non-empty `trust/apple-team-id` sets `require_developer_id`: `macos-sign` fails without the Apple secrets.
  - A missing trust file counts as empty.
  - With a platform's secrets, the job signs in either mode.
  - Missing secrets: `optional` writes a `::notice::` and ships the unsigned output; `required` fails, naming what's
    missing.
  - The exception is Android: an unsigned APK can't be installed. With no APK key it stays a workflow artifact only
    and is left out of the release, and `play-upload` skips with no upload key.
- **Dry runs vs publishing:**
  - A tag push publishes, and so does a dispatch with `publish: true` on a tag ref (`gh workflow run release --ref
    vX.Y.Z`). The `tag` input goes away, and `concurrency.group` and the checkout use `github.ref_name`/`github.ref`.
  - A dispatch with `publish: false` on any ref is a dry run:
    - the version comes from that ref's `Cargo.toml`, with no tag check;
    - nothing uses an environment, and signing happens with throwaway keys only;
    - outputs stay as workflow artifacts;
    - its `platforms` input (default all) chooses which build, sign and verify jobs run, and `checksums` covers what's
      present.
- **Updater trust:**
  - Trust anchors are committed in `crates/ngterm-update/trust/`: `minisign.pub` and `apple-team-id`. Both are empty
    until the user fills them, and both are compiled in with `include_str!`.
  - A build trusts exactly what its source commit says, so a release can't talk an installed build out of checking.
  - With a key, the updater:
    - requires `SHA256SUMS.minisig` and verifies it;
    - requires the trusted comment to be `ngterm v<version> SHA256SUMS`. This is defense in depth on top of the
      versioned asset names and `validate()`, and it also covers the MSI and AppImage, which skip `validate()`.
  - With a team id, the existing `codesign --verify --deep --strict` (`--strict` alone for a single Mach-O) also
    gets `-R='=<requirement>'`, with the Developer ID requirement
    `anchor apple generic and certificate 1[field.1.2.840.113635.100.6.2.6] exists and certificate leaf[field.1.2.840.113635.100.6.1.13] exists and certificate leaf[subject.OU] = "<team id>"`:
    - for a `MacBundle`, on the bundle;
    - for a `MacArchive`, on every Mach-O in its `bin/`.
- **macOS entitlements parity:**
  - ngterm is the responsible process for what its panes run. Under the hardened runtime, a missing TCC entitlement
    means macOS never even prompts (upstream issue #396).
  - So the app takes upstream's shipping entitlement set and its `NS*UsageDescription` strings, reworded for ngterm.
  - Local `--profile debug` bundles add `com.apple.security.get-task-allow`, so a debugger can attach.
  - Helpers are signed first with no entitlements; the app is signed last, without `--deep`.
  - In the `.tar.gz`, `bin/ngterm` is the responsible process too. So it is signed with `ngterm.entitlements`, and
    only the other binaries are signed as helpers. `macos-verify` checks both layouts.

## Technical Details

- **Secrets** (environment `release`):
  - macOS:
    - `APPLE_DEVELOPER_ID_APP_P12`, `APPLE_DEVELOPER_ID_INSTALLER_P12` (base64);
    - `APPLE_P12_PASSWORD`;
    - `APPLE_NOTARY_KEY_P8` (base64 App Store Connect API key);
  - Windows: `SIGNPATH_API_TOKEN`;
  - Android:
    - `ANDROID_UPLOAD_KEYSTORE` (base64), `ANDROID_UPLOAD_KEYSTORE_PASSWORD`, `ANDROID_UPLOAD_KEY_ALIAS`,
      `ANDROID_UPLOAD_KEY_PASSWORD`;
    - the same four with `ANDROID_APK_` for the GitHub APK key;
  - updater: `MINISIGN_SECRET_KEY`, generated with `minisign -G -W` (no password: the environment already protects
    it).
  - Secrets are read only in the `env:` of the steps that use them, never at job level.
- **Variables:** `NGTERM_RELEASE_SIGNING` is repository-level; every other variable lives in `release`.
  - `NGTERM_RELEASE_SIGNING`;
  - Apple: `APPLE_TEAM_ID`, `APPLE_NOTARY_KEY_ID`, `APPLE_NOTARY_ISSUER_ID`;
  - SignPath:
    - `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`;
    - `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG`, plus a second slug if the two-request fallback is used;
  - Google: `GCP_WORKLOAD_IDENTITY_PROVIDER`, `GCP_PLAY_SERVICE_ACCOUNT`, `PLAY_RELEASE_STATUS` (default
    `completed`; `draft` while the Play app is still a draft).
- **macOS (`scripts/sign-macos.sh`)**, the same code for ad-hoc, throwaway and Developer ID identities:
  1. Keychain:
     - `security create-keychain` with a random password, `set-keychain-settings -lut 21600`, `unlock-keychain`;
     - import both `.p12` files with `-T /usr/bin/codesign -T /usr/bin/productsign`;
     - `security set-key-partition-list -S apple-tool:,apple:,codesign:,productsign: -s -k`;
     - add the keychain to the user search list.
     - An EXIT trap restores the original search list, deletes the keychain and removes the notary `.p8` from disk.
  2. Sign every Mach-O in `Contents/MacOS` except `ngterm` with
     `codesign --force --options runtime --timestamp --sign "$ID"`. Then sign the app with
     `--entitlements packaging/macos/ngterm.entitlements`. Then run `codesign --verify --deep --strict`.
  3. Submit the zipped app to `xcrun notarytool submit --key/--key-id/--issuer`. Then `stapler staple` and
     `stapler validate` on the app.
  4. From the stapled app, make the `.app.zip` and the DMG. Then `codesign --timestamp` the DMG.
  5. Make the PKG with `pkgbuild` then `productsign` (Installer identity).
  6. Make the `.tar.gz` from the loose `bin`/`share`. `bin/ngterm` is signed with the app's entitlements, the other
     Mach-Os as helpers. They are notarized as a zip and can't be stapled.
  7. Submit the DMG, PKG and loose-binaries zip in parallel (`submit` without `--wait`, then `notarytool wait` on
     each id). On rejection, fetch and print the log. Staple and validate the DMG and PKG.
  - Throwaway identity: steps 1, 2, 4 and 5, with no notarization.
    - The `.p12` is made with `/usr/bin/openssl` (LibreSSL), since `security import` rejects OpenSSL 3's default
      PKCS#12.
    - The certificate has the codeSigning extended key usage.
    - Signing uses the identity's SHA-1 hash, because an untrusted identity isn't found by name.
  - Ad-hoc identity: step 2 with `--sign -` and no `--timestamp`, done in `macos-build`.
- **Windows (SignPath):**
  - The artifact configuration is committed at `packaging/windows/signpath/` for the user to paste into SignPath.
    It describes the real nesting: the artifact zip holds the MSI and the portable zip, and each of those holds the
    exes.
  - Requests go through the official `signpath/github-action-submit-signing-request` with `wait-for-completion` and a
    timeout long enough for a manual approval.
  - `windows-sign` gets `actions: read`, so SignPath can fetch the artifact; check this against the action's docs.
  - Two-request fallback, used only if one request can't sign the files inside the MSI:
    1. `windows-sign-binaries` signs a zip of the exes;
    2. `windows-msi` (no secrets) builds the MSI and the zip from them;
    3. `windows-sign-msi` signs the MSI.
    - WiX never runs where the token is, but each release then needs two SignPath approvals.
  - Every exe carries a `VERSIONINFO` resource (product name, version, company), which SignPath Foundation requires:
    - a shared template at `packaging/windows/version.rc.in`;
    - for `ngterm.exe`, it goes into the existing `crates/ngterm-gpui/windows/ngterm.rc` (its `build.rs` compiles that
      one file);
    - the other three crates get a `build.rs` gated on `CARGO_CFG_TARGET_OS == "windows"`, as `windows_resources()`
      already is.
- **Android:**
  - `versionName` is the workspace version and `versionCode` is `major * 1_000_000 + minor * 1_000 + patch`. Both
    come from the `version` job and reach Gradle as `-Pngterm.versionName`/`-Pngterm.versionCode`; Gradle keeps its
    defaults when they're absent.
  - `libngterm_mobile.so` is linked with `-z max-page-size=16384`. The APK is aligned with `zipalign -P 16 -f 4`.
    `android-verify` checks it with `zipalign -c -P 16 -v 4` and checks the ELF LOAD alignment of every `.so`.
  - The APK is named `ngterm-<version>-android.apk`.
  - The Play upload (`scripts/play-upload.sh AAB`):
    - takes the access token from `google-github-actions/auth` (WIF, `token_format: access_token`, scope
      `androidpublisher`);
    - calls the Play Developer API v3: `edits.insert`, `edits.bundles.upload`, `edits.tracks.update` (internal track,
      `PLAY_RELEASE_STATUS`), `edits.commit`;
    - is four curl calls on purpose, so no third-party action runs in a job holding a Play token.
- **Attestations:** `actions/attest-build-provenance` over every asset in `publish`. Users verify with
  `gh attestation verify <file> --repo nickalie/ngterm`.
- **Permissions:** a job-level `permissions:` replaces the workflow default, so each widened job lists everything it
  needs.
  - workflow level: `contents: read`;
  - `publish`: `contents: write`, `id-token: write`, `attestations: write`;
  - `play-upload`: `contents: read`, `id-token: write`;
  - `windows-sign`: `contents: read`, `actions: read`.
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

- **Implementation Steps:** workflows, scripts, the updater, Android build changes, Windows resources, LICENSE, docs.
- **Post-Completion:** certificates, keys and accounts (Apple, SignPath, Google Play, GCP), running
  `setup-release-protection.sh`, entering secrets and variables, the first manual Play upload.

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

- [ ] `scripts/release-version.py`:
      - resolves `version`, `commit`, `android_version_code`, `publishing`, `signing_mode`, `require_minisign`,
        `require_developer_id` and `platforms` from the event, the ref, `Cargo.toml`, `NGTERM_RELEASE_SIGNING` and
        the trust files (a missing file counts as empty);
      - checks the tag only when publishing; rejects `publish: true` on a non-tag ref; forces all platforms when
        publishing;
      - `--self-test`, run in `lint.yml` and in the `version` job, covers every combination: the version code's bounds
        (minor and patch < 1000); dry runs always `optional`; each trust anchor forcing only its own component
- [ ] drop the `tag` input; `concurrency.group` and checkouts use `github.ref_name`/`github.ref`; add `platforms`
- [ ] `approve` job on `release-gate`, only when publishing
- [ ] pin every action in `release.yml` by SHA; add `persist-credentials: false`; set per-job permissions, listing
      `contents: read` beside any widened scope
- [ ] tests: `--self-test`; `scripts/lint.sh`
- [ ] run checks; must pass before Task 3

### Task 3: Build/sign/verify split with tarball hand-off and digests

**Files:**
- Modify: `.github/workflows/release.yml`
- Modify: `scripts/package-release.sh`, `scripts/bundle-macos.sh`, `scripts/build-msi.sh`

- [ ] `bundle-macos.sh` takes the version and commit as arguments (from `version`) instead of running
      `--build-identity`; local use keeps reading them from the staged `app-BUILD`
- [ ] `package-release.sh`: split macOS into a bundle step (build job) and a package step (signing job) that executes
      no build output; move `--build-identity` checks into the verify jobs
- [ ] `build-msi.sh`: an `--arch` option so it needs no `rustc`, and an option to accept a tree whose font notices
      are already in place, skipping the font prefix check. `windows-build` builds the unsigned MSI, zip and upgrade
      MSI and uploads them as plain files, because SignPath reads that artifact
- [ ] the other build jobs upload tarballs. Every build job outputs artifact ids, per-file digests and its release
      asset list, with per-arch output names in matrix jobs
- [ ] signing, verify and `checksums` jobs download by id and check digests first; signing jobs reject absolute and
      escaping symlinks in extracted trees. `checksums` writes `SHA256SUMS` over the reported release assets only.
      `publish` (publishing only) checks the `SHA256SUMS` digest and runs `sha256sum -c` before uploading
- [ ] job conditions as in Solution Overview: signing jobs accept a skipped `approve` only on dry runs; `checksums`
      skips after any failure; `publish` and `play-upload` need every verify job
- [ ] macOS packaging runs in one `macos-sign` job for both arches from this task on, so Task 5 only adds the
      Developer ID steps to it
- [ ] fonts: the build jobs copy the font licences into the tree, so the signing jobs need no font prefix
- [ ] `macos-verify` and `windows-verify` run the smoke checks moved out of packaging (`--build-identity`,
      `check-msi.ps1` with the upgrade test)
- [ ] tests: `scripts/lint.sh`; the release asset names stay as before, and the upgrade MSI stays out of
      `SHA256SUMS`, checked by reading the job graph
- [ ] run checks; must pass before Task 4

### Task 4: macOS hardened runtime, entitlements and layout checks (ad-hoc)

**Files:**
- Create: `packaging/macos/ngterm.entitlements`, `packaging/macos/ngterm-debug.entitlements`, `scripts/sign-macos.sh`
- Modify: `scripts/bundle-macos.sh`, `.github/workflows/release.yml`, `docs/macos.md`

- [ ] port upstream's shipping entitlements and `NS*UsageDescription` strings, reworded for ngterm; the debug file is
      the shipping set plus `get-task-allow`
- [ ] `scripts/sign-macos.sh` step 2 (helpers, then app, hardened runtime, no `--deep` on the app); `bundle-macos.sh`
      uses it ad-hoc in place of today's `--deep` sign, with the debug entitlements for `--profile debug`
- [ ] `macos-verify` layout checks: hardened runtime on every Mach-O; no entitlements on helpers; the app's
      `ngterm` and the archive's `bin/ngterm` carry exactly the shipping entitlements
- [ ] tests: `scripts/lint.sh`
- [ ] run checks; must pass before Task 5

### Task 5: macOS Developer ID signing, notarization and stapling

**Files:**
- Modify: `scripts/sign-macos.sh`, `scripts/package-release.sh`, `.github/workflows/release.yml`

- [ ] keychain steps 1 with the EXIT trap (keychain, search list, `.p8`)
- [ ] the order of steps 2-7: notarize and staple the app before the zip, DMG and PKG are made; parallel submissions
      for the DMG, PKG and loose binaries; print the notary log on rejection; `spctl` in `macos-verify` when Developer
      ID signed
- [ ] dry runs: the throwaway identity from Technical Details (LibreSSL `.p12`, codeSigning EKU, signing by SHA-1
      hash) runs steps 1, 2, 4 and 5. Check early whether `pkgutil --check-signature` exits 0 for an untrusted signer;
      if not, the test matches its output
- [ ] mode handling: secrets → Developer ID; `optional` without secrets → notice, the ad-hoc output ships;
      `required` or `require_developer_id` without secrets → fail, naming the secrets
- [ ] tests: `scripts/lint.sh`
- [ ] run checks; must pass before Task 6

### Task 6: Windows: version resources and SignPath signing

**Files:**
- Create: `packaging/windows/signpath/` (artifact configurations and a README of the SignPath settings)
- Create: `packaging/windows/version.rc.in`
- Modify: `crates/ngterm-gpui/windows/ngterm.rc`
- Create: `build.rs` for `ngterm-ctl`, `ngterm-session`, `ngterm-headless` (gated on `CARGO_CFG_TARGET_OS ==
  "windows"`)
- Modify: `.github/workflows/release.yml`

- [ ] a `VERSIONINFO` resource on all four exes through `embed-resource` (the workspace version, product name
      ngterm); the icon stays on `ngterm.exe` only
- [ ] read SignPath's GitHub integration docs. Record here whether one request can sign the PE files inside the MSI
      and zip as well as the MSI itself; prefer that, otherwise use the three-job fallback from Technical Details, so
      WiX never runs where the token is
- [ ] `windows-sign`: digest check, then the SignPath request(s) with `wait-for-completion` and a long timeout,
      `contents: read` and `actions: read`; mode handling as in Task 5. Dry runs take the unsigned path, with no token
- [ ] `windows-verify`: `Get-AuthenticodeSignature` is `Valid` on the four exes in the portable zip, on the installed
      exes, and on the MSI when signed; `NotSigned` is accepted only in `optional` mode; then `check-msi.ps1` with
      the upgrade test
- [ ] tests: the version resource is present (`(Get-Item ...).VersionInfo` in `windows-verify`, and in
      `windows-installer.yml`'s run on main); `scripts/lint.sh`
- [ ] run checks; must pass before Task 7

### Task 7: Updater: minisign over SHA256SUMS and the macOS team requirement

**Files:**
- Modify: `crates/ngterm-update/src/{download.rs,install.rs,lib.rs,tests.rs}`, `crates/ngterm-update/Cargo.toml`,
  `Cargo.toml`

- [ ] add `minisign-verify` (the minisign author's verifier, used by the Tauri updater)
- [ ] a `Trust` value (key, team id) passed into `select`/`prepare`, which production builds from the `trust/` files
      (empty or whitespace means none)
- [ ] with a key, require `SHA256SUMS.minisig`, verify the manifest bytes, and check the trusted comment before
      reading any checksum
- [ ] with a team id, add `-R='=<requirement>'` to the existing `codesign --verify --deep --strict` of a `MacBundle`,
      and run `codesign --verify --strict -R=...` on every Mach-O in a `MacArchive`'s `bin/`; the user-facing error
      stays the existing message
- [ ] tests: the cases in Testing Strategy. Fixtures come from the minisign CLI and a test-only key, with the commands
      in a comment. The requirement is checked as an argument string, plus one real `codesign` run on macOS CI against
      an ad-hoc binary: no team id → passes; a team id → rejected
- [ ] run `cargo test -p ngterm-update` and clippy on the crate; must pass before Task 8

### Task 8: Publish: SHA256SUMS signature and attestations

**Files:**
- Modify: `.github/workflows/release.yml`

- [ ] `publish`:
      - checks the `SHA256SUMS` digest from `checksums` and runs `sha256sum -c` over the downloaded assets first;
      - installs minisign from Ubuntu's archive;
      - writes the key to a `0600` temp file removed on exit;
      - signs `SHA256SUMS` with trusted comment `ngterm v<version> SHA256SUMS`;
      - verifies the signature against `trust/minisign.pub` when that is non-empty
- [ ] `actions/attest-build-provenance` over every asset
- [ ] mode handling: `require_minisign` (a non-empty `minisign.pub`) or `required` mode with no key → fail; otherwise
      no key → notice and no `.minisig`
- [ ] dry runs: `checksums` signs with a throwaway `minisign -G -W` key and verifies it
- [ ] tests: `scripts/lint.sh`
- [ ] run checks; must pass before Task 9

### Task 9: Android release builds, signing and Play upload

**Files:**
- Modify: `mobile/android/app/build.gradle.kts`, `scripts/build-android.sh`, `.cargo/config.toml` or the cargo-ndk
  invocation
- Create: `scripts/sign-android.sh`, `scripts/play-upload.sh`
- Modify: `.github/workflows/release.yml`

- [ ] Gradle reads `ngterm.versionName`/`ngterm.versionCode`, keeping its defaults
- [ ] link `libngterm_mobile.so` with `-z max-page-size=16384`
- [ ] `android-build`: `scripts/build-android.sh bundleRelease assembleRelease` with the version properties; uploads
      the unsigned AAB and APK with digests
- [ ] `scripts/sign-android.sh`: keystores to temp files removed on exit; `zipalign -P 16 -f 4` then `apksigner sign`
      for the APK; `jarsigner` for the AAB; both verified
- [ ] `android-sign`: with the APK key, `ngterm-<version>-android.apk` joins the release assets; without it the APK
      stays a workflow artifact. Dry runs sign with `keytool -genkeypair` throwaway keys
- [ ] `play-upload` (`id-token: write`): WIF login, then `scripts/play-upload.sh`; skipped with a notice in `optional`
      mode without the GCP variables or upload key, failing in `required` mode
- [ ] `android-verify`:
      - `apksigner verify --print-certs`, `jarsigner -verify`;
      - `zipalign -c -P 16 -v 4`;
      - `llvm-readelf -lW` LOAD alignment of at least `0x4000` on every `lib/*/*.so`, including JNA's
        `libjnidispatch.so`;
      - `aapt2 dump badging`: version name and code equal the `version` job's outputs
- [ ] tests: `scripts/lint.sh`; `scripts/sign-android.sh` signs and verifies with throwaway `keytool` keys
      locally
- [ ] run checks; must pass before Task 10

### Task 10: Verify acceptance criteria

- [ ] every Overview item has its job or step; every secret and variable is read somewhere and documented
- [ ] `release-version.py --self-test` covers `required` from the variable, and each trust anchor forcing only its own
      component
- [ ] grep checks:
      - no job that runs cargo or Gradle references the `release` environment;
      - no signing job executes a staged or packaged binary;
      - every job using `release` needs `approve`, and none accepts a skipped `approve` while publishing;
      - no job references a secret at job level;
      - only `publish`, `play-upload` and `windows-sign` widen the workflow's read-only permissions
- [ ] `actionlint`, `shellcheck`; `cargo test -p ngterm-update` and clippy on the crate

### Task 11: [Final] Documentation

- [ ] `docs/release-signing.md`:
      - every secret and variable, with its source and the job that reads it;
      - Apple: Developer ID Application and Installer certificates exported as `.p12`, the notary API key, the team id
        and `trust/apple-team-id`;
      - SignPath Foundation: application, project, policy, artifact configurations, the README code signing policy
        section, and a privacy note that the updater contacts api.github.com and github.com;
      - Google Play:
        - app creation, Play App Signing, the upload key (`keytool`);
        - the first manual AAB upload: where it comes from (Post-Completion), rolled out to internal testing so the
          app leaves draft (or `PLAY_RELEASE_STATUS=draft` until then);
        - the GCP WIF pool and provider: the attribute condition checks `assertion.repository_id`,
          `assertion.ref.startsWith('refs/tags/v')` and `assertion.environment == 'release'`, since a job with an
          environment gets an environment `sub`;
        - the service account's Play Console access;
      - the APK key (`keytool`, with the warning that it can't be reset; keep an offline backup);
      - minisign (`minisign -G -W`, filling `trust/minisign.pub`);
      - running `setup-release-protection.sh`; switching to `NGTERM_RELEASE_SIGNING=required`;
      - verifying downloads (`gh attestation verify`, `minisign -Vm`)
- [ ] README: replace the "ad-hoc signed / unsigned" paragraph and the release steps with a summary and a link to the
      page; add the SignPath code signing policy section
- [ ] `docs/macos.md`: the signing notes
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

*Items requiring manual intervention or external systems.*

**Accounts and credentials (user):**
- Apple Developer Program:
  - Developer ID Application and Installer certificates, exported as `.p12`;
  - an App Store Connect API key for notarization;
  - the team id, committed to `trust/apple-team-id` once Developer ID signing works.
- SignPath Foundation:
  - apply for the project and set up the project, signing policy and artifact configurations from
    `packaging/windows/signpath/`;
  - create the API token.
- Google Play Console:
  - create the app, enroll in Play App Signing, generate the upload key;
  - make the first AAB upload manually and roll it out to internal testing. The API can't make an app's first
    upload, and dry-run AABs carry throwaway keys. So the AAB comes from the first publishing run with the upload
    key set and the GCP variables still unset: `play-upload` skips, and `android-sign` keeps the signed AAB as a
    workflow artifact. Its version code is then used up, so the next release bumps the patch version;
  - a GCP project with the WIF pool and provider, and a service account with release access.
- APK signing key (permanent; offline backup).
- minisign key pair: the secret key to `release`, the public key committed to `trust/minisign.pub`.

**Repository settings (user, or with the user's go-ahead):**
- Run `scripts/setup-release-protection.sh`.
- Enter the secrets and variables from `docs/release-signing.md`.
- Set `NGTERM_RELEASE_SIGNING=required` once every platform signs.

**First signed release:**
- On a real Mac: the DMG and PKG open with no Gatekeeper prompt, and a tool in a pane gets a TCC prompt (microphone
  or camera).
- On Windows: SmartScreen shows the publisher.
- Android: the APK installs, and the AAB reaches the internal track.
- The updater accepts the signed release and rejects one with a tampered `SHA256SUMS`.
