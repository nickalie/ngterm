---
worth: yes
where: .github/workflows/ios-release.yml
added: 2026-11-06
---
# The iOS release workflow has never signed, exported or uploaded a build

`ios-release.yml` (Task 17 of `docs/plans/20261103-ios-app.md`) has run only as a dry run without secrets:

- it made the unsigned Release archive;
- it checked the archive (no fake core, the export compliance key, the privacy manifest against the binary) and
  `mobile/ios/ExportOptions.plist`;
- it skipped the export, because exporting signs.

These paths have never run:

- the signed archive (automatic signing through the App Store Connect API key);
- `xcodebuild -exportArchive` with `destination = export`;
- the upload (`destination = upload`);
- the distribution-certificate fallback.

They need the user's Apple Developer setup: the bundle id, the app record, an Admin API key, and the secrets in
`docs/ios-release.md` > Secrets. An upload is a publish, so the autonomous run didn't attempt one.

Once the secrets are set, run Actions > ios-release > Run workflow with `dry_run` on. That signs and exports
without uploading. Then push a release tag, or rerun with a tag and `dry_run` off, for the first TestFlight build.

At the first upload, App Store Connect asks for the build's export compliance documentation, since
`ITSAppUsesNonExemptEncryption` is `YES`. That means the French encryption declaration while the app is offered in
France (`docs/ios-release.md` > Export compliance). Once Apple approves it, add its code to `Info.plist` as
`ITSEncryptionExportComplianceCode`.
