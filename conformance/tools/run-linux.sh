#!/usr/bin/env bash
# Build the Rust workspace, stage it and run the protocol, cli, behaviors and ui layers against it, then hold the
# ratchet: every id in conformance/rust-passing.txt must still pass. Every instance comes from the harness (unique
# NGTERM_APP_ID, fresh root under this run's own /tmp/agt-run-* parent, its own headless weston and session bus; a ui
# instance its own Xvfb on the run's private a11y bus); no window, notification or a11y object reaches the desktop.
#
#   conformance/tools/run-linux.sh [--stage DIR] [--out DIR] [--tests DIR] [--coverage]
#                                       [--ratchet FILE] [--ratchet-only [--also SELECTOR]... [--also-k EXPR]...]
#                                       [-- PYTEST_ARGS...]
#
# --ratchet-only runs only the ids the ratchet list holds, plus each --also directory, file or node-id prefix (a
# path outside the layers is collected too) and each --also-k -k expression, and judges the ratchet as a full run
# does: every listed id must pass, and newly passing ids come only from what ran. The red tests a build short of
# parity has wait out their read-backs, so this is a task's routine run; a full run (the default) stays required at
# milestones (conformance/README.md > Rust runner and ratchet).
# The build (bin/ngterm, staged into target/conformance-stage, reports in conformance/reports/rust-linux) renders
# on Mesa's lavapipe only, from scripts/fetch-lavapipe.sh (exit 2 without one), its weston with --fake-seat.
# --stage runs an already staged tree (scripts/stage.sh layout) instead of building and staging into target/.
# --coverage builds instrumented (cargo llvm-cov show-env, into target/llvm-cov-target after cargo llvm-cov clean
# --workspace), stages into target/conformance-stage-cov, and gives every process of the build a pooled
# LLVM_PROFILE_FILE in target/llvm-cov-target (agterm_conformance/llvm_profiles.py); a --stage not built so exits 2.
# The processes the harness had to end past their exit are listed in DIR/lost-profiles.txt. After the suite
# the profiles are merged, read against each staged program and joined into DIR/coverage (lcov.info, html/,
# summary.md: line, region and function % per workspace crate, vendor/ and the -sys crates excluded); a run that left no profile, or a stage that
# is not the instrumented build, exits 2 with the reason (agterm_conformance/code_coverage.py).
# Without the app's binary in the stage every test skips and parity reads 0%.
# Writes report.md (with the ui ids a passing test exercised), junit.xml, pytest.log and nodeids.jsonl into DIR.
# Exits with check_ratchet.py's status (0 holds, 1 broken, 2 no usable report), 1 when a run left an instance
# root behind, or 2 when --coverage yields no code coverage report; pytest's own status does not count, since most
# tests are red by design until parity.
# CONFORMANCE_LINUX_DISPLAY=session uses this desktop's display and bus instead, for debugging.
# One run at a time per user: a second run waits on the lock $CONFORMANCE_LOCK (default
# $XDG_RUNTIME_DIR/agterm-conformance.lock, else /tmp/agterm-conformance-<uid>/agterm-conformance.lock), shared by
# every checkout. It does not hold back a stand-alone cargo run.
set -euo pipefail

# Set for the child the lock block below runs; cleared before anything spawns, so a nested runner queues again.
locked="${CONFORMANCE_RUN_LOCKED:-}"
unset CONFORMANCE_RUN_LOCKED
args=("$@")

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"
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
        --stage) stage="$2"; shift 2 ;;
        --out) out="$2"; shift 2 ;;
        --tests) tests="$2"; shift 2 ;;
        --ratchet) ratchet="$2"; shift 2 ;;
        --ratchet-only) ratchet_only=1; shift ;;
        --also) also+=("$2"); shift 2 ;;
        --also-k) also_k+=("$2"); shift 2 ;;
        --) shift; extra=("$@"); break ;;
        --coverage) coverage=1; shift ;;
        -h|--help) sed -n '2,35s/^# \{0,1\}//p' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done
