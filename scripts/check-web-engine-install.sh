#!/usr/bin/env bash
# Acceptance check of the HTML overlay engine on an installed ngterm, run on the machine that holds the install (a test
# VM or a container, never the operator's own session): starts the app, opens scripts/web-engine-check.html, which plays
# a video and Web Audio, and fails unless both played and the engine came from where --expect says.
#
# --expect system    no runtime in the cache, no download, the app's WebKit loaded from outside the cache
# --expect download  the archive served with --serve is downloaded, once, and the runtime is used
# --expect cached    the runtime already in the cache is used, nothing is downloaded
#
# Without --desktop the app runs on its own Xvfb and session bus, this script clicking the page (sound needs a gesture);
# with it, on the display and session bus in the environment (the caller's desktop session), and the caller clicks. NGTERM_* variables are dropped, and the control socket lives in the
# output directory. The app is stopped afterwards; the live-session server it may have started stays for the next run.
#
# Usage: scripts/check-web-engine-install.sh --expect MODE --out DIR [--ngterm BIN] [--ctl BIN] [--serve ARCHIVE]
#          [--state DIR] [--desktop] [--video FILE] [--timeout SECONDS]
# Prints one JSON summary line on stdout.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
expect="" out="" ngterm=ngterm ctl="" serve="" state="" desktop=0 timeout=600
video="$here/../crates/ngterm-gpui/tests/html_overlay/red.webm"
while (($#)); do
  case "$1" in
    --expect) expect="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    --ngterm) ngterm="$2"; shift 2 ;;
    --ctl) ctl="$2"; shift 2 ;;
    --serve) serve="$2"; shift 2 ;;
    --state) state="$2"; shift 2 ;;
    --video) video="$2"; shift 2 ;;
    --timeout) timeout="$2"; shift 2 ;;
    --desktop) desktop=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
case "$expect" in system | download | cached) ;; *) echo "--expect system|download|cached" >&2; exit 2 ;; esac
[[ -n "$out" ]] || { echo "--out DIR" >&2; exit 2; }
[[ "$expect" != download || -n "$serve" ]] || { echo "--expect download needs --serve ARCHIVE" >&2; exit 2; }

for name in $(compgen -e | grep -E '^(NGTERM|AGTERM)_' || true); do unset "$name"; done

mkdir -p "$out/page"
out="$(cd "$out" && pwd)"
cp "$here/web-engine-check.html" "$out/page/index.html"
cp "$video" "$out/page/red.webm"
socket="$out/ctl.sock"
rm -f "$socket"
export NGTERM_CONTROL_SOCKET="$socket"
[[ -z "$state" ]] || export NGTERM_STATE_DIR="$state"
cache="${state:-${XDG_DATA_HOME:-$HOME/.local/share}/ngterm}/web-engine"
if [[ -z "$ctl" ]]; then
  ctl="$(dirname "$(command -v "$ngterm")")/ngtermctl"
  [[ -x "$ctl" ]] || ctl=ngtermctl
fi

server=0 app=0
# shellcheck disable=SC2329  # the trap below runs it
cleanup() {
  ((app == 0)) || kill -TERM -- "-$app" 2>/dev/null || true
  ((server == 0)) || kill "$server" 2>/dev/null || true
}
trap cleanup EXIT

http_log="$out/http.log"
: >"$http_log"
if [[ -n "$serve" ]]; then
  # A port of its own: the kernel picks it, and the server's first line says which.
  python3 -u -m http.server --bind 127.0.0.1 --directory "$(dirname "$serve")" 0 >"$http_log" 2>&1 &
  server=$!
  port=""
  for _ in $(seq 100); do
    port="$(grep -oE 'port [0-9]+' "$http_log" | head -1 | cut -d' ' -f2 || true)"
    [[ -z "$port" ]] || break
    read -r -t 0.1 <> <(:) || true
  done
  [[ -n "$port" ]] || { echo "the local server did not start" >&2; exit 1; }
  NGTERM_WEB_ENGINE_URL="http://127.0.0.1:$port/$(basename "$serve")"
  export NGTERM_WEB_ENGINE_URL
fi

if ((desktop)); then
  setsid "$ngterm" >"$out/app.log" 2>&1 &
