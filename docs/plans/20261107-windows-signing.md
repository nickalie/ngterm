# Windows signing: Authenticode through SignPath Foundation, and winget

## Overview

- Split from `docs/plans/20261105-release-signing.md` (user, 2026-11-07), which delivers the GitHub release with the
  Windows MSI and portable zip unsigned. This plan signs them:
  - Authenticode through SignPath Foundation (free code signing for open-source projects) on the four exes, inside the
    MSI and the portable zip, and on the MSI itself, so SmartScreen shows the publisher;
  - every exe carries a `VERSIONINFO` resource, which SignPath Foundation requires;
  - winget: the signed MSI in `microsoft/winget-pkgs`.
- Built on the release plan's job graph (`windows-build`, `windows-verify`, `checksums`), its signed rehearsal runs
  (its Task 4) and its mode handling.
- Credentials are requested as their task lands, one step at a time, through the private keys repository
  `nickalie/ngterm-release-keys` (`./add.sh`, `./bitwarden.sh push`, `./sync.sh`), never through the chat; the
  release plan's Overview has the details.
- Long lead: the SignPath Foundation application is reviewed by hand. It is started with Task 1, so the approval can
  arrive while the version resources land.
- `NGTERM_RELEASE_SIGNING=required` covers every platform, so it is set only once this plan and the Apple plan sign.

## Context (from discovery)

- `.github/workflows/release.yml`: `windows-build` (no secrets) builds the unsigned MSI, the portable zip and the
  upgrade-test MSI and uploads them as plain files, because SignPath opens the artifact itself and can't read tar.
  `windows-verify` and `checksums` take `windows-build`'s manifest directly; there is no signing job yet.
- `scripts/build-msi.sh` (`--arch`, the font notices option) and `scripts/check-msi.ps1` (installs the MSI and runs
  the installed exes, with the upgrade test).
- `crates/ngterm-gpui/windows/ngterm.rc` holds only the icon. The other three exes (`ngtermctl`, the session server,
  the headless server) have no resources. The workspace already depends on `embed-resource`.
- `LICENSE` (MIT) exists since the release plan's Task 1; SignPath Foundation requires one.
- `.github/workflows/windows-installer.yml` builds and checks the MSI on main.

## Development Approach

- As the release plan's: regular testing approach, tests in the same task, all checks green before the next task,
  this file kept in sync with the work.
- Checks are sized to the change (AGENTS.md > Choosing checks): no CI dry runs; a workflow task proves itself with
  `scripts/lint.sh`; never `--target *-windows-*` cargo on the Linux host; Windows behavior is checked by the CI
  workflows on main and in the local Win11 VM, and the signed path by a rehearsal once the credentials exist.

## Testing Strategy

- `windows-verify` (no secrets): `Get-AuthenticodeSignature` is `Valid` on the four exes in the portable zip, on the
  installed exes, and on the MSI when signed; `NotSigned` is accepted only in `optional` mode; then `check-msi.ps1`
  with the upgrade test.
- The version resource: `(Get-Item ...).VersionInfo` in `windows-verify` and in `windows-installer.yml`'s run on main.
- Dry runs take the unsigned path: SignPath's token lives only in `release`, which dry runs never reach.
- Rehearsals with the real token: `windows-verify` showing `Valid`.

## Progress Tracking

- Mark completed items with `[x]` immediately when done.
- Add newly discovered tasks with ➕ prefix; document blockers with ⚠️ prefix.
- Keep the plan in sync with the work.

## Solution Overview

