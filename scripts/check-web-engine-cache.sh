#!/usr/bin/env bash
# Acceptance check of the per-user engine runtime cache, run on a machine that holds a deb install (a throwaway
# container: scripts/check-web-engine-container.sh ubuntu-24.04-cache):
#
# - offline: `ngterm --install-web-engine` installs from a file with no network, and a tampered archive is refused;
# - versions: an older and a newer app (this install, and a copy pinned to another runtime key) share the data dir, and
#   neither deletes the other's runtime while it runs; once unused, the next install removes it.
#
# The "newer" app is the same build with its compiled-in pins rewritten to a repacked runtime under another key: the
# pins are the only thing two releases' runtime handling differs by, and the build identity still matches its worker.
#
# Usage: scripts/check-web-engine-cache.sh --archive ARCHIVE --out DIR [--video FILE]
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
archive="" out="" video="$here/../crates/ngterm-gpui/tests/html_overlay/red.webm"
while (($#)); do
  case "$1" in
    --archive) archive="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --video) video="$2"; shift 2 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[[ -n "$archive" && -n "$out" ]] || { echo "--archive ARCHIVE --out DIR" >&2; exit 2; }
for name in $(compgen -e | grep -E '^(NGTERM|AGTERM)_' || true); do unset "$name"; done

mkdir -p "$out/page"
cp "$here/web-engine-check.html" "$out/page/index.html"
cp "$video" "$out/page/red.webm"
cache="${XDG_DATA_HOME:-$HOME/.local/share}/ngterm/web-engine"
work="$(mktemp -d "$out/work.XXXXXX")"
older=/opt/ngterm
newer="$work/newer"
ctl="$older/usr/bin/ngtermctl"
pass() { echo "PASS: $*"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
complete() { [[ -f "$cache/$1/MANIFEST" && -f "$cache/$1/lib/libWPEWebKit-2.0.so.1" ]]; }

key_a="$(basename "$archive" | sed -E 's/^ngterm-web-engine-([0-9a-f]+)-linux-.*/\1/')"
[[ ${#key_a} == 64 ]] || fail "no key in the archive's name"
sha_a="$(sha256sum "$archive" | cut -d' ' -f1)"
size_a="$(stat -c %s "$archive")"

# --- offline and tampered installs
tampered="$work/tampered.tar.zst"
cp "$archive" "$tampered"
printf '\xff' | dd of="$tampered" bs=1 seek=$((size_a / 2)) conv=notrunc status=none
if unshare -r -n "$older/usr/bin/ngterm" --install-web-engine "$tampered" 2>"$work/tampered.err"; then
  fail "a tampered archive was installed"
fi
complete "$key_a" && fail "a tampered archive left a runtime"
pass "a tampered archive is refused: $(cat "$work/tampered.err")"
unshare -r -n "$older/usr/bin/ngterm" --install-web-engine "$archive"
complete "$key_a" || fail "the offline install left no runtime"
grep -qx "key=$key_a" "$cache/$key_a/MANIFEST" || fail "the installed MANIFEST names another key"
pass "offline install with no network"

# --- a second runtime under another key, and the app pinned to it
key_b="$(printf 'newer %s' "$key_a" | sha256sum | cut -d' ' -f1)"
mkdir "$work/unpack"
tar --zstd -xf "$archive" -C "$work/unpack"
sed -i "s/^key=.*/key=$key_b/" "$work/unpack/MANIFEST"
archive_b="$work/ngterm-web-engine-$key_b-linux-x86_64.tar.zst"
# The size pin is folded into the build's code, so this test runtime must have the same size: it leaves out libav
# (its check page plays VP8), which makes it smaller, and fills the difference with a zstd skippable frame.
find "$work/unpack/lib" -maxdepth 2 \( -name 'libgstlibav.so' -o -name 'libavcodec.so*' \) -delete
tar -C "$work/unpack" -cf - . | zstd -q -T0 -19 --long=27 -o "$archive_b"
python3 - "$archive_b" "$size_a" <<'PY_PAD'
import pathlib
import struct
import sys

path, size = pathlib.Path(sys.argv[1]), int(sys.argv[2])
fill = size - path.stat().st_size
if fill < 8:
    sys.exit(f"the repacked runtime is too large to pad: {fill} bytes to spare")
with path.open("ab") as output:
    output.write(struct.pack("<II", 0x184D2A50, fill - 8) + bytes(fill - 8))
PY_PAD
cp -a "$older" "$newer"
python3 - "$newer/usr/bin/ngterm" "$key_a" "$key_b" "$sha_a" "$(sha256sum "$archive_b" | cut -d' ' -f1)" <<'PY'
import pathlib
import sys

path, *pairs = sys.argv[1:]
binary = pathlib.Path(path)
data = binary.read_bytes()
for old, new in zip(pairs[::2], pairs[1::2]):
    if len(old) != len(new) or old.encode() not in data:
        sys.exit(f"cannot rewrite the pin {old} -> {new}")
    data = data.replace(old.encode(), new.encode())
binary.write_bytes(data)
PY

# --- an app with a page open: its worker holds its runtime
app=0
stop_app() {
  ((app == 0)) || kill -TERM -- "-$app" 2>/dev/null || true
  if ((app != 0)); then
    for _ in $(seq 100); do
      kill -0 "$app" 2>/dev/null || break
      read -r -t 0.1 <> <(:) || true
    done
  fi
  app=0
}
trap stop_app EXIT
mapped() { grep -lsq "/$1/lib/libWPEWebKit" /proc/[0-9]*/maps; }
start_app() { # BIN KEY NAME
  local socket="$out/$3.sock" session="" _
  rm -f "$socket"
  NGTERM_CONTROL_SOCKET="$socket" setsid xvfb-run -a -s "-screen 0 1280x800x24" dbus-run-session -- "$1" \
    >"$out/$3.log" 2>&1 &
  app=$!
  for _ in $(seq 300); do
    session="$("$ctl" tree --socket "$socket" --json 2>/dev/null | python3 -c '
import json, sys
print(json.load(sys.stdin)["result"]["tree"]["workspaces"][0]["sessions"][0]["id"])' 2>/dev/null || true)"
    [[ -z "$session" ]] || break
    read -r -t 0.2 <> <(:) || true
  done
  [[ -n "$session" ]] || fail "$3: no session came up"
  "$ctl" session overlay open --html "$out/page/index.html" --cwd "$out/page" --js --socket "$socket" --target "$session" \
    >/dev/null 2>&1 || true # a relaunch restores the page the last run left open
  for _ in $(seq 600); do
    if mapped "$2"; then return; fi
    read -r -t 0.2 <> <(:) || true
  done
  fail "$3: the engine never mapped runtime $2"
}

start_app "$older/AppRun" "$key_a" older
pass "the older app runs on its runtime"
"$newer/AppRun" --install-web-engine "$archive_b"
complete "$key_b" || fail "the newer app's install left no runtime"
complete "$key_a" || fail "the newer app's install deleted the runtime the older app runs on"
mapped "$key_a" || fail "the older app's engine stopped"
pass "the newer app's install keeps the older app's runtime while it runs"
stop_app

start_app "$newer/AppRun" "$key_b" newer
pass "the newer app runs on its runtime"
"$older/AppRun" --install-web-engine "$archive"
complete "$key_a" || fail "the older app's runtime went away"
complete "$key_b" || fail "the older app's install deleted the runtime the newer app runs on"
mapped "$key_b" || fail "the newer app's engine stopped"
pass "the older app's install keeps the newer app's runtime while it runs"
stop_app

"$older/AppRun" --install-web-engine "$archive"
[[ ! -e "$cache/$key_b" ]] || fail "a runtime nothing uses was kept"
complete "$key_a" || fail "the kept runtime is gone"
pass "a runtime nothing uses is removed by the next install"
