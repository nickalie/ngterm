# Releasing the iOS app

The iOS app (`mobile/ios`) reaches the App Store through TestFlight. `.github/workflows/ios-release.yml` builds a signed
Release archive, exports it for App Store Connect and uploads it. Submitting a processed build for App Review stays a
manual click in App Store Connect.

## Runs

| run | what it does |
|---|---|
| a release tag `vMAJOR.MINOR.PATCH` (the tag `release.yml` builds the release from) | archives, exports and uploads the tag's build; without the secrets below, archives unsigned and skips the upload with a notice |
| Actions > ios-release > Run workflow on the default branch, `dry_run` on (the default) | archives and exports without uploading; with `tag` empty, the default branch's head |
| Run workflow on the default branch with `tag` set and `dry_run` off | uploads that tag's build, for example after a failed upload |
| Run workflow on another branch | an unsigned dry run of that branch (or of `tag`): it gets no signing secrets |
| a push to a `ci/**` branch that changes `ios-release.yml` or `mobile/ios/ExportOptions.plist` | an unsigned dry run, since a workflow can only be run by hand once it is on the default branch |

- **Versions.** The marketing version (`CFBundleShortVersionString`) is the workspace version in `Cargo.toml`, which
  a tag has to match. The build number (`CFBundleVersion`) is the tag's commit count, so it grows with every release
  from `main`. `scripts/build-ios.sh archive` sets both. Since `manageAppVersionAndBuildNumber` is off, Xcode
  uploads them unchanged.
- **The export.** `mobile/ios/ExportOptions.plist` exports for App Store Connect (`method = app-store-connect`), with
  automatic signing and `destination = export`. An upload sets `destination = upload` in its own copy, with the team
  id. Exporting always signs, so a dry run without the secrets below archives unsigned, checks the archive and the
  export options, and skips the export with a notice. A release tag without them does the same, noting that nothing
  went to TestFlight (the GitHub release still carries the unsigned IPA, from `release.yml`); an upload run by hand
  without them fails at once.
- **The tag.** A release tag has to point at a commit on the default branch. A run of a tag stops before building
  anything otherwise.
- **The checks.** `scripts/build-ios.sh archive` checks every archive, in `ios.yml` and here:
  - the fake core isn't in it;
  - the app answers the export compliance question;
  - the privacy manifest covers the binary.
- **The artifact.** A run keeps the export (the `.ipa`, its `DistributionSummary.plist`) and the archive's dSYMs as
  `ngterm-ios-<version>-<build>`.

## Secrets

Secrets of the `ios-release` environment (Settings > Environments > New environment), never repository secrets:

| secret | what it holds |
|---|---|
| `APP_STORE_CONNECT_KEY_ID` | the App Store Connect API key's id |
| `APP_STORE_CONNECT_ISSUER_ID` | the issuer id shown above the keys (Users and Access > Integrations) |
| `APP_STORE_CONNECT_KEY` | the key's `.p8` file, its whole text |
| `IOS_DEVELOPMENT_TEAM` | the team id (Membership details) |
| `IOS_DISTRIBUTION_CERTIFICATE` | optional: an Apple Distribution certificate with its private key, a `.p12`, base64 |
| `IOS_DISTRIBUTION_CERTIFICATE_PASSWORD` | optional: that `.p12`'s password |

- **The environment's rules.** Under Deployment branches and tags, choose "Selected branches and tags" and add the
  default branch (`main`) and the tag pattern `v*`. The job asks for the environment only on such a ref, and GitHub
  refuses it to any other ref, so a `ci/**` push, a run by hand from another branch, or a branch that edits the
  workflow to ask for it gets no secrets. Optionally, add required reviewers, so that every signing run waits for an
  approval.
- **The tags.** A `v*` tag's run uses the workflow and the scripts of the commit it points at, so whoever can push a
  `v*` tag can sign. Restrict who may create `v*` tags with a tag ruleset (Settings > Rules > Rulesets). The workflow
  also refuses a tag whose commit isn't on the default branch.
- **Each secret reaches one step.** The job's environment holds only whether the secrets are set. The key's text goes
  only to the step that writes its file, after the core is built. The certificate goes only to the step that imports
  it. The archive and export steps get the key's path and ids. No other step, build script or action sees them.
- **The API key needs the Admin role.** Automatic signing on a runner (`-allowProvisioningUpdates` with the key) uses
  cloud-managed distribution certificates and creates the provisioning profiles, and that needs Admin.
- **The certificate is the fallback.** Use it only if cloud-managed signing is refused. The workflow imports it into
  a keychain of the run's own, which automatic signing then finds, and deletes it at the end.

Before the first upload, the user registers the bundle id `com.nickalie.ngterm.ios` and creates the app record in App
Store Connect. The plan's Post-Completion lists these steps.

## Export compliance

**Answer:** `ITSAppUsesNonExemptEncryption = YES` (`mobile/ios/NgTerm/Info.plist`). Checked against Apple's and BIS's
rules on 2026-11-06.