else
  # The page's sound needs a click: the middle of the window, over and over until the run ends.
  # shellcheck disable=SC2016  # expanded by the inner shell
  setsid xvfb-run -a -s "-screen 0 1280x800x24" bash -c '
    (n=0; while sleep 1; do
      # the welcome alert of a first launch blocks the page until dismissed (Escape: Later)
      n=$((n + 1)); ((n > 8)) || xdotool key Escape
      largest=0
      for window in $(xdotool search --onlyvisible --name . 2>/dev/null); do
        eval "$(xdotool getwindowgeometry --shell "$window")"
        ((WIDTH * HEIGHT <= largest)) || { largest=$((WIDTH * HEIGHT)); x=$((X + WIDTH / 2)); y=$((Y + HEIGHT / 2)); }
      done
      ((largest == 0)) || xdotool mousemove "$x" "$y" click 1
    done) >/dev/null 2>&1 &
    exec dbus-run-session -- "$1"' _ "$ngterm" >"$out/app.log" 2>&1 &
fi
app=$!

session=""
for _ in $(seq 300); do
  if [[ -S "$socket" ]]; then
    session="$("$ctl" tree --socket "$socket" --json 2>/dev/null | python3 -c '
import json, sys
def walk(node):
    if isinstance(node, dict):
        if node.get("id") and "surfaces" in node:
            print(node["id"]); sys.exit()
        for value in node.values():
            walk(value)
    elif isinstance(node, list):
        for value in node:
            walk(value)
walk(json.load(sys.stdin))' || true)"
    [[ -z "$session" ]] || break
  fi
  kill -0 "$app" 2>/dev/null || { echo "the app exited early; see $out/app.log" >&2; cat "$out/app.log" >&2; exit 1; }
  read -r -t 0.2 <> <(:) || true
done
[[ -n "$session" ]] || { echo "no session came up; see $out/app.log" >&2; exit 1; }

status=0
timeout "$timeout" "$ctl" session overlay open --html "$out/page/index.html" --cwd "$out/page" --js --block \
  --socket "$socket" --target "$session" >"$out/page.json" 2>"$out/page.err" || status=$?
"$ctl" tree --socket "$socket" --json >"$out/tree.json" 2>/dev/null || true

# Which WebKit the engine's processes really mapped.
library=""
for maps in /proc/[0-9]*/maps; do
  library="$(grep -m1 -oE '/[^ ]*libWPEWebKit-2\.0\.so[^ ]*' "$maps" 2>/dev/null || true)"
  [[ -z "$library" ]] || break
done

gets="$(grep -c '"GET ' "$http_log" || true)"
dirs="$(find "$cache" -mindepth 1 -maxdepth 1 -type d ! -name '*.partial' -printf '%f\n' 2>/dev/null | sort | paste -sd, -)"

summary="$(python3 - "$out/page.json" "$expect" "$status" "$library" "$gets" "$dirs" "$cache" <<'PY'
import json, pathlib, sys

raw, expect, status, library, gets, dirs, cache = sys.argv[1:]
status, gets = int(status), int(gets)
problems = []
page = {}
if status:
    problems.append(f"the page did not answer (ngtermctl exit {status})")
else:
    answer = json.loads(pathlib.Path(raw).read_text())
    page = json.loads(answer.get("value") or "{}")
    for media in ("video", "audio"):
        if not page.get(media, {}).get("ok"):
            problems.append(f"{media}: {page.get(media)}")
in_cache = library.startswith(cache)
if expect == "system":
    if gets or dirs:
        problems.append(f"expected no download: {gets} requests, cache has {dirs!r}")
    if not library or in_cache:
        problems.append(f"expected the system's WebKit, mapped {library!r}")
elif expect == "download":
    if gets < 1:
        problems.append("expected a download, the server saw none")
    if not dirs or not in_cache:
        problems.append(f"expected the downloaded runtime: cache {dirs!r}, mapped {library!r}")
else:
    if gets:
        problems.append(f"expected no download, the server saw {gets}")
    if not dirs or not in_cache:
        problems.append(f"expected the cached runtime: cache {dirs!r}, mapped {library!r}")
print(json.dumps({"expect": expect, "ok": not problems, "problems": problems, "library": library, "gets": gets,
                  "runtimes": dirs, "page": page}))
sys.exit(1 if problems else 0)
PY
)" || status=$?
echo "$summary"
exit "$status"
