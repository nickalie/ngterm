# Google Play: the signed App Bundle, the internal track and the store listing

## Overview

- Split from `docs/plans/20261105-release-signing.md` (user, 2026-11-07). That plan ships the Android APK, signed
  with ngterm's own APK key, in the GitHub release. This plan takes the app to Google Play:
  - the App Bundle (AAB), built beside the APK and signed with the Play upload key; Play App Signing re-signs what
    users install with ngterm's own APK key, the one the GitHub APK carries;
  - a `play-upload` job that sends each release's bundle to the internal track through the Play Developer API, logged
    in with Workload Identity Federation (no long-lived Google key in GitHub);
  - the store listing: text, graphics, and the written answers for the forms the API can't fill.
- Built on the release plan's job graph, its signed rehearsal runs (its Task 4), and its Android build and
  `scripts/sign-android.sh` (its Task 8), which must land first.
- Credentials are requested as their task lands, one step at a time, with passwords and keys asked through zenity
  dialogs piped straight into the private keys repository's scripts (`./add.sh`, `./bitwarden.sh push`,
  `./sync.sh`); never through the chat.
- Long lead: the Play Console developer account (identity verification). A personal account created since late 2023
  must also run a closed test with at least 12 testers for 14 days before it can apply for production access; an
  organization account (D-U-N-S number) is exempt. Start the account at Task 1.
