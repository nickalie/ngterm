# Releasing ngterm

A release is a `vMAJOR.MINOR.PATCH` tag, one approval, and `.github/workflows/release.yml`. It builds every platform,
signs what has a key, verifies the result and publishes a GitHub release with:

| asset | signed with |
|---|---|
| Linux x86_64 and aarch64: `.deb`, `.rpm`, Arch `.pkg.tar.zst`, `.AppImage`, `.tar.gz` | covered by `SHA256SUMS.minisig` |
| Windows x86_64: per-user `.msi`, portable `.zip` | unsigned for now: Authenticode through SignPath is `docs/plans/20261107-windows-signing.md` |
| macOS Intel and Apple Silicon: `.pkg`, `.dmg`, `.app.zip`, `.tar.gz` | ad-hoc, with the hardened runtime and ngterm's entitlements; Developer ID and notarization are `docs/plans/20261107-apple-signing.md` |
| iOS: `ngterm-<version>-ios-unsigned.ipa` | unsigned (installs once re-signed); TestFlight and the App Store get the signed app from `ios-release.yml` (`docs/ios-release.md`) |
| Android: `ngterm-<version>-android.apk` | ngterm's APK key, which Google Play will also sign with |
| Linux web engine runtime, per architecture: `ngterm-web-engine-<key>-linux-<arch>.tar.zst`, on its own `web-engine-<key>` release | listed in `SHA256SUMS`; the app itself trusts only the size and SHA-256 compiled into it |
| `SHA256SUMS`, `SHA256SUMS.minisig` | the minisign key; every asset also gets a GitHub build provenance attestation |

Installed builds update themselves from these releases (docs/development.md > Downloads and releases). They trust exactly the
minisign key their source commit carries, `crates/ngterm-update/trust/minisign.pub`, and refuse a release whose
`SHA256SUMS` signature doesn't verify against it or whose trusted comment isn't `ngterm v<version> SHA256SUMS`.

## Runs

| run | what it does |
|---|---|
| a push of a `v*` tag | publishing: waits for the approval, then builds, signs, verifies and publishes that tag's release |
| `gh workflow run release --ref vX.Y.Z -f rehearse=true` | a rehearsal: approved and signed exactly as a release, with the real keys, but nothing is published; the packages stay workflow artifacts. `-f platforms=android,linux` limits it to some platforms |
| `gh workflow run release --ref vX.Y.Z -f publish=true` | publishes, or re-uploads, that tag's assets |
| `gh workflow run release --ref <any ref>` | a dry run: builds the `platforms` input's platforms at that ref's version, signs with throwaway keys only, publishes nothing |

- The tag must equal `v` plus the workspace version in `Cargo.toml`, and the version job checks it before anything
  builds. The Android version code is `major * 1_000_000 + minor * 1_000 + patch`.
- Only a signed run (publishing or a rehearsal) reaches the `release` environment, which holds every key, and only
  after the `approve` job's `release-gate` environment is approved by the release reviewer. Building (cargo, Gradle,
  Xcode) happens in jobs with no environment; the signing jobs run no build and restore no cache; verify jobs check
  the signed output without secrets. `scripts/check-release-workflow.py`, run by the lint CI, keeps these rules.
- The Linux build job takes its portable WPE WebKit prefix from GHCR, unpacked only once its provenance verifies as
  `release-wpe-cache.yml`'s on main.
- The Linux build job reuses the runtime already published for its key, or builds it, and compiles its URL, size and
  SHA-256 into the app. Only a publishing run writes a `web-engine-<key>` release, as a prerelease with `--latest=false`
  so that `releases/latest` keeps naming the app, and before the app release leaves draft; dry runs and rehearsals keep
  the runtime as a workflow artifact. Each architecture has its own release, since the key covers the architecture.
- Never delete a `web-engine-*` release that a shipped version pins, and never replace its archive: that version could
  no longer install its engine. `release-artifacts.py web-engine-publish` fails when an archive under a published key
  differs.
- The app release's generated notes start at the previous stable `vX.Y.Z` release, named explicitly, so they never start
  at a `web-engine-*` release.
- Pushing a tag starts a publishing run at once. To rehearse a tag first, reject (or cancel) that run's approval, then
  dispatch the rehearsal, which queues behind it on the same tag.
- A tag stays on the commit it was made at. When a rehearsal finds something to fix, the fix goes out under a new
  patch version, never by moving the tag (the tag ruleset lets only an admin create, move or delete `v*` tags).

