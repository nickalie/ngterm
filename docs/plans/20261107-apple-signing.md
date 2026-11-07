# Apple signing: macOS Developer ID, signed iOS builds and the Homebrew cask

## Overview

- Split from `docs/plans/completed/20261105-release-signing.md` (user, 2026-11-07), which delivers the GitHub
  release with the macOS packages ad-hoc signed and the iOS app as an unsigned IPA. This plan signs them for
  distribution:
  - macOS: Developer ID signing, notarization and stapling of the `.app.zip`, `.tar.gz`, DMG and PKG in the GitHub
    release, so they open with no Gatekeeper prompt;
  - the updater: an installed build accepts only a macOS update signed by ngterm's team;
  - iOS: signed builds uploaded to TestFlight (`ios-release.yml` already does this once its secrets exist), and the
    App Store listing and review readiness. The iOS app's code stays as `docs/plans/20261103-ios-app.md` left it. The
    desktop app stays out of the Mac App Store, whose sandbox doesn't fit a terminal;
  - Homebrew: a cask in the user's tap `nickalie/homebrew-apps`, which needs the notarized app.
- Built on the release plan's job graph, signed rehearsal runs (its Task 4) and updater `Trust` value (its Task 6).
  Tasks 1-2 here were done before the split, as that plan's Tasks 4-5.
- Credentials are requested as their task lands, one step at a time, through the private keys repository
  `nickalie/ngterm-release-keys` (`./add.sh`, `./bitwarden.sh push`, `./sync.sh`), never through the chat; the
  release plan's Overview has the details.
- ⚠️ Apple Developer Program enrollment fails for the user's Apple ID (2026-11-07): the web enrollment ends at "We are
  unable to process your request. An unknown error occurred." in Brave and Safari, and the Apple Developer app offers
  no enrollment. The Apple ID belongs to other organizations' teams. Task 3 waits on it.
- `NGTERM_RELEASE_SIGNING=required` covers every platform, so it is set only once this plan and the Windows plan sign;
  until then `trust/apple-team-id` forces Developer ID signing on its own.

## Context (from discovery)

- `scripts/bundle-macos.sh` (before Task 1):
  - ran the bundled `ngterm --build-identity` to fill `Info.plist`, which had no usage descriptions;
  - ad-hoc `--deep` sign, no hardened runtime, no entitlements.
- Upstream (`.upstream/agterm/scripts/release.sh`, `agterm/agterm.entitlements`, `agterm/agterm-debug.entitlements`,
  `agterm/Info.plist`):
  - signs helpers first with no entitlements, then the app with its TCC entitlements;
  - `--options runtime --timestamp`, never `--deep` on the Developer ID sign;
  - order: sign, zip for the notary, notarize, staple the app, then the DMG from the stapled app, then
    codesign/notarize/staple the DMG;
  - `NS*UsageDescription` strings for every TCC resource.
- `crates/ngterm-update/src/install.rs` runs `codesign --verify --deep --strict` on a `MacBundle` and nothing on a
  `MacArchive`.
- `scripts/release-version.py` already outputs `require_developer_id` from `crates/ngterm-update/trust/apple-team-id`
  (empty).
- `.github/workflows/ios-release.yml` archives, exports and uploads to TestFlight with the `ios-release` environment's
  secrets (`docs/ios-release.md` > Secrets); the release plan's Task 5 makes a tag without them skip the upload with a
  notice.

## Development Approach

- As the release plan's: regular testing approach, tests in the same task, all checks green before the next task,
  this file kept in sync with the work.
- Checks are sized to the change (AGENTS.md > Choosing checks): no CI dry runs; a workflow task proves itself with
  `scripts/lint.sh` and the scripts' self-tests; macOS behavior is checked on the real Mac
  (`nick@192.168.50.38`), and the signed path by a rehearsal once the credentials exist.

## Testing Strategy

- **Unit tests:** the macOS requirement string, with and without a team id, applied to `MacBundle` and `MacArchive`.
- **Throwaway identity on dry runs:** a self-signed codesigning identity in the temporary keychain, which covers
  import, partition list, helper/app order and `productsign`, but not notarization.
- **`macos-verify`** (no secrets): hardened runtime on every Mach-O; helpers carry no entitlements; the app's
  entitlements match the file exactly; `--build-identity`; `spctl` when Developer ID signed.
- **Rehearsals** with the real credentials: `macos-verify` passing with `developer-id`.

## Progress Tracking

- Mark completed items with `[x]` immediately when done.
- Add newly discovered tasks with ➕ prefix; document blockers with ⚠️ prefix.
- Keep the plan in sync with the work.

## Solution Overview

- **`macos-sign`** (one job, both arches; `environment: release` on a signed run, no cache, no cargo, no build output
  executed):
  - imports the keychain and re-signs helpers then app;
  - notarizes and staples the app, then makes the `.app.zip`, `.tar.gz`, DMG and PKG;
  - signs, notarizes and staples the DMG and PKG.
