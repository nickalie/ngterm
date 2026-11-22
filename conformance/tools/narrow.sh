# shellcheck shell=bash disable=SC2034,SC2154 # the runner sets the inputs and reads the outputs
# Sourced by run-linux.sh and run-windows.sh: reads the PYTEST_ARGS after `--` (the runner's `extra`) and sets
#   narrowed     the paths and node ids among them, each absolute, its `::` part kept;
#   pytest_args  the rest, options and their values, with --narrowed when narrowed is not empty;
#   partial      1 when they select a part of the suite (-k, -m, or a path or node id).
# Paths and node ids replace the layers, which pytest would otherwise run whole beside them. pytest reads every
# argument that is not an option or an option's value as a path, so each one must be there. Each is spelled as the
# list's ids are (from the suite's root), with conformance/ in front (from the repo's root), absolute, or from the
# working directory. Needs `tests` (the layers' directory, as realpath spells it), `ratchet_only` and `posix_path` (a
# path in the form the shell's tools read). Exits 2 on a path that is not there, or one beside --ratchet-only.

# The options of pytest, its plugins and the harness that take a separate value (selftests/test_run_linux.py checks
# the list against pytest's parser).
value_options=(
    -k -m -p -c -o -r -W --maxfail --pdbcls --capture --cache-show --lfnf --last-failed-no-failures --durations
    --durations-min --verbosity --report-chars --tb --show-capture --color --code-highlight --pastebin --junitxml
    --junit-xml --junitprefix --junit-prefix --pythonwarnings --max-warnings --ignore --ignore-glob --deselect
    --confcutdir --import-mode --doctest-report --doctest-glob --config-file --rootdir --basetemp --debug
    --override-ini --assert --log-level --log-format --log-date-format --log-cli-level --log-cli-format
    --log-cli-date-format --log-file --log-file-mode --log-file-level --log-file-format --log-file-date-format
    --log-auto-indent --log-disable --trace-mode --trace-dir --impl --bin --ctl --macos-launch-mode --must-pass
    --catalog --ui-ids --coverage-baseline --nodeid-report --ratchet-only --also --also-k --shard --shard-weights
    --force-reruns --only-rerun --reruns --reruns-delay --reruns-delay-backoff-factor --rerun-except
    --rerun-exclude-path --reruns-mode --max-suite-reruns
)

# A short option takes its value attached (-ktwo) or as the next argument, also at the end of a cluster (-vk two), as
# argparse reads it. Sets value_of when the next argument is the value, and selects when it is -k or -m.
read_option() {
    local arg="$1" letter i
    if [[ "$arg" == --* ]]; then
        if [[ " ${value_options[*]} " == *" $arg "* ]]; then value_of=1; fi
        return
    fi
    for ((i = 1; i < ${#arg}; i++)); do
        letter="${arg:i:1}"
        [[ "$letter" == [a-zA-Z] ]] || return
        if [[ " ${value_options[*]} " == *" -$letter "* ]]; then
            if [[ "$letter" == [km] ]]; then selects=1; fi
            if ((i == ${#arg} - 1)); then value_of=1; fi
            return
        fi
    done
}

narrowed=()
pytest_args=()
value_of=""
selects=""
for arg in "${extra[@]}"; do
    if [[ -n "$value_of" ]]; then
        pytest_args+=("$arg")
        value_of=""
        continue
    fi
    if [[ "$arg" == -* ]]; then
        pytest_args+=("$arg")
        read_option "$arg"
        continue
    fi
    if [[ -n "$ratchet_only" ]]; then
        echo "--ratchet-only takes its extra node ids and paths with --also, not after --: $arg" >&2
        exit 2
    fi
    path="${arg%%::*}"
    node="${arg:${#path}}"
    path="$(posix_path "$path")"
    relative="${path#./}"
    relative="${relative#conformance/}"
    candidates=("$path")
    [[ "$path" == /* ]] || candidates=("$(dirname "$tests")/$relative" "$path")
    found=""
    for candidate in "${candidates[@]}"; do
        if [[ -e "$candidate" ]]; then found="$(realpath -- "$candidate")"; break; fi
    done
    if [[ -z "$found" ]]; then
        echo "no test path $path for $arg: give it from the repo's root or from conformance/" >&2
        exit 2
    fi
    narrowed+=("$found$node")
done
partial="$selects"
if ((${#narrowed[@]})); then
    pytest_args+=(--narrowed)
    partial=1
fi