## Making a release

1. Set the workspace version in `Cargo.toml`, update `Cargo.lock` (`cargo update -w`), commit and push to `main`.
2. Push the tag: `git tag vX.Y.Z && git push origin vX.Y.Z`.
3. Approve the run's `approve` job in Actions (the `release-gate` environment). The run then builds, signs and
   verifies every platform.
4. When `publish` succeeds, the release is out of draft. A failure anywhere before `publish` publishes nothing.

To check a release before it goes out, reject the tag push's approval in step 3, run a rehearsal of the same tag
(`-f rehearse=true`), look at its artifacts, then `gh workflow run release --ref vX.Y.Z -f publish=true`.

## Keys and settings

Every secret is kept, age-encrypted, in the private repository `nickalie/ngterm-release-keys` and mirrored to
Bitwarden; its `add.sh`, `sync.sh` and `bitwarden.sh` add a secret, send it to GitHub and mirror it. GitHub's copies
can't be read back, so that repository is where a lost or rotated key is restored from.

| name | kind | where | read by | what |
|---|---|---|---|---|
| `MINISIGN_SECRET_KEY` | secret | `release` | `publish` | signs `SHA256SUMS` (`minisign -G -W`: no password); its public key is `crates/ngterm-update/trust/minisign.pub` |
| `ANDROID_APK_KEYSTORE` | secret | `release` | `android-sign` | the APK key's PKCS12 keystore, base64 |
| `ANDROID_APK_KEYSTORE_PASSWORD`, `ANDROID_APK_KEY_PASSWORD` | secret | `release` | `android-sign` | its passwords |
| `ANDROID_APK_KEY_ALIAS` | variable | `release` | `android-sign` | its alias, `ngterm`. A variable on purpose: GitHub masks a secret's value in every log and drops any job output holding it, and `ngterm` is in every file name |
| `NGTERM_RELEASE_SIGNING` | variable | repository | `version` | `optional` (default) or `required` |
| `APPLE_*` | secret, variable | `release` | `macos-sign` | Developer ID: `docs/plans/20261107-apple-signing.md` |

`scripts/setup-release-protection.sh`, run once by the repository admin, creates the two environments
(`release-gate` with the reviewer, `release` limited to `v*` tags) and the tag ruleset. It was run on 2026-11-07.

### Signing mode

- A platform with its keys always signs. Without them, `optional` mode ships it unsigned with a notice (the APK stays
  out of the release, since an unsigned APK can't be installed), and `required` mode fails the run.
- A committed trust anchor makes its own component required regardless of the mode: a non-empty
  `trust/minisign.pub` requires the `SHA256SUMS` signature, a non-empty `packaging/android/apk-certificate.sha256`
  requires the APK key, and a non-empty `trust/apple-team-id` requires Developer ID.
- Set `NGTERM_RELEASE_SIGNING=required` only once macOS and Windows sign too: it covers every platform.

### The keys that can't be replaced

- **The APK key** (`packaging/android/apk-certificate.sha256` holds its certificate's SHA-256). An installed APK
  accepts updates only signed with it, and Google Play will hold a copy as its app signing key, so a lost key means
  users must uninstall to update. It was generated with `keytool -genkeypair` (PKCS12, RSA 4096, alias `ngterm`,
  100 years, a random password); the keys repository and Bitwarden are its backups. `android-sign` refuses any key
  whose certificate isn't the committed one.
- **The minisign key.** Installed builds trust only `trust/minisign.pub`; a new key reaches users only through a
  release signed with the old one that ships the new public key.

## Verifying a download

```sh
# the signature over the checksums, then the files against them
minisign -Vm SHA256SUMS -x SHA256SUMS.minisig -P RWSX4LSrCmx6JMEuC+YIcKqnCodkV06ovAhb1umcA7Jcl0XOiecnn9ZP
sha256sum --check --ignore-missing SHA256SUMS
# where a file was built: this repository's release workflow, at the tag's commit
gh attestation verify ngterm-0.1.0-linux-x86_64.tar.gz --repo nickalie/ngterm
# the APK's signer: compare with packaging/android/apk-certificate.sha256
apksigner verify --print-certs ngterm-0.1.0-android.apk
```

The trusted comment `minisign` prints must be `ngterm v<version> SHA256SUMS` for the release you downloaded.