if [[ -z "$ratchet_only" ]] && ((${#also[@]} + ${#also_k[@]})); then
    echo "--also and --also-k need --ratchet-only" >&2
    exit 2
fi

# The app binary in the stage, and the defaults.
app=ngterm
out="${out:-$repo/conformance/reports/rust-linux}"
ratchet="${ratchet:-$repo/conformance/rust-passing.txt}"

[[ -f "$ratchet" ]] || { echo "no ratchet list at $ratchet" >&2; exit 2; }
python="${PYTHON:-$repo/.venv/bin/python}"
[[ -x "$python" ]] || python="$(command -v python3)"
# This checkout's harness, ahead of whichever checkout the venv's editable install points at.
export PYTHONPATH="$repo/conformance${PYTHONPATH:+:$PYTHONPATH}"
display="${CONFORMANCE_LINUX_DISPLAY:-headless}"
if [[ "$display" == headless ]]; then
    for tool in weston dbus-daemon; do
        if ! command -v "$tool" >/dev/null; then
            echo "no $tool on PATH: install it, or set CONFORMANCE_LINUX_DISPLAY=session" >&2
            exit 2
        fi
    done
fi

layers=()
for layer in protocol cli behaviors ui; do
    if [[ -d "$tests/$layer" ]]; then layers+=("$tests/$layer"); fi
done
((${#layers[@]})) || { echo "no protocol, cli, behaviors or ui layer under $tests" >&2; exit 2; }

# --ratchet-only: the plugin keeps the listed ids and the extras. An --also path outside the layers (a fixture dir) is
# collected as well; selectors are spelled like the list's ids, relative to the suite's root (the tests dir's parent).
selection=()
if [[ -n "$ratchet_only" ]]; then
    selection=(--ratchet-only "$ratchet")
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

# UI selections need real accessibility bindings even before the AT-SPI driver is enabled.
for layer in "${layers[@]}"; do
    if [[ "$layer" == "$tests/ui" || "$layer" == "$tests/ui/"* ]]; then
        [[ "$display" == headless ]] || { echo "Linux UI instances require headless display mode" >&2; exit 2; }
        "$python" -m agterm_conformance.linux_ui || exit 2
        break
    fi
done

[[ -z "$stage" || -d "$stage" ]] || { echo "no staged tree at $stage" >&2; exit 2; }
# --coverage: the instrumented build's own target dir, where its profiles go and the report reads them.
# Absolute: the build runs from the repo and the harness's children from their instance homes.
target_root="$(realpath -m -- "${CARGO_TARGET_DIR:-$repo/target}")"
cov_target="$target_root/llvm-cov-target"
if [[ -n "$coverage" && -n "$stage" ]] && ! "$python" -m agterm_conformance.llvm_profiles "$stage/bin"; then
    exit 2
fi

# The build renders through Vulkan: on lavapipe and nothing else (the harness points VK_DRIVER_FILES at it), never on
# this machine's GPU driver, so a missing lavapipe stops the run here. Not NGTERM_- or AGTERM_-prefixed: the plugin
# scrubs those.
if ! lavapipe="$("$repo/scripts/fetch-lavapipe.sh")"; then
    echo "no lavapipe for the GPUI build: see scripts/fetch-lavapipe.sh" >&2
    exit 2
fi
export CONFORMANCE_LAVAPIPE_ICD="$lavapipe"
# GPUI panics without a wl_seat, which headless weston offers only with --fake-seat (weston 15.0 and later): a system
# weston without it gives way to the one scripts/build-weston.sh vendors. grep reads the whole help (no -q): an early
# exit could SIGPIPE weston, which pipefail would report as a missing --fake-seat.
if [[ "$display" == headless ]] && ! env -u WAYLAND_DISPLAY -u DISPLAY weston --help 2>/dev/null \
    | grep -- --fake-seat >/dev/null; then
    if ! "$repo/scripts/build-weston.sh" --verify; then
        echo "$(command -v weston) has no --fake-seat, which the GPUI build needs (weston 15.0 or later):" \
            "run scripts/build-weston.sh" >&2
        exit 2
    fi
    PATH="$("$repo/scripts/build-weston.sh" --print-prefix)/bin:$PATH"
    export PATH
fi

# Concurrent suites starve each other's software-rendered pane mounts past their deadlines: queue instead. This shell
# holds the lock and runs the suite in a child without its descriptor, so the instances' session servers, which outlive
# a run by design, do not keep holding it. One descriptor for the try and the wait: no window in which a queued run says
# nothing.
if [[ -z "$locked" ]]; then
    lock="${CONFORMANCE_LOCK:-}"
    if [[ -z "$lock" ]]; then
        lock_dir="${XDG_RUNTIME_DIR:-/tmp/agterm-conformance-$(id -u)}"
        if [[ -z "${XDG_RUNTIME_DIR:-}" ]]; then
            mkdir -p -m 0700 "$lock_dir"
            [[ -O "$lock_dir" ]] || { echo "$lock_dir belongs to another user" >&2; exit 2; }
        fi
        lock="$lock_dir/agterm-conformance.lock"
    fi
    if ! exec {lock_fd}>>"$lock"; then
        echo "cannot open the lock $lock: set CONFORMANCE_LOCK to a writable path" >&2
        exit 2
    fi
    if ! flock -n "$lock_fd"; then
        echo "waiting for another conformance run (lock $lock)"
        flock "$lock_fd" || { echo "cannot take the lock $lock" >&2; exit 2; }
    fi
    status=0
    CONFORMANCE_RUN_LOCKED=1 bash "$here/$(basename "${BASH_SOURCE[0]}")" "${args[@]}" {lock_fd}>&- || status=$?
    exit "$status"
fi

if [[ -z "$stage" && -n "$coverage" ]]; then
    stage="$target_root/conformance-stage-cov"
    # as below, instrumented: show-env's rustc wrapper adds -C instrument-coverage and cfg(coverage) in a target dir
    # of its own, cleaned of the last run's workspace build and profiles first
    # shellcheck source=../../scripts/lib/native.sh
    (
        cd "$repo" && source scripts/lib/native.sh || exit
        export CARGO_TARGET_DIR="$cov_target"
        eval "$(cargo llvm-cov show-env --sh)" || exit
        cargo llvm-cov clean --workspace || exit
        cargo build --release "${WORKSPACE_WITHOUT_WPE[@]}" || exit
        cargo build --release -p ngterm-gpui || exit
        rm -rf "$stage"
        "$repo/scripts/stage.sh" "$stage"
    )
    "$python" -m agterm_conformance.llvm_profiles "$stage/bin" || exit 2
elif [[ -z "$stage" ]]; then
    stage="$target_root/conformance-stage"
    # the workspace without what links WPE WebKit (WORKSPACE_WITHOUT_WPE: ngtermctl is in it), then the app with its
    # HTML overlays, which need a WPE WebKit (README > WPE WebKit)
    # shellcheck source=../../scripts/lib/native.sh
    (
        cd "$repo" && source scripts/lib/native.sh && cargo build --release "${WORKSPACE_WITHOUT_WPE[@]}" || exit
        cargo build --release -p ngterm-gpui
    )
    rm -rf "$stage"
    "$repo/scripts/stage.sh" "$stage"
fi
[[ -d "$stage" ]] || { echo "no staged tree at $stage" >&2; exit 2; }
bin_args=()
if [[ -x "$stage/bin/$app" ]]; then
    bin_args=(--bin "$stage/bin/$app")
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
    export CONFORMANCE_COVERAGE_DIR="$cov_target"
fi
# A per-run parent, so a concurrent run's roots are neither blamed on this one nor listed for removal.
parent="$(mktemp -d /tmp/agt-run-XXXXXX)"
trap 'rmdir "$parent" 2>/dev/null || true' EXIT
export CONFORMANCE_TMP="$parent"

set +e
started=$SECONDS
# -rfE: the hundreds of skips of a build short of parity would bury the failures; the summary line counts them.
"$python" -m pytest "${layers[@]}" --impl rust "${bin_args[@]}" -rfE \
    --junitxml "$out/junit.xml" --nodeid-report "$out/nodeids.jsonl" "${selection[@]}" "${extra[@]}" 2>&1 \
    | tee "$out/pytest.log"
pytest_status=${PIPESTATUS[0]}
elapsed=$((SECONDS - started))
ratchet_log="$("$python" "$here/check_ratchet.py" --report "$out/nodeids.jsonl" --list "$ratchet" 2>&1)"
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
    stage_bin="$(realpath -m -- "$stage/bin")"
    coverage_out="$(realpath -m -- "$out/coverage")"
    if ! coverage_error="$(
        cd "$repo" || exit 2
        export CARGO_TARGET_DIR="$cov_target"
        llvm_cov_env="$(cargo llvm-cov show-env --sh 2>/dev/null)" \
            || { echo "code coverage: cargo llvm-cov show-env failed"; exit 2; }
        eval "$llvm_cov_env"
        "$python" -m agterm_conformance.code_coverage --profiles "$cov_target" --stage-bin "$stage_bin" \
            --release-dir "$cov_target/release" --repo "$repo" --out "$coverage_out" 2>&1 >/dev/null
    )"; then
        status=2
        echo "$coverage_error" >&2
    fi
fi
# The ui ids a passing test of this run exercised (agterm_conformance/coverage.py), counted against the catalog
# files the run used (nodeids.jsonl names them, --catalog/--ui-ids in the pytest args included).
catalog_line="$("$python" -m agterm_conformance.coverage "$out/nodeids.jsonl" 2>&1)" || true

build_commit="$(git -C "$repo" rev-parse --short HEAD)"
if [[ -n "$(git -C "$repo" status --porcelain)" ]]; then build_commit+=" (dirty)"; fi

{
    echo "## rust-linux conformance"
    echo
    echo "- build: $build_commit, \`$stage\`"
    if [[ -n "$ratchet_only" ]]; then
        echo "- mode: ratchet-only, the ids of \`$ratchet\`$( ((${#also[@]})) && printf ' + --also %s' "${also[@]}")$( \
            ((${#also_k[@]})) && printf ' + --also-k %s' "${also_k[@]}")"
    else
        echo "- mode: full"
    fi
    echo "- suite time: ${elapsed}s"
    if ((${#bin_args[@]})); then echo "- ngterm: \`$stage/bin/$app\`"; else echo "- ngterm: not staged"; fi
    echo "- Vulkan: lavapipe only, \`$lavapipe\`"
    echo "- host: $(uname -sr), $(date -u +%Y-%m-%dT%H:%M:%SZ)"
    echo "- layers: ${layers[*]#"$tests/"}"
    echo "- display: $display"
    if [[ -n "$coverage" ]]; then
        echo "- coverage: $profiles profile files in \`$cov_target\`, $lost processes ended without one" \
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
            echo "lcov: \`$out/coverage/lcov.info\`, html: \`$out/coverage/html/index.html\`"
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
