#!/usr/bin/env bash
# The Windows counterpart of run-linux.sh, run from Git Bash: build the app, the CLI and the live sessions' server
# (release), stage them with scripts/stage.sh (bin\ngterm.exe, bin\ngtermctl.exe and bin\ngterm-session.exe beside
# share\ghostty and share\terminfo), run the protocol, cli and behaviors layers against the stage, then hold the
# Windows ratchet: every id in conformance/rust-passing-windows.txt must still pass. Every instance comes from the
# harness (a fresh root under this run's own %TEMP%\agt-run-* parent holding its home, application data, state dir and
# control socket, whose named pipe its path names; NGTERM_*, AGTERM_* and ZMX_* stripped; its process tree reaped by
# pid at the stop). Its window opens on this desktop: run it on a CI runner or a throwaway VM only, never beside a
# live ngterm.
#
#   conformance/tools/run-windows.sh [--stage DIR] [--out DIR] [--tests DIR]
#                                         [--ratchet FILE] [--ratchet-only [--also SELECTOR]... [--also-k EXPR]...]
#                                         [-- PYTEST_ARGS...]
#
# The options are run-linux.sh's. The build needs the libghostty-vt prefix (scripts/build-ghostty-vt.sh) and the
# terminal fonts (scripts/fetch-fonts.sh); its HTML overlays need the WebView2 runtime. The panes run Git for Windows'
# sh.exe (the harness's CONFORMANCE_POSIX_SHELL, else the one beside git). Reports go to
# conformance/reports/rust-windows by default. Exits with check_ratchet.py's status (0 holds, 1 broken, 2 no usable
# report), or 1 when a run left an instance root behind. No lock: one suite per machine, as a runner or a VM runs it.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*) ;;
    *) echo "run-windows.sh runs on Windows (Git Bash); on Linux run run-linux.sh" >&2; exit 2 ;;
esac
stage=""
out=""
tests="$repo/conformance/tests"
ratchet=""
ratchet_only=""
also=()
also_k=()
extra=()

while (($#)); do
    case "$1" in
        --stage) stage="$(cygpath -u "$2")"; shift 2 ;;
        --out) out="$(cygpath -u "$2")"; shift 2 ;;
        --tests) tests="$(cygpath -u "$2")"; shift 2 ;;
        --ratchet) ratchet="$(cygpath -u "$2")"; shift 2 ;;
        --ratchet-only) ratchet_only=1; shift ;;
        --also) also+=("$2"); shift 2 ;;
        --also-k) also_k+=("$2"); shift 2 ;;
        --) shift; extra=("$@"); break ;;
        -h|--help) sed -n '2,23s/^# \{0,1\}//p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done