- **Signing mode:** a non-empty `trust/apple-team-id` sets `require_developer_id`, so `macos-sign` fails without the
  Apple secrets. With the secrets it signs in either mode; without them, `optional` writes a notice and ships the
  ad-hoc output, `required` fails naming them.
- **Updater trust:** with a team id, the existing `codesign --verify --deep --strict` (`--strict` alone for a single
  Mach-O) also gets `-R='=<requirement>'`, with the Developer ID requirement
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
- **iOS:** `ios-release.yml` stays the signed route: a release tag's build goes to TestFlight, and the user submits it
  for review from App Store Connect. The GitHub release keeps the unsigned IPA.
- **Homebrew:** a `homebrew` job (publishing only) pushes the cask after `publish`, and only when macOS shipped
  Developer ID signed.

## Technical Details

- **Secrets** (environment `release`):
  - `APPLE_DEVELOPER_ID_APP_P12`, `APPLE_DEVELOPER_ID_INSTALLER_P12` (base64);
  - `APPLE_P12_PASSWORD`;
  - `APPLE_NOTARY_KEY_P8` (base64 App Store Connect API key);
  - `HOMEBREW_TAP_TOKEN`.
- **Secrets** (environment `ios-release`, `docs/ios-release.md` > Secrets): `APP_STORE_CONNECT_KEY_ID`,
  `APP_STORE_CONNECT_ISSUER_ID`, `APP_STORE_CONNECT_KEY`, `IOS_DEVELOPMENT_TEAM`, and optionally
  `IOS_DISTRIBUTION_CERTIFICATE`, `IOS_DISTRIBUTION_CERTIFICATE_PASSWORD`.
- **Variables** (environment `release`): `APPLE_TEAM_ID`, `APPLE_NOTARY_KEY_ID`, `APPLE_NOTARY_ISSUER_ID`.
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

## Implementation Steps

### Task 1: macOS hardened runtime, entitlements and layout checks (ad-hoc)

Done as Task 4 of the release plan, before the split.