- One app signing key for both channels (user, 2026-11-07): Play App Signing is enrolled with ngterm's APK key
  (the release plan's Task 8), not a key Google generates, so a Play install and the GitHub APK update each other with
  no uninstall. Google then holds a copy of that permanent key. The choice is made once, at enrollment, before the
  first upload, and can't be changed later; Play's key upgrade would split the channels again. The upload key stays
  separate: it only proves an upload to Play, and Google can reset it.

## Context (from discovery)

- `mobile/android/app/build.gradle.kts`: the application id `com.nickalie.ngterm.android`; the release plan's Task 8
  adds the version properties and the APK build.
- `scripts/build-android.sh` runs Gradle over the Rust core (`libngterm_mobile.so`); `android.yml` is CI only.
- The keys repository already lists `ANDROID_UPLOAD_KEYSTORE`, `ANDROID_UPLOAD_KEYSTORE_PASSWORD`,
  `ANDROID_UPLOAD_KEY_PASSWORD` and the variables `ANDROID_UPLOAD_KEY_ALIAS`, `GCP_WORKLOAD_IDENTITY_PROVIDER`,
  `GCP_PLAY_SERVICE_ACCOUNT`, `PLAY_RELEASE_STATUS`.

## Development Approach

- As the release plan's: regular testing approach, tests in the same task, all checks green before the next task,
  this file kept in sync with the work.
- Checks are sized to the change (AGENTS.md > Choosing checks): no CI dry runs; a workflow task proves itself with
  `scripts/lint.sh` and the scripts' self-tests; the signed path by a rehearsal once the credentials exist.

## Testing Strategy

- `android-verify` (no secrets): `jarsigner -verify` on the AAB, and its signer equal to the upload key's
  certificate when signed with it; the AAB's version code equal to the `version` job's.
- `scripts/sign-android.sh` signs and verifies an AAB with a throwaway `keytool` key locally.
- `scripts/play-upload.sh` and `scripts/play-listing.sh`: a `--dry-run` that prints the API calls without a token,
  checked locally; the listing files against Play's limits.
- A rehearsal signs the AAB with the real upload key; it never uploads.

## Progress Tracking

- Mark completed items with `[x]` immediately when done.
- Add newly discovered tasks with ➕ prefix; document blockers with ⚠️ prefix.
- Keep the plan in sync with the work.

## Solution Overview

- `android-build` also runs `bundleRelease`; the unsigned AAB travels with the APK and is never a release asset.
- `android-sign` also signs the AAB (`jarsigner`, with the upload key). Without the upload key the AAB stays unsigned
  and `play-upload` skips with a notice (`optional` mode) or fails (`required` mode).
- `play-upload` (publishing only, `environment: release`, needs `approve` and `android-verify` to succeed;
  `contents: read`, `id-token: write`):
  - logs in with Workload Identity Federation;
  - sends the AAB to the internal track, with the listing (Task 4) in the same edit.
- Rehearsals sign the bundle but never upload; `play-upload` follows `publishing` only.

## Technical Details

- **Secrets** (environment `release`): `ANDROID_UPLOAD_KEYSTORE` (base64), `ANDROID_UPLOAD_KEYSTORE_PASSWORD`,
  `ANDROID_UPLOAD_KEY_PASSWORD`. The alias is the variable `ANDROID_UPLOAD_KEY_ALIAS`: a plain word as a secret would
  be masked in every log line and drop every job output containing it.
- **Variables** (environment `release`): `GCP_WORKLOAD_IDENTITY_PROVIDER`, `GCP_PLAY_SERVICE_ACCOUNT`,
  `PLAY_RELEASE_STATUS` (default `completed`; `draft` while the Play app is still a draft).
- **The Play upload (`scripts/play-upload.sh AAB`):**
  - takes the access token from `google-github-actions/auth` (WIF, `token_format: access_token`, scope
    `androidpublisher`);
  - calls the Play Developer API v3: `edits.insert`, `edits.bundles.upload`, `edits.tracks.update` (internal track,
    `PLAY_RELEASE_STATUS`), `edits.commit`;
  - is four curl calls on purpose, so no third-party action runs in a job holding a Play token.
- **The WIF provider's attribute condition** checks `assertion.repository_id`,
  `assertion.ref.startsWith('refs/tags/v')` and `assertion.environment == 'release'`, since a job with an environment
  gets an environment `sub`.
- **The first upload:** the API can't make an app's first upload, and dry-run AABs carry throwaway keys. So the first
  AAB comes from a publishing run with the upload key set and the GCP variables still unset: `play-upload` skips, and
  `android-sign` keeps the signed AAB as a workflow artifact for the user to upload by hand and roll out to internal
  testing. Its version code is then used up, so the next release bumps the patch version.

## Implementation Steps

### Task 1: The App Bundle, signed with the upload key

**Files:**
- Modify: `scripts/build-android.sh` (if needed), `scripts/sign-android.sh`, `.github/workflows/release.yml`

- [ ] `android-build`: `bundleRelease` beside `assembleRelease`; the unsigned AAB in the artifact, never a release
      asset
- [ ] `sign-android.sh`: `jarsigner` for an AAB with the upload key (temp keystore removed on exit), verified
- [ ] `android-sign`: signs the AAB with the upload key; without it, a notice or a failure by mode. Dry runs sign it
      with a throwaway key
- [ ] `android-verify`: the AAB checks of Testing Strategy
- [ ] tests: `scripts/lint.sh`; `sign-android.sh` on an AAB with a throwaway key locally
- [ ] run checks; must pass before Task 2

**Credentials** (one step at a time): start the Play Console developer account now (long lead). Then the upload key,
generated here with `keytool` and its passwords asked through zenity, stored with `add.sh` under the
`ANDROID_UPLOAD_` names, mirrored to Bitwarden and synced; a rehearsal with `platforms=android`, `android-verify`
passing.

### Task 2: The Play upload job

**Files:**
- Create: `scripts/play-upload.sh` (with `--dry-run`)
- Modify: `.github/workflows/release.yml`

- [ ] `play-upload` as Solution Overview describes; `google-github-actions/auth` pinned by SHA
- [ ] skipped with a notice in `optional` mode without the GCP variables or the upload key, failing in `required` mode
- [ ] the job never runs on a rehearsal or a dry run
- [ ] tests: `scripts/lint.sh`; `play-upload.sh --dry-run` locally
- [ ] run checks; must pass before Task 3

### Task 3: Play Console, GCP and the first upload

- [ ] the app `com.nickalie.ngterm.android` created in the Play Console; Play App Signing enrolled with ngterm's APK
      key ("use a different key": the keystore exported with Google's `pepk` tool, encrypted to Google's public
      key, the keystore passwords asked through zenity), and the upload key's certificate registered as the upload
      key
- [ ] the app signing certificate Play shows equals the GitHub APK's (`apksigner verify --print-certs`)
- [ ] GCP: a project, the WIF pool and provider with the attribute condition above, a service account with release
      access to the app in the Play Console; the variables in the keys repository's `vars.tsv`, synced
- [ ] the first AAB uploaded by hand from a publishing run (Technical Details), rolled out to internal testing
- [ ] the next release's `play-upload` reaches the internal track
- [ ] ⚠️ a personal developer account: the closed test (12 testers, 14 days) before production access

**Credentials** (one step at a time): the Play Console account and app; the GCP project, pool, provider and service
account (the console steps, or `gcloud` commands for the user to run); the variable values.

### Task 4: Google Play store listing

**Files:**
- Create: `mobile/android/play/` (listing text per locale, screenshots, feature graphic, the 512px icon, release
  notes), `scripts/play-listing.sh`
- Modify: `scripts/play-upload.sh`, `.github/workflows/release.yml`, `mobile/android/DESIGN.md` if a screenshot needs
  a demo state

- [ ] listing text: title, short and full description (en-US first), from the README and DESIGN.md, within Play's
      limits (30/80/4000 characters)
- [ ] graphics: phone and 7"/10" tablet screenshots made from the emulator by a scripted demo session (no real host
      data), the 1024x500 feature graphic and the 512px icon from the app's icon source; sizes checked by the script
- [ ] `play-listing.sh` uploads text and images through the Play Developer API (`edits.listings`, `edits.images`) in
      the same edit as the AAB; `play-upload` runs it; release notes from the tag's notes
- [ ] the forms the API can't fill are written out for the user in `docs/release-signing.md` with the answers: Data
      safety (what the app collects and sends: nothing to us; ssh to the user's own hosts), content rating
      questionnaire, target audience, app access (no login), ads (none)
- [ ] a privacy policy page (the site's or the repo's) linked from the listing; the Apple plan's App Store listing
      links the same page
- [ ] tests: `scripts/lint.sh`; the listing files pass the script's limits locally
- [ ] run checks; must pass before Task 5

**Credentials:** none new. The user fills the Data safety, content rating and target audience forms in the Play
Console with the written answers, and promotes the internal release to production when ready.

### Task 5: [Final] Verify and document

- [ ] every Overview item has its job or step; every Play secret and variable is read somewhere and documented
- [ ] grep checks: `play-upload` runs only when `publishing`, never on a rehearsal; it and `publish` are the only
      jobs that widen the workflow's read-only permissions; no job that runs Gradle references `release`
- [ ] `docs/release-signing.md`, a Google Play section: app creation, Play App Signing, the upload key, the first
      manual upload, the WIF pool, provider and attribute condition, the service account's access
- [ ] README: the Google Play link
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

*Items requiring manual intervention or external systems.*

- Play Console: the forms of Task 4, the closed test if the account is personal, then promoting a release to
  production.
- From the first release on Play: the internal-track build installs from Play, and a later one updates it; a phone
  with the GitHub APK installed takes the Play update with its data intact.
