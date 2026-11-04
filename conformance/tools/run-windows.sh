#!/usr/bin/env bash
# The Windows counterpart of run-linux.sh, run from Git Bash: build the app, the CLI and the live sessions' server
# (release), stage them with scripts/stage.sh (bin\ngterm.exe, bin\ngtermctl.exe and bin\ngterm-session.exe beside
# share\ghostty and share\terminfo), run the protocol, cli, behaviors and ui layers against the stage, then hold the
# Windows ratchet: every id in conformance/rust-passing-windows.txt must still pass. Every instance comes from the
# harness (a fresh root under this run's own %TEMP%\agt-run-* parent holding its home, application data, state dir and
# control socket, whose named pipe its path names; NGTERM_* and AGTERM_* stripped; its process tree reaped by
# pid at the stop). Its window opens on this desktop: run it on a CI runner or a throwaway VM only, never beside a
# live ngterm.
#
#   conformance/tools/run-windows.sh [--stage DIR] [--out DIR] [--tests DIR] [--coverage]
#                                         [--ratchet FILE] [--ratchet-only [--also SELECTOR]... [--also-k EXPR]...]
#                                         [-- PYTEST_ARGS...]
#
# The options are run-linux.sh's, --coverage included (an instrumented build in target\llvm-cov-target staged into
# target\conformance-stage-cov, its profiles in target\llvm-cov-target, merged after the suite into DIR\coverage).
# The build needs the libghostty-vt prefix (scripts/build-ghostty-vt.sh) and the terminal fonts
# (scripts/fetch-fonts.sh); its HTML overlays need the WebView2 runtime. The panes run Git for Windows' sh.exe (the
# harness's CONFORMANCE_POSIX_SHELL, else the one beside git). Reports go to conformance/reports/rust-windows by
# default. Exits with check_ratchet.py's status (0 holds, 1 broken, 2 no usable report), 1 when a run left an
# instance root behind, or 2 when --coverage yields no code coverage report. No lock: one suite per machine, as a
# runner or a VM runs it.
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
coverage=""
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
        --coverage) coverage=1; shift ;;
        -h|--help) sed -n '2,25s/^# \{0,1\}//p' "${BASH_SOURCE[0]}"; exit 0 ;;
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
for layer in protocol cli behaviors ui; do
    if [[ -d "$tests/$layer" ]]; then layers+=("$tests/$layer"); fi