- **`windows-sign`** (`environment: release` on a signed run, no cache, no cargo, no build output executed; the
  release plan's signing-job condition and digest checks): one SignPath request signs the PE files inside the MSI and
  zip, and the MSI itself. Two requests are the fallback (Task 2). It reports the signed files' manifest, which
  `windows-verify`, `checksums` and `publish` then take instead of `windows-build`'s.
- **Mode handling:** with the token it signs in either mode; without it, `optional` writes a notice and ships the
  unsigned output, `required` fails naming it.
- **winget:** a `winget` job (publishing only) submits the manifests after `publish`, only when the MSI is signed.

## Technical Details

- **Secrets** (environment `release`): `SIGNPATH_API_TOKEN`; `WINGET_TOKEN`.
- **Variables** (environment `release`): `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`,
  `SIGNPATH_SIGNING_POLICY_SLUG`, `SIGNPATH_ARTIFACT_CONFIGURATION_SLUG`, plus a second slug if the two-request
  fallback is used.
- **SignPath:**
  - The artifact configuration is committed at `packaging/windows/signpath/` for the user to paste into SignPath.
    It describes the real nesting: the artifact zip holds the MSI and the portable zip, and each of those holds the
    exes.
  - Requests go through the official `signpath/github-action-submit-signing-request` with `wait-for-completion` and a
    timeout long enough for a manual approval.
  - `windows-sign` gets `contents: read` and `actions: read`, so SignPath can fetch the artifact; check this against
    the action's docs.
  - Two-request fallback, used only if one request can't sign the files inside the MSI:
    1. `windows-sign-binaries` signs a zip of the exes;
    2. `windows-msi` (no secrets) builds the MSI and the zip from them;
    3. `windows-sign-msi` signs the MSI.
    - WiX never runs where the token is, but each release then needs two SignPath approvals.
- **Version resources:** every exe carries a `VERSIONINFO` resource (product name, version, company):
  - a shared template at `packaging/windows/version.rc.in`;
  - for `ngterm.exe`, it goes into the existing `crates/ngterm-gpui/windows/ngterm.rc` (its `build.rs` compiles that
    one file);
  - the other three crates get a `build.rs` gated on `CARGO_CFG_TARGET_OS == "windows"`, as `windows_resources()`
    already is.

## Implementation Steps

### Task 1: Version resources on every exe

**Files:**
- Create: `packaging/windows/version.rc.in`
- Modify: `crates/ngterm-gpui/windows/ngterm.rc`
- Create: `build.rs` for `ngterm-ctl`, `ngterm-session`, `ngterm-headless` (gated on `CARGO_CFG_TARGET_OS ==
  "windows"`)
- Modify: `.github/workflows/release.yml` (`windows-verify`), `.github/workflows/windows-installer.yml`

- [ ] a `VERSIONINFO` resource on all four exes through `embed-resource` (the workspace version, product name
      ngterm); the icon stays on `ngterm.exe` only
- [ ] `windows-verify` and `windows-installer.yml` check `(Get-Item ...).VersionInfo` on each exe
- [ ] tests: the version resource on main's `windows-installer.yml` run; `scripts/lint.sh`
- [ ] run checks; must pass before Task 2

**Credentials** (start now, long lead, one step at a time): apply to SignPath Foundation for `nickalie/ngterm`. The
README's code signing policy section it asks for lands in Task 5; draft it with the application if SignPath wants it
up front.

### Task 2: SignPath signing and Authenticode checks

**Files:**
- Create: `packaging/windows/signpath/` (artifact configurations and a README of the SignPath settings)
- Modify: `.github/workflows/release.yml`

- [ ] read SignPath's GitHub integration docs. Record here whether one request can sign the PE files inside the MSI
      and zip as well as the MSI itself; prefer that, otherwise use the three-job fallback from Technical Details, so
      WiX never runs where the token is
- [ ] `windows-sign`: digest check, then the SignPath request(s) with `wait-for-completion` and a long timeout,
      `contents: read` and `actions: read`; mode handling as in Solution Overview. Dry runs take the unsigned path,
      with no token
- [ ] `windows-verify`, `checksums` and `publish` take `windows-sign`'s manifest
- [ ] `windows-verify`: the Authenticode checks of Testing Strategy, then `check-msi.ps1` with the upgrade test
- [ ] tests: `scripts/lint.sh`
- [ ] run checks; must pass before Task 3

### Task 3: SignPath credentials and a signed rehearsal

- [ ] the SignPath Foundation project approved; in SignPath: the project, the signing policy and the artifact
      configuration(s) from `packaging/windows/signpath/`
- [ ] the credentials below stored in the keys repository, mirrored to Bitwarden and synced to `release`
- [ ] a rehearsal with Windows signed: `windows-verify` showing `Valid`; fix what it finds
- [ ] run checks; must pass before Task 4

**Credentials** (one step at a time): an API token for a SignPath CI user, `./add.sh SIGNPATH_API_TOKEN`; not secret,
in `vars.tsv`: `SIGNPATH_ORGANIZATION_ID`, `SIGNPATH_PROJECT_SLUG`, `SIGNPATH_SIGNING_POLICY_SLUG`,
`SIGNPATH_ARTIFACT_CONFIGURATION_SLUG` (and the second slug if the fallback is used); then `./bitwarden.sh push` and
`./sync.sh`.

### Task 4: winget

**Files:**
- Create: `packaging/windows/winget/` (manifest templates: version, installer, defaultLocale), `scripts/winget.py`
  (with `--self-test`)
- Modify: `.github/workflows/release.yml`

- [ ] manifests for `Nickalie.ngterm`: the per-user MSI (`Scope: user`, its ProductCode and UpgradeCode from
      `ngterm.wxs`), the portable zip as a second installer if winget's rules allow both, license, tags, release notes
      URL
- [ ] job `winget` (publishing only, after `publish`): renders the manifests from the release, validates them with
      `winget validate` on a Windows runner, and opens the pull request to `microsoft/winget-pkgs` from the user's fork
      with the pinned `wingetcreate submit`. Runs only when the MSI is Authenticode signed (a notice otherwise)
- [ ] the first submission (a new package) is made the same way; later ones update it
- [ ] tests: `scripts/lint.sh`; `winget validate` and a local `winget install --manifest` in the Win11 VM
- [ ] run checks; must pass before Task 5

**Credentials:** a fork of `microsoft/winget-pkgs` in the user's account and a classic token with `public_repo` for
it: `./add.sh WINGET_TOKEN`, then `./sync.sh`.

### Task 5: [Final] Verify and document

- [ ] every Overview item has its job or step; every Windows secret and variable is read somewhere and documented
- [ ] grep checks: `windows-sign` runs no cargo or WiX and executes no build output; only `windows-sign` adds
      `actions: read`; `winget` runs only when `publishing`, never on a rehearsal
- [ ] `docs/release-signing.md`, a Windows section: the SignPath Foundation application, project, policy, artifact
      configurations, and a privacy note that the updater contacts api.github.com and github.com
- [ ] README: the SignPath code signing policy section; the Windows packages are signed; installing through winget
- [ ] move this plan to `docs/plans/completed/`

## Post-Completion

*Items requiring manual intervention or external systems.*

- On Windows, from the first signed release: SmartScreen shows the publisher, and the MSI's and the exes' signatures
  are `Valid`.
- Each release's SignPath request may need the user's approval in SignPath, as its signing policy says.
- `NGTERM_RELEASE_SIGNING=required` once the Apple plan signs too.