**Files:**
- Create: `packaging/macos/ngterm.entitlements`, `packaging/macos/ngterm-debug.entitlements`, `scripts/sign-macos.sh`
- Modify: `scripts/bundle-macos.sh`, `.github/workflows/release.yml`, `docs/macos.md`
- ➕ Modify: `scripts/package-release.sh` (signs the app and the archive's loose binaries through `sign-macos.sh`)

- [x] port upstream's shipping entitlements and `NS*UsageDescription` strings, reworded for ngterm; the debug file is
      the shipping set plus `get-task-allow`
- [x] `scripts/sign-macos.sh` step 2 (helpers, then app, hardened runtime, no `--deep` on the app); `bundle-macos.sh`
      uses it ad-hoc in place of today's `--deep` sign, with the debug entitlements for `--profile debug`
- [x] ➕ helpers are every Mach-O in the bundle but `ngterm`, not only `Contents/MacOS`'s, so the check's "every
      Mach-O" and the signing cover the same files; `bundle` refuses a non-Mach-O in `Contents/MacOS`, which
      `--verify --deep --strict` rejects unsigned (a script's signature would live in xattrs the archives drop)
- [x] ➕ step 6's loose-binary signing (helpers, then `bin/ngterm` with the entitlements) lands here, ad-hoc, since
      the archive layout check needs it; `sign-macos.sh check` holds the layout checks
- [x] `macos-verify` layout checks: hardened runtime on every Mach-O; no entitlements on helpers; the app's
      `ngterm` and the archive's `bin/ngterm` carry exactly the shipping entitlements
- [x] tests: `scripts/lint.sh`; ➕ on the real Mac (macOS 27.2) with stand-in binaries: `bundle-macos.sh --staged`,
      `sign-macos.sh loose` and `check` pass, the signed binary runs, the layout survives the build tarball and
      `.app.zip` round trips, and `check` refuses a helper without the runtime, a helper with entitlements and the
      debug set on a release
- [x] run checks; must pass before Task 2

### Task 2: macOS Developer ID signing, notarization and stapling

Done as Task 5 of the release plan, before the split.

**Files:**
- Modify: `scripts/sign-macos.sh`, `scripts/package-release.sh`, `.github/workflows/release.yml`
- ➕ Modify: `docs/macos.md`

- [x] keychain steps 1 with the EXIT trap (keychain, search list, `.p8`)
- [x] the order of steps 2-7: notarize and staple the app before the zip, DMG and PKG are made; parallel submissions
      for the DMG, PKG and loose binaries; print the notary log on rejection; `spctl` in `macos-verify` when Developer
      ID signed
- [x] dry runs: the throwaway identity from Technical Details (LibreSSL `.p12`, codeSigning EKU, signing by SHA-1
      hash) runs steps 1, 2, 4 and 5. Check early whether `pkgutil --check-signature` exits 0 for an untrusted signer;
      if not, the test matches its output
- [x] ➕ found on the real Mac (macOS 27.2): `productsign` takes no untrusted installer identity ("An installer
      signing identity ... is required"), whatever its EKU or Apple's installer extension. So the throwaway Installer
      certificate is trusted in the runner's admin domain with `sudo` (GitHub-hosted runners only) until the EXIT
      trap withdraws it. That trust step and `productsign` itself are unverified until the first dry run; `codesign`
      by hash, the keychain, the DMG signature and the trap's restore were run on the Mac. `pkgutil
      --check-signature` on an untrusted signer couldn't be produced there, so `macos-verify` matches the signer's name
- [x] ➕ the macOS packaging moved from `package-release.sh --macos-package` into `sign-macos.sh package`, which runs
      steps 1-7 in one process (both arches, so the notary submissions overlap); local macOS packaging calls it with
      the ad-hoc identity. `macos-sign`'s timeout is 240 minutes for two rounds of 90-minute notary waits
- [x] ➕ a Developer ID run fails when the signed app's team isn't `APPLE_TEAM_ID`, or when a non-empty
      `trust/apple-team-id` names another team
- [x] mode handling: secrets → Developer ID; `optional` without secrets → notice, the ad-hoc output ships;
      `required` or `require_developer_id` without secrets → fail, naming the secrets
- [x] tests: `scripts/lint.sh`; ➕ on the real Mac with stand-in binaries: `package --identity ad-hoc` for both arches
      (the unpacked zip and archive pass `check` and run), `throwaway` up to `productsign` with `sudo` stubbed (the
      search list restored after), `developer-id` without secrets naming them, `throwaway` refused off GitHub runners
- [x] run checks; must pass before Task 3

### Task 3: Developer ID credentials and a signed rehearsal

Needs the release plan's rehearsal runs (its Task 4).

**Files:**
- Modify: `crates/ngterm-update/trust/apple-team-id`, `docs/macos.md` if the rehearsal finds anything

- [ ] ⚠️ Apple Developer Program membership (Developer ID needs the paid program); blocked on the enrollment error in
      the Overview
- [ ] the credentials below stored in the keys repository, mirrored to Bitwarden and synced to `release`
- [ ] a rehearsal: `macos-sign` signs with `developer-id`, the notary accepts every submission, and `macos-verify`
      passes, `spctl` included. Fix what it finds (the `sudo` trust step and `productsign` were never run on CI)
- [ ] commit `trust/apple-team-id` with the team id once the rehearsal passes
- [ ] run checks; must pass before Task 4

**Credentials** (one step at a time):
- Apple Developer Program membership;
- Developer ID Application and Developer ID Installer certificates (Certificates, Identifiers & Profiles), exported
  from Keychain Access together with their keys as two `.p12` files sharing one password: `./add.sh
  APPLE_DEVELOPER_ID_APP_P12 app.p12`, the same for `APPLE_DEVELOPER_ID_INSTALLER_P12`, and `./add.sh
  APPLE_P12_PASSWORD`;
- an App Store Connect team API key (Users and Access > Integrations, the Developer role is enough for notarytool):
  `./add.sh APPLE_NOTARY_KEY_P8 AuthKey_XXXX.p8`;
- not secret, in `vars.tsv`: `APPLE_TEAM_ID`, `APPLE_NOTARY_KEY_ID`, `APPLE_NOTARY_ISSUER_ID`;
- then `./bitwarden.sh push` and `./sync.sh`.

### Task 4: Updater: the macOS team requirement

**Files:**
- Modify: `crates/ngterm-update/src/{install.rs,lib.rs,tests.rs}`

- [ ] the release plan's `Trust` value gains the team id, read from `trust/apple-team-id` (empty or whitespace means
      none)
- [ ] with a team id, add `-R='=<requirement>'` to the existing `codesign --verify --deep --strict` of a `MacBundle`,
      and run `codesign --verify --strict -R=...` on every Mach-O in a `MacArchive`'s `bin/`; the user-facing error
      stays the existing message
- [ ] tests: the requirement is checked as an argument string, plus one real `codesign` run on macOS CI against an
      ad-hoc binary: no team id → passes; a team id → rejected
- [ ] run `cargo test -p ngterm-update` and clippy on the crate; must pass before Task 5

### Task 5: iOS: signed TestFlight builds from a release tag

**Files:**
- Modify: `.github/workflows/ios-release.yml`, `docs/ios-release.md`

- [ ] the `ios-release` environment created as `docs/ios-release.md` describes, and its secrets stored in the keys
      repository and synced
- [ ] `ios-release.yml`: once the secrets exist, a release tag without them fails again (the release plan's Task 5
      made it skip with a notice)
- [ ] a release tag's build reaches TestFlight; a tester installs it
- [ ] tests: `scripts/lint.sh`
- [ ] run checks; must pass before Task 6

**Credentials** (one step at a time): the App Store Connect API key with the Admin role (`APP_STORE_CONNECT_KEY`,
`APP_STORE_CONNECT_KEY_ID`, `APP_STORE_CONNECT_ISSUER_ID`), `IOS_DEVELOPMENT_TEAM`, the bundle id
`com.nickalie.ngterm.ios` registered and its app record created in App Store Connect.

### Task 6: iOS App Store readiness

**Files:**
- Create: `mobile/ios/appstore/` (metadata per locale, screenshots, review notes), `scripts/appstore-listing.sh`
- Modify: `.github/workflows/ios-release.yml`, `docs/ios-release.md`

- [ ] metadata: name, subtitle, description, keywords, support and marketing URLs, the privacy policy URL (the Google
      Play plan's Task 4 makes the page; whichever lands first makes it), the copyright line, review notes (how to
      reach a test host: the demo mode or a reviewer-reachable sshd)
- [ ] screenshots for the required display sizes (6.9" iPhone, 13" iPad) from the simulator with a scripted demo
      session, on the real Mac's Xcode
- [ ] `appstore-listing.sh` uploads metadata and screenshots through the App Store Connect API to the version being
      prepared; `ios-release.yml` runs it after the TestFlight upload on a release tag
- [ ] the App Privacy answers and the age rating written out for the user in `docs/ios-release.md`
- [ ] tests: `scripts/lint.sh`; the metadata against App Store limits locally
- [ ] run checks; must pass before Task 7

**Credentials:** none new. The user fills App Privacy and the age rating with the written answers and presses Submit
for Review.

### Task 7: Homebrew cask in nickalie/homebrew-apps

**Files:**
- Create: `packaging/homebrew/ngterm.rb.in`, `scripts/homebrew-cask.py` (with `--self-test`)
- Modify: `.github/workflows/release.yml`, README > Downloads and releases

- [ ] the cask: `app "ngterm.app"` from each arch's notarized `.app.zip` (`on_arm`/`on_intel`), `binary` for
      `ngtermctl` inside the app, `depends_on macos: ">= :sonoma"`, `livecheck` on GitHub releases, a `zap` of the
      state and config dirs, and `auto_updates true`: the `.app.zip` keeps its `macos-bundle` update marker, so the
      app updates itself and `brew upgrade` must not fight it; Linux: decide with Homebrew's Linux cask support at
      the time, else macOS only
- [ ] `homebrew-cask.py` renders it from the version and the digests in `SHA256SUMS`; its self-test pins the output
- [ ] job `homebrew` (publishing only, `environment: release`, after `publish`): renders the cask, checks it with
      `brew audit --cask --strict` and `brew style` on a macOS runner, and pushes `Casks/ngterm.rb` to the tap's
      `main`. It runs only when macOS shipped Developer ID signed (a notice otherwise), so the cask never installs an
      app Gatekeeper blocks
- [ ] tests: `scripts/lint.sh`; `brew install --cask` from a local tap of the rendered cask on the real Mac with a
      rehearsal's packages
- [ ] run checks; must pass before Task 8

**Credentials:** a fine-grained token with Contents read/write on `nickalie/homebrew-apps` only: `./add.sh
HOMEBREW_TAP_TOKEN`, then `./sync.sh`.

### Task 8: Verify acceptance criteria

- [ ] every Overview item has its job or step; every Apple secret and variable is read somewhere and documented
- [ ] `release-version.py --self-test` covers `trust/apple-team-id` forcing only Developer ID
- [ ] grep checks: `macos-sign` executes no staged or packaged binary and references secrets only in step `env:`;
      `homebrew` runs only when `publishing`, never on a rehearsal
- [ ] `actionlint`, `shellcheck`; `cargo test -p ngterm-update` and clippy on the crate

### Task 9: [Final] Documentation

- [ ] `docs/release-signing.md`, an Apple section: Developer ID Application and Installer certificates exported as
      `.p12`, the notary API key, the team id and `trust/apple-team-id`, the `ios-release` secrets
- [ ] `docs/macos.md`: the signing notes
- [ ] README: the macOS packages are Developer ID signed and notarized; installing through Homebrew; the App Store
      link
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

*Items requiring manual intervention or external systems.*

- On a real Mac, from the first signed release: the DMG and PKG open with no Gatekeeper prompt, and a tool in a pane
  gets a TCC prompt (microphone or camera).
- App Store Connect: App Privacy, the age rating, Submit for Review.
- `NGTERM_RELEASE_SIGNING=required` once the Windows plan signs too.