if [[ -z "$ratchet_only" ]] && ((${#also[@]} + ${#also_k[@]})); then
    echo "--also and --also-k need --ratchet-only" >&2
    exit 2
fi

app=ngterm.exe
out="${out:-$repo/conformance/reports/rust-windows}"
ratchet="${ratchet:-$repo/conformance/rust-passing-windows.txt}"
[[ -f "$ratchet" ]] || { echo "no ratchet list at $ratchet" >&2; exit 2; }
python="${PYTHON:-$repo/.venv/Scripts/python.exe}"
[[ -x "$python" ]] || python="$(command -v python)"
win() { cygpath -w "$1"; }
# This checkout's harness, ahead of whichever checkout the venv's editable install points at; UTF-8 for the CLI's
# output the cli layer reads as text.
export PYTHONPATH="$(win "$repo/conformance")${PYTHONPATH:+;$PYTHONPATH}"
export PYTHONUTF8=1

layers=()
for layer in protocol cli behaviors; do
    if [[ -d "$tests/$layer" ]]; then layers+=("$tests/$layer"); fi
done
((${#layers[@]})) || { echo "no protocol, cli or behaviors layer under $tests" >&2; exit 2; }

selection=()
if [[ -n "$ratchet_only" ]]; then
    selection=(--ratchet-only "$(win "$ratchet")")
    for selector in "${also[@]}"; do
        selection+=(--also "$selector")
        path="${selector#./}"
        path="${path#conformance/}"
        path="$(dirname "$tests")/${path%%::*}"
        path="${path%/}"
        inside=""
        for layer in "${layers[@]}"; do
            if [[ "$path" == "$layer" || "$path" == "$layer"/* ]]; then inside=1; fi
        done
        [[ -e "$path" ]] || { echo "--also $selector: no $path" >&2; exit 2; }
        if [[ -z "$inside" ]]; then layers+=("$path"); fi
    done
    for expression in "${also_k[@]}"; do selection+=(--also-k "$expression"); done
fi

[[ -z "$stage" || -d "$stage" ]] || { echo "no staged tree at $stage" >&2; exit 2; }
if [[ -z "$stage" ]]; then
    target="$(cygpath -u "${CARGO_TARGET_DIR:-$repo/target}")"
    stage="$target/conformance-stage"
    # the app with its HTML overlays (WebView2), the CLI and the live sessions' server
    (cd "$repo" && cargo build --release -p ngterm-gpui -p ngterm-ctl -p ngterm-session)
    rm -rf "$stage"
    "$repo/scripts/stage.sh" "$stage"
fi
bin_args=()
if [[ -x "$stage/bin/$app" ]]; then
    bin_args=(--bin "$(win "$stage/bin/$app")")
fi

# A shell inside a live ngterm or agterm session exports its socket and session; the plugin scrubs these too.
while IFS= read -r name; do unset "$name"; done < <(compgen -e | grep -E '^(NGTERM_|AGTERM_|ZMX_)' || true)

mkdir -p "$out"
rm -f "$out/report.md" "$out/junit.xml" "$out/pytest.log" "$out/nodeids.jsonl"
# A per-run parent under %TEMP%, so a concurrent run's roots are neither blamed on this one nor listed for removal.
parent="$(mktemp -d "$(cygpath -u "${TEMP:-/tmp}")/agt-run-XXXXXX")"
trap 'rmdir "$parent" 2>/dev/null || true' EXIT
export CONFORMANCE_TMP="$(win "$parent")"

win_layers=()
for layer in "${layers[@]}"; do win_layers+=("$(win "$layer")"); done
set +e
started=$SECONDS
"$python" -m pytest "${win_layers[@]}" --impl rust "${bin_args[@]}" -rfE \
    --junitxml "$(win "$out/junit.xml")" --nodeid-report "$(win "$out/nodeids.jsonl")" "${selection[@]}" \
    "${extra[@]}" 2>&1 | tee "$out/pytest.log"
pytest_status=${PIPESTATUS[0]}
elapsed=$((SECONDS - started))
ratchet_log="$("$python" "$(win "$here/check_ratchet.py")" --report "$(win "$out/nodeids.jsonl")" \
    --list "$(win "$ratchet")" 2>&1)"
status=$?
set -e
echo "$ratchet_log"

left="$(find "$parent" -mindepth 1 -maxdepth 1 | sort)"
if [[ -n "$left" && $status -eq 0 ]]; then
    status=1
fi

# A VM's copy of the sources may carry no .git: the report then says so rather than the run ending here.
if build_commit="$(git -C "$repo" rev-parse --short HEAD 2>/dev/null)"; then
    if [[ -n "$(git -C "$repo" status --porcelain)" ]]; then build_commit+=" (dirty)"; fi
else
    build_commit="no git checkout"
fi

{
    echo "## rust-windows conformance"
    echo
    echo "- build: $build_commit, \`$(win "$stage")\`"
    if [[ -n "$ratchet_only" ]]; then
        echo "- mode: ratchet-only, the ids of \`$(basename "$ratchet")\`$( ((${#also[@]})) && printf ' + --also %s' "${also[@]}")$( \
            ((${#also_k[@]})) && printf ' + --also-k %s' "${also_k[@]}")"
    else
        echo "- mode: full"
    fi
    echo "- suite time: ${elapsed}s"
    if ((${#bin_args[@]})); then echo "- ngterm: \`${bin_args[1]}\`"; else echo "- ngterm: not staged"; fi
    echo "- host: $(uname -sr), $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "- layers: ${layers[*]#"$tests/"}"
    if ((${#extra[@]})); then echo "- pytest args: ${extra[*]}"; fi
    echo "- pytest status: $pytest_status"
    echo "- exit status: $status"
    echo
    echo '```'
    grep -E '^rust: ' "$out/pytest.log" || echo "no conformance summary"
    if grep -q 'short test summary info' "$out/pytest.log"; then
        sed -n '/short test summary info/,$p' "$out/pytest.log" | sed '1d'
    else
        tail -n 1 "$out/pytest.log"
    fi
    echo '```'
    echo
    echo "### ratchet"
    echo
    echo '```'
    echo "$ratchet_log"
    echo '```'
    if [[ -n "$left" ]]; then
        echo
        echo "Left behind (kept for diagnosis, remove by hand):"
        printf -- '- `%s`\n' $left
    fi
} > "$out/report.md"

echo "report: $out/report.md"
exit "$status"
