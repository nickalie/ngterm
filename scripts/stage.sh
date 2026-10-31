#!/usr/bin/env bash
# Assemble the staging layout the conformance runner, scripts/run-gpui-dev.sh and the install check run from:
#
#   bin/{ngterm,ngterm-headless,ngtermctl,ngterm-session}  share/ghostty/{themes,shell-integration}  share/terminfo
#   share/ngterm/session-BUILD (the staged ngterm-session's build id, with bin/ngterm-session)
#   share/ngterm/app-BUILD (the staged ngterm's version and commit, with bin/ngterm)
#   share/ngterm/{agent-status,skills/ngterm}  share/ngterm/shell-integration/pwsh/ngterm.ps1
#
# from cargo's target dir and the vendored prefixes, plus ngterm's PowerShell integration and the agent
# integrations' payloads from resources/. Binaries are staged when they exist (each appears with its crate's build:
# ngterm, ngterm-headless, ngtermctl and the live sessions' server, ngterm-session); Ghostty's resources are
# required. There is no lib/: libghostty-vt is linked statically. The terminfo, themes and shell integration are
# Ghostty's at LIBGHOSTTY_VT_REV, from the libghostty-vt prefix (scripts/build-ghostty-vt.sh).
#
# On Windows (Git Bash) the binaries are bin/{ngterm,ngterm-headless,ngtermctl,ngterm-session}.exe, the rest as on
# Unix; DESTINATION may be given in Windows' form (C:\\...). conformance/tools/run-windows.sh stages there through this
# script.
#
# Usage: scripts/stage.sh DESTINATION [--profile release|debug]   (default: release)
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"

usage() { die "usage: scripts/stage.sh DESTINATION [--profile release|debug]"; }

(($# >= 1)) || usage
DEST="$1"
shift
PROFILE=release
while (($#)); do
  (($# >= 2)) || usage
  case "$1" in
    --profile)
      [[ "$2" == release || "$2" == debug ]] || usage
      PROFILE="$2"
      ;;
    *) usage ;;
  esac
  shift 2
done
EXE=""
if [[ "$(host_os)" == windows ]]; then
  EXE=.exe
  DEST="$(cygpath -u "$DEST")"
fi
[[ "$DEST" = /* ]] || DEST="$PWD/$DEST"
if [[ -d "$DEST" ]] && find "$DEST" -mindepth 1 -print -quit | grep -q .; then
  die "staging destination is not empty: $DEST"
fi

GHOSTTY="$(ghostty_vt_prefix)"
ghostty_vt_prefix_is_complete "$GHOSTTY" \
  || die "no complete libghostty-vt prefix at $GHOSTTY: run scripts/build-ghostty-vt.sh"
TARGET="${CARGO_TARGET_DIR:-$NATIVE_ROOT/target}"
[[ -z "$EXE" ]] || TARGET="$(cygpath -u "$TARGET")"
TARGET="$TARGET/$PROFILE"

mkdir -p "$DEST/bin" "$DEST/share/ghostty"
cp -R "$GHOSTTY/share/ghostty/themes" "$GHOSTTY/share/ghostty/shell-integration" "$DEST/share/ghostty/"
cp -R "$GHOSTTY/share/terminfo" "$DEST/share/terminfo"

staged=()
for binary in ngterm ngterm-headless ngtermctl; do
  if [[ -x "$TARGET/$binary$EXE" ]]; then
    install -m755 "$TARGET/$binary$EXE" "$DEST/bin/$binary$EXE"
    staged+=("$binary$EXE")
  fi
done
# The installed build's identity `version` reports as `installed` (crates/ngterm-core/src/identity.rs): the version
# line and the commit line the staged binary recorded when it was built, printed by that binary (`--build-identity`
# ends before the app starts), never this shell's NGTERM_BUILD_COMMIT or the checkout's version now, so the file names
# the binary beside it even when the two were built and staged apart.
if [[ -x "$DEST/bin/ngterm$EXE" ]]; then
  mkdir -p "$DEST/share/ngterm"
  "$DEST/bin/ngterm$EXE" --build-identity >"$DEST/share/ngterm/.app-BUILD.tmp" \
    || die "$DEST/bin/ngterm$EXE --build-identity failed"
  [[ -n "$(head -n1 "$DEST/share/ngterm/.app-BUILD.tmp")" ]] \
    || die "$DEST/bin/ngterm$EXE --build-identity named no version"
  mv -f "$DEST/share/ngterm/.app-BUILD.tmp" "$DEST/share/ngterm/app-BUILD"
fi
# The live sessions' server and its build id (crates/ngterm-session/src/build_id.rs): the binary is published by a
# rename, then the id, the sha256 of the staged binary, with the size and modification time of the binary it was
# written for, the same way, so a server or client never takes the id of another binary for this one's. `install`
# gives the staged copy a modification time of its own (no -p), so two builds of one size never share both.
if [[ -x "$TARGET/ngterm-session$EXE" ]]; then
  install -m755 "$TARGET/ngterm-session$EXE" "$DEST/bin/.ngterm-session.tmp"
  mv -f "$DEST/bin/.ngterm-session.tmp" "$DEST/bin/ngterm-session$EXE"
  mkdir -p "$DEST/share/ngterm"
  {
    sha256_of <"$DEST/bin/ngterm-session$EXE"
    echo "binary $(size_and_mtime_ns "$DEST/bin/ngterm-session$EXE")"
  } >"$DEST/share/ngterm/.session-BUILD.tmp"
  mv -f "$DEST/share/ngterm/.session-BUILD.tmp" "$DEST/share/ngterm/session-BUILD"
  staged+=("ngterm-session$EXE")
fi

verify_ghostty_resources "$DEST/share" >&2
# ngterm's own PowerShell integration (Ghostty has none) in ngterm's own resources, not Ghostty's, which an inherited
# GHOSTTY_RESOURCES_DIR replaces: a live PowerShell pane's wrap finds it there (crates/ngterm-session/src/launch.rs)
install -Dm644 "$NATIVE_ROOT/resources/shell-integration/pwsh/ngterm.ps1" \
  "$DEST/share/ngterm/shell-integration/pwsh/ngterm.ps1"
# the agent integrations' payloads (upstream's agent-status package and agent skill, see resources/): the Help menu
# installers copy them from share/ngterm
mkdir -p "$DEST/share/ngterm/skills"
cp -R "$NATIVE_ROOT/resources/agent-status" "$DEST/share/ngterm/agent-status"
cp -R "$NATIVE_ROOT/resources/skills/ngterm" "$DEST/share/ngterm/skills/ngterm"
echo "→ staged ${staged[*]:-no binaries} ($PROFILE) into $DEST" >&2