**What the app uses.** The app ships its own cryptography. The ssh client of `crates/ngterm-mobile` is `russh` over
`ring` and the RustCrypto crates, and it encrypts the session's data, not only authenticates it:

- key exchange: curve25519, ECDH and DH;
- ciphers: AES-GCM, AES-CTR, ChaCha20-Poly1305;
- MACs: HMAC-SHA1/SHA2;
- host and user keys: Ed25519, ECDSA and RSA.

These are all published, standard algorithms, used for standard ssh (IETF RFC 4253 and its kin). The app uses no
proprietary cryptography, and it uses no encryption from Apple's operating system for this.

**Apple's rules.**

- "Complying with encryption export regulations" says to set the key to `NO` only when an app uses no encryption, or
  "only uses forms of encryption that are exempt from export compliance documentation requirements". It says to set
  it to `YES` otherwise.
- App Store Connect's "Export compliance documentation for encryption" has a row for an app that "uses an industry
  standard algorithm, not provided within the Apple operating system". The document it requires is the French
  encryption declaration, "only required if you're distributing your app on the App Store in France". A CCATS is
  required only for proprietary algorithms.

**The answer, by where the app is sold.**

- The App Store target includes France, so a document is required, and the key is `YES`.
- Once the French declaration is uploaded and approved, App Store Connect shows a code. Add it to `Info.plist` as
  `ITSEncryptionExportComplianceCode`, so builds stop asking the encryption questions. Until then, each uploaded
  build waits on its compliance answer in App Store Connect (App Encryption Documentation, or Manage next to the
  build).
- If the app is made unavailable in France, Apple's table requires no document. The key could then become `NO`. That
  is a change of the answer, so record it here too.

**The French declaration.** France controls the supply of encryption means (ANSSI, `cyber.gouv.fr`). The
declaration for this app is the user's to file, under the account holder's name, before the app is offered in France.

**BIS (the U.S. EAR).**

- An upload to App Store Connect is an export from the United States, wherever the developer is. So the app is
  subject to the EAR.
- It is mass-market software (Note 3 to Category 5, Part 2) using standard cryptography. That makes it `5D992.c`,
  self-classified, exported under NLR or License Exception ENC.
- **No yearly self-classification report.** Since BIS's rule of 2021-03-29, the annual report of § 740.17(e)(3) (due
  February 1) covers only mass-market encryption components and 'executable software' from hardware components, and
  non-mass-market items. A finished end-user app is neither. It needs no classification request (CCATS) either,
  since it uses no non-standard cryptography.
- What remains is to keep the records of this classification (EAR part 762, five years).

**When to look again.** Review this section when:

- the app adds non-standard cryptography (that needs a CCATS);
- it becomes a component or SDK for other products (that brings back the yearly report);
- or Apple's or BIS's rules change.

Sources:

- Apple, [Complying with encryption export regulations](https://developer.apple.com/documentation/security/complying-with-encryption-export-regulations)
- Apple, [Export compliance documentation for encryption](https://developer.apple.com/help/app-store-connect/reference/app-information/export-compliance-documentation-for-encryption)
- Apple, [Overview of export compliance](https://developer.apple.com/help/app-store-connect/manage-app-information/overview-of-export-compliance)
- eCFR, [15 CFR § 740.17](https://www.ecfr.gov/current/title-15/subtitle-B/chapter-VII/subchapter-C/part-740/section-740.17) ((b)(1) and (e)(3))
- BIS, [Mass market encryption](https://www.bis.gov/learn-support/encryption-controls/mass-market)

## Privacy manifest

`mobile/ios/NgTerm/PrivacyInfo.xcprivacy` declares the following.

- **No tracking.** It sets no tracking and lists no tracking domains.
- **No collected data.** The app talks only to the user's own ssh servers. It sends nothing to the developer or
  anyone else, so the App Store privacy label is "Data Not Collected".
- **Two categories of required-reason API**, found by a scan of the Release binary, not assumed:

| category | reason | what uses it |
|---|---|---|
| `FileTimestamp` | `C617.1` (files inside the app's container) | `fstat`, `fstatat`: the core's libraries (Rust's std reading the app's own executable for a panic's backtrace; libghostty-vt's file helpers) |
| `SystemBootTime` | `35F9.1` (time between events within the app) | `ProcessInfo.systemUptime`: the terminal's double tap and the spacing of its VoiceOver announcements |

**The scan.** `scripts/ios-privacy-check.sh` reads the archived app's executable. It finds the C functions it imports
(`nm -u`) and the Objective-C selectors it calls (`__objc_methname`), and matches them against Apple's list of
required-reason APIs. It fails when:

- a used category isn't declared;
- a declared category is unused;
- a reason isn't one Apple approves for its category;
- the manifest tracks or collects anything.

`scripts/build-ios.sh archive` runs it on every archive. The app's Swift packages link statically into the
executable and carry no manifests of their own, and the test-only packages (swift-snapshot-testing and its
dependencies) are not in the app. So Xcode's privacy report for the archive (Organizer > the archive > Generate
Privacy Report) shows exactly these declarations.