done
((${#layers[@]})) || { echo "no protocol, cli, behaviors or ui layer under $tests" >&2; exit 2; }

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
# absolute: the build runs from the repo and the harness's children from their instance homes
target="$(cygpath -ua "${CARGO_TARGET_DIR:-$repo/target}")"
# --coverage: the instrumented build's own target dir, where its profiles go and the report reads them.
cov_target="$target/llvm-cov-target"
if [[ -n "$coverage" && -n "$stage" ]] && ! "$python" -m agterm_conformance.llvm_profiles "$(win "$stage/bin")"; then
    exit 2
fi
if [[ -z "$stage" && -n "$coverage" ]]; then
    stage="$target/conformance-stage-cov"
    # as below, instrumented: show-env's rustc wrapper adds -C instrument-coverage and cfg(coverage) in a target dir
    # of its own, cleaned of the last run's workspace build and profiles first
    (
        cd "$repo" || exit
        export CARGO_TARGET_DIR="$(win "$cov_target")"
        eval "$(cargo llvm-cov show-env --sh)" || exit
        cargo llvm-cov clean --workspace || exit
        cargo build --release -p ngterm-gpui -p ngterm-ctl -p ngterm-session || exit
        rm -rf "$stage"
        "$repo/scripts/stage.sh" "$stage"
    )
    "$python" -m agterm_conformance.llvm_profiles "$(win "$stage/bin")" || exit 2
elif [[ -z "$stage" ]]; then
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
while IFS= read -r name; do unset "$name"; done < <(compgen -e | grep -E '^(NGTERM_|AGTERM_)' || true)

mkdir -p "$out"
rm -rf "$out/traces"
rm -f "$out/report.md" "$out/junit.xml" "$out/pytest.log" "$out/nodeids.jsonl" "$out/lost-profiles.txt"
# --coverage: this run's profiles only, pooled per binary; the harness reads the directory from the environment.
if [[ -n "$coverage" ]]; then
    mkdir -p "$cov_target"
    rm -f "$cov_target"/*.profraw "$cov_target/lost-profiles.txt"
    export CONFORMANCE_COVERAGE_DIR="$(win "$cov_target")"
fi
# A per-run parent under %TEMP%, so a concurrent run's roots are neither blamed on this one nor listed for removal.
parent="$(mktemp -d "$(cygpath -u "${TEMP:-/tmp}")/agt-run-XXXXXX")"
trap 'rmdir "$parent" 2>/dev/null || true' EXIT
export CONFORMANCE_TMP="$(win "$parent")"

win_layers=()
for layer in "${layers[@]}"; do win_layers+=("$(win "$layer")"); done
set +e
# The WebView2 runtime's cold start, paid once here and not by the suite's first page (tools/warm_webview2.py); not
# under --coverage, whose instrumented app would write a profile of its own.
if ((${#bin_args[@]})) && [[ -z "$coverage" ]]; then
    "$python" "$(win "$here/warm_webview2.py")" "${bin_args[@]}"
fi
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
profiles=0
lost=0
if [[ -n "$coverage" ]]; then
    profiles="$(find "$cov_target" -maxdepth 1 -name '*.profraw' | wc -l)"
    if [[ -f "$cov_target/lost-profiles.txt" ]]; then
        cp "$cov_target/lost-profiles.txt" "$out/lost-profiles.txt"
        lost="$(wc -l < "$out/lost-profiles.txt")"
    fi
fi

# --coverage: merge this run's profiles against the instrumented build (agterm_conformance/code_coverage.py) into
# DIR/coverage; no profiles or a stage that is not that build exits 2 with the reason.
coverage_error=""
if [[ -n "$coverage" ]]; then
    rm -rf "$out/coverage"
    stage_bin="$(win "$(cygpath -ua "$stage/bin")")"
    coverage_out="$(win "$(cygpath -ua "$out/coverage")")"
    if ! coverage_error="$(
        cd "$repo" || exit 2
        export CARGO_TARGET_DIR="$(win "$cov_target")"
        llvm_cov_env="$(cargo llvm-cov show-env --sh 2>/dev/null)" \
            || { echo "code coverage: cargo llvm-cov show-env failed"; exit 2; }
        eval "$llvm_cov_env"
        "$python" -m agterm_conformance.code_coverage --profiles "$(win "$cov_target")" --stage-bin "$stage_bin" \
            --release-dir "$(win "$cov_target/release")" --repo "$(win "$repo")" --out "$coverage_out" 2>&1 >/dev/null
    )"; then
        status=2
        echo "$coverage_error" >&2
    fi
fi
# The ui ids a passing test of this run exercised (agterm_conformance/coverage.py), counted against the catalog
# files the run used (nodeids.jsonl names them, --catalog/--ui-ids in the pytest args included).
catalog_line="$("$python" -m agterm_conformance.coverage "$(win "$out/nodeids.jsonl")" 2>&1)" || true

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
    if [[ -n "$coverage" ]]; then
        echo "- coverage: $profiles profile files in \`$(win "$cov_target")\`, $lost processes ended without one" \
            "$( ((lost)) && echo '(lost-profiles.txt)')"
    fi
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
    if [[ -n "$coverage" ]]; then
        echo
        echo "### code coverage"
        echo
        if [[ -z "$coverage_error" ]]; then
            cat "$out/coverage/summary.md"
            echo
            echo "lcov: \`$(win "$out/coverage/lcov.info")\`, html: \`$(win "$out/coverage/html/index.html")\`"
        else
            echo "no report: ${coverage_error#code coverage: }"
        fi
    fi
    echo
    echo "### catalog coverage"
    echo
    echo "$catalog_line"
    if [[ -n "$left" ]]; then
        echo
        echo "Left behind (kept for diagnosis, remove by hand):"
        printf -- '- `%s`\n' $left
    fi
} > "$out/report.md"

echo "report: $out/report.md"
exit "$status"
