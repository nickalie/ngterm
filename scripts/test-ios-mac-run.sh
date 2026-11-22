#!/usr/bin/env bash
# Tests scripts/ios-mac-run.sh on Linux against a stand-in Mac: a home directory of its own, reached through an `ssh`
# that runs the command locally (rsync's too) and a `lockf` over flock(1). It checks the arguments, the copy and its
# checksum (a file that drifted on the Mac is copied again), the removal of what the tree lost, the lock (held for
# the command, its wait bounded), the time limit, the exit status, the logs and the cleanup. It also tests the iOS
# build lock under an outer lockf (lib/native.sh), the core cache (lib/ios-core-cache.sh), the record of a test-plan
# build (lib/native.sh), the wallpaper cache bound (lib/ios-sim-cache.sh) and the removal of a simulator's hardware
# keyboard state (lib/native.sh, over a stand-in simctl). Needs rsync, flock, perl, python3, git and cargo.
#
# Usage: scripts/test-ios-mac-run.sh
# shellcheck disable=SC2016,SC2034 # check() takes its condition as text and evaluates it later, with the variables it reads
set -euo pipefail

source_root="$(cd "$(dirname "$0")/.." && pwd)"
work="$(mktemp -d)"
cleanup() {
  [[ -z "${holder:-}" ]] || pkill -P "$holder" 2>/dev/null || true
  [[ -z "${holder:-}" ]] || kill "$holder" 2>/dev/null || true
  rm -rf "${work:?}"
}
trap cleanup EXIT
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
export GIT_AUTHOR_NAME=test GIT_AUTHOR_EMAIL=test@example.invalid GIT_COMMITTER_NAME=test
export GIT_COMMITTER_EMAIL=test@example.invalid

failures=0
pass() { printf 'ok   %s\n' "$1"; }
flunk() {
  printf 'FAIL %s\n' "$1" >&2
  failures=$((failures + 1))
}
check() {
  [[ -z "${TEST_DEBUG:-}" ]] || echo "-- status=${status:-} status_bg=${status_bg:-}" >&2
  if eval "$2"; then pass "$1"; else flunk "$1"; fi
}

# --- the stand-in Mac ------------------------------------------------------------------------------------------------

mac="$work/mac"
bin="$work/bin"
mkdir -p "$mac" "$bin"
cat >"$bin/ssh" <<'EOF'
#!/usr/bin/env bash
while (($#)); do
  case "$1" in
    -o) shift 2 ;;
    -*) shift ;;
    *) break ;;
  esac
done
shift
cd "$FAKE_MAC_HOME"
export HOME="$FAKE_MAC_HOME"
unset XDG_CACHE_HOME
# FAKE_MAC_FAIL_OP=<op>: that operation fails (1) and does nothing. FAKE_MAC_LOSE_OP=<op>: the first call of it runs,
# then the connection is lost (255). FAKE_MAC_END_OP=<op>: the run ends during the process check.
# FAKE_MAC_PEEK_OP=<op>: at each echo of that operation, a reader looks at the exit file and records an empty one.
if [[ "$*" == "bash -s" && -n "${FAKE_MAC_FAIL_OP:-}${FAKE_MAC_LOSE_OP:-}${FAKE_MAC_END_OP:-}${FAKE_MAC_PEEK_OP:-}" ]]; then
  script="$(cat)"
  op="${script##*$'\n'op_}"
  [[ "$op" != "${FAKE_MAC_FAIL_OP:-}" ]] || exit 1
  if [[ "$op" == "${FAKE_MAC_LOSE_OP:-}" && ! -e "$FAKE_MAC_HOME/.lost-$op" ]]; then
    : >"$FAKE_MAC_HOME/.lost-$op"
    bash -c "$script" >/dev/null 2>&1 || true
    exit 255
  fi
  if [[ "$op" == "${FAKE_MAC_END_OP:-}" ]]; then
    script="${script%$'\n'op_*}"$'\n''
recorded_alive() {
  [[ "$1" == "$RUN/${FAKE_MAC_END_PID:-pid}" ]] || return 1
  printf "the run ended during the process check\n" >>"$RUN/command.log"
  echo 7 >"$RUN/exit.tmp"
  mv "$RUN/exit.tmp" "$RUN/exit"
  return "${FAKE_MAC_END_ALIVE:-1}"
}
kill() { echo "$*" >>"$RUN/signalled"; return 1; }
'"op_$op"
  fi
  if [[ "$op" == "${FAKE_MAC_PEEK_OP:-}" ]]; then
    script="${script%$'\n'op_*}"$'\n''
echo() {
  if [[ -f "$RUN/exit" && ! -s "$RUN/exit" ]]; then printf "empty\n" >>"$RUN/peeked"; fi
  builtin echo "$@"
}
'"op_$op"
  fi
  exec bash -c "$script"
fi
exec bash -c "$*"
EOF
cat >"$bin/lockf" <<'EOF'
#!/usr/bin/env bash
wait=()
while (($#)); do
  case "$1" in
    -[ksn]*) shift ;;
    -t)
      if (($2 == 0)); then wait=(-n); else wait=(-w "$2"); fi
      shift 2
      ;;
    *) break ;;
  esac
done
lock="$1"
shift
flock -E 75 ${wait[@]+"${wait[@]}"} "$lock" "$@"
EOF
chmod +x "$bin/ssh" "$bin/lockf"
export PATH="$bin:$PATH" FAKE_MAC_HOME="$mac"
mac_lock="$mac/.cache/ngterm/build-ios.flock"

# --- a checkout to copy ----------------------------------------------------------------------------------------------

repo="$work/repo"
mkdir -p "$repo/scripts/lib" "$repo/build" "$repo/src"
cp "$source_root/scripts/ios-mac-run.sh" "$repo/scripts/"
cp "$source_root/scripts/lib/native.sh" "$repo/scripts/lib/"
cp "$source_root/build/pins.env" "$repo/build/"
printf 'ignored/\ntarget/\n' >"$repo/.gitignore"
printf 'one\n' >"$repo/src/a.txt"
printf 'gone soon\n' >"$repo/src/b.txt"
printf '#!/bin/sh\n' >"$repo/src/tool.sh"
chmod +x "$repo/src/tool.sh"
ln -s a.txt "$repo/src/link"
git -C "$repo" init --quiet -b team/test-runner
git -C "$repo" add -A
git -C "$repo" commit --quiet -m test
printf 'untracked\n' >"$repo/src/new.txt"
mkdir -p "$repo/ignored"
printf 'ignored\n' >"$repo/ignored/x.txt"
runner="$repo/scripts/ios-mac-run.sh"
scratch="$mac/.cache/ngterm/run/team-test-runner"
logs="$repo/target/ios-mac-run/team-test-runner"
out="$work/out"
err="$work/err"

# Runs the runner, its output in $out and $err; sets $status.
run() {
  status=0
  timeout 120 "$@" >"$out" 2>"$err" || status=$?
  [[ -z "${TEST_DEBUG:-}" ]] || { echo "== $* -> $status"; cat "$out" "$err"; } >&2
}

# --- arguments -------------------------------------------------------------------------------------------------------

run "$runner"
check "no operation is a usage error" '((status == 2))'
run env IOS_MAC_HOST=fake "$runner" run
check "run with no command is a usage error" '((status == 2))'
run env IOS_MAC_HOST=fake "$runner" run --timeout soon -- true
check "a time limit that is not a number is a usage error" '((status == 2))'
run env IOS_MAC_HOST=fake "$runner" run --fetch ../x -- true
check "a fetch path out of the tree is a usage error" '((status == 2))'
run env IOS_MAC_HOST=fake "$runner" run --name 'a/b' -- true
check "a name with a slash is refused" '((status == 1)) && grep -q "is not 1 to 48" "$err"'
run env -u IOS_MAC_HOST "$runner" run -- true
check "no Mac configured is refused with what to set" '((status == 1)) && grep -q "IOS_MAC_HOST" "$err"'
check "the local config file is git-ignored in the repository" \
  'git -C "$source_root" check-ignore -q scripts/ios-mac-run.local.env'
printf 'IOS_MAC_HOST=fake-mac\n' >"$repo/scripts/ios-mac-run.local.env"
printf 'scripts/ios-mac-run.local.env\n' >>"$repo/.git/info/exclude"

# --- a run: copy, lock, environment, exit status, logs ---------------------------------------------------------------

run env -u IOS_MAC_HOST "$runner" run -- bash -c \
  'cat src/a.txt; echo "lock=$BUILD_IOS_LOCK_HELD stamp=$BUILD_IOS_TREE_STAMP cache=$BUILD_IOS_CORE_CACHE"
  echo "build=$BUILD_IOS_BUILD_NUMBER"; exit 3'
check "the command's exit status is the runner's" '((status == 3))'
check "the command ran in the copied tree" 'grep -qx one "$out"'
check "the command holds the build lock (BUILD_IOS_LOCK_HELD)" 'grep -qF "lock=$mac_lock " "$out"'
check "the command gets the tree's checksum and the core cache" \
  'grep -qE "stamp=[0-9a-f]{64} cache=$mac/.cache/ngterm/ios-core$" "$out"'
check "the tree on the Mac has the untracked file" '[[ -f "$scratch/tree/src/new.txt" ]]'
check "the tree on the Mac has no ignored file" '[[ ! -e "$scratch/tree/ignored/x.txt" ]]'
check "an executable stays executable and a link a link" \
  '[[ -x "$scratch/tree/src/tool.sh" && -L "$scratch/tree/src/link" ]]'
check "the log is copied back" 'grep -qx one "$logs/command.log" && grep -qx "exit: 3" "$logs/run.txt"'
check "the summary says the exit status" 'grep -q "exit 3; copy" "$err"'
check "the command gets the checkout's commit count as the build number" 'grep -qx "build=1" "$out"'
check "the launch frees its claim of the scratch" '[[ ! -e "$scratch/claim" ]]'
stamp1="$(sed -n 's/.*stamp=\([0-9a-f]*\).*/\1/p' "$out")"

# --- the next run: removal, build output kept, drift -----------------------------------------------------------------

rm "$repo/src/b.txt"
mkdir -p "$scratch/tree/target"
printf 'built\n' >"$scratch/tree/target/out.o"
printf 'ONE\n' >"$scratch/tree/src/a.txt"
touch -r "$repo/src/a.txt" "$scratch/tree/src/a.txt"
run "$runner" run -- bash -c 'cat src/a.txt; echo "stamp=$BUILD_IOS_TREE_STAMP"'
check "a file that drifted on the Mac (same size and time) is copied again" \
  '((status == 0)) && grep -qx one "$out" && grep -q "differ on the Mac" "$err"'
check "a file the tree lost is removed on the Mac" '[[ ! -e "$scratch/tree/src/b.txt" ]]'
check "build output on the Mac stays" '[[ -f "$scratch/tree/target/out.o" ]]'
stamp2="$(sed -n 's/.*stamp=\([0-9a-f]*\).*/\1/p' "$out")"
check "the tree's checksum changes with the tree" '[[ -n "$stamp2" && "$stamp1" != "$stamp2" ]]'
run "$runner" run -- bash -c 'echo "stamp=$BUILD_IOS_TREE_STAMP"'
check "an unchanged tree keeps its checksum" 'grep -qx "stamp=$stamp2" "$out"'

# --- a copy that did not end, the claim of the scratch, a start whose connection was lost ----------------------------

printf 'experiment\n' >"$repo/src/experiment.txt"
run env FAKE_MAC_FAIL_OP=verify "$runner" run -- true
check "a copy whose check failed stops the run" \
  '((status == 1)) && [[ -f "$scratch/tree/src/experiment.txt" && ! -e "$scratch/claim" ]]'
rm "$repo/src/experiment.txt"
run "$runner" run -- true
check "the next copy removes a file that only the copy that did not end brought" \
  '((status == 0)) && [[ ! -e "$scratch/tree/src/experiment.txt" ]]'

printf 'other-launch %s\n' "$(date +%s)" >"$scratch/claim"
run "$runner" run -- true
check "a launch is refused while another launch claims the scratch" \
  '((status == 1)) && grep -q "another launch (other-launch)" "$err" && grep -q other-launch "$scratch/claim"'
run "$runner" clean
check "clean refuses while another launch claims the scratch" '((status == 3)) && [[ -d "$scratch" ]]'
printf 'other-launch %s\n' "$(($(date +%s) - 4000))" >"$scratch/claim"
run "$runner" run -- true
check "a claim that was not renewed for an hour is taken over" '((status == 0)) && [[ ! -e "$scratch/claim" ]]'
printf 'other-launch %s\n' "$(date +%s)" >"$scratch/claim"
run "$runner" cancel
check "cancel frees the claim" '((status == 0)) && [[ ! -e "$scratch/claim" ]]'

rm -f "$mac/.lost-start"
run env FAKE_MAC_LOSE_OP=start "$runner" run -- bash -c 'echo started >>"$HOME/starts"'
check "a start whose connection was lost is tried again and does not start a second run" \
  '((status == 0)) && [[ -e "$mac/.lost-start" && "$(grep -c started "$mac/starts")" == 1 ]]'

# --- the time limit, the lock wait, one run per name, cancel ---------------------------------------------------------

run "$runner" run --timeout 2 -- bash -c 'echo $$ >sleeper.pid; sleep 60 & wait'
sleeper="$(cat "$scratch/tree/sleeper.pid" 2>/dev/null || echo 0)"
check "a command past its time limit gives 124" '((status == 124)) && grep -q "stopping it" "$logs/command.log"'
check "the stopped command's process group is gone" \
  '! kill -0 "$sleeper" 2>/dev/null && ! pgrep -g "$sleeper" >/dev/null'
check "the Mac keeps the run's limits for wait" '[[ "$(cat "$scratch/run/limits")" == "2 10800" ]]'

mkdir -p "$(dirname "$mac_lock")"
flock "$mac_lock" sleep 30 &
holder=$!
sleep 1
run "$runner" run --queue-timeout 2 -- true
check "a lock wait past its limit gives 75" '((status == 75)) && grep -q "not free within 2 s" "$logs/command.log"'
pkill -P "$holder" 2>/dev/null || true # flock's child holds the lock too
kill "$holder" 2>/dev/null || true
wait "$holder" 2>/dev/null || true
holder=""

# another time zone in the run's environment: ps writes the start time of the command in it (cancel must still match it)
"$runner" run --env TZ=Asia/Tokyo -- sleep 30 >"$work/bg.out" 2>"$work/bg.err" &
background=$!
for _ in $(seq 1 50); do
  [[ ! -s "$scratch/run/cmdpid" ]] || break
  sleep 0.2
done
check "the command of the background run started" '[[ -s "$scratch/run/cmdpid" ]]'
run "$runner" run -- true
check "a second run of the same name is refused while one runs" '((status == 1)) && grep -q "still going" "$err"'
run "$runner" clean
check "clean refuses while a run is going" '((status == 3)) && [[ -d "$scratch" ]]'
run "$runner" cancel
status_bg=0
wait "$background" || status_bg=$?
check "cancel stops the run, whose runner gives 143" '((status == 0 && status_bg == 143))' || cat "$work/bg.err" >&2
command_pid="$(cat "$scratch/run/cmdpid" 2>/dev/null || echo 0)"
check "cancel stops the command of the live run and its process group" \
  '((command_pid > 0)) && ! kill -0 "$command_pid" 2>/dev/null && ! pgrep -g "$command_pid" >/dev/null'
run "$runner" wait
check "wait on an ended run gives its exit status at once" '((status == 143))'
setsid sleep 30 &
stranger=$!
sleep 0.5
echo "$stranger" >"$scratch/run/cmdpid"
echo "$stranger" >"$scratch/run/pid"
run "$runner" cancel
check "cancel sends no signal to the stored pids of an ended run" '((status == 0)) && kill -0 "$stranger" 2>/dev/null'
run "$runner" clean
check "clean does not take a stored pid of an ended run for a live run" '((status == 0)) && [[ ! -e "$scratch" ]]'
# A run whose job died has no exit status. Its stored pids, which the system gave again to a different process
# (another start time), are not the run's.
lost_run() {
  mkdir -p "$scratch/run"
  for name in pid cmdpid; do
    echo "$stranger" >"$scratch/run/$name"
    echo "Thu Jan  1 00:00:00 1970" >"$scratch/run/$name.start"
  done
}
lost_run
run "$runner" clean
check "clean does not take a pid of a lost run that names a different process for a live run" \
  '((status == 0)) && [[ ! -e "$scratch" ]] && kill -0 "$stranger" 2>/dev/null'
lost_run
run "$runner" cancel
check "cancel sends no signal to a pid of a lost run that names a different process" \
  '((status == 0)) && kill -0 "$stranger" 2>/dev/null && [[ "$(cat "$scratch/run/exit")" == 143 ]]'
run "$runner" clean
# A run whose start times are empty: its processes were gone when the runner recorded them.
mkdir -p "$scratch/run"
for name in pid cmdpid; do
  echo "$stranger" >"$scratch/run/$name"
  : >"$scratch/run/$name.start"
done
run "$runner" cancel
check "cancel sends no signal to a pid whose recorded start time is empty" \
  '((status == 0)) && kill -0 "$stranger" 2>/dev/null'
run "$runner" clean
check "clean does not take a pid with an empty start time for a live run" \
  '((status == 0)) && [[ ! -e "$scratch" ]] && kill -0 "$stranger" 2>/dev/null'
# A run of a runner that wrote no start times: its live pids count, so the scratch stays.
mkdir -p "$scratch/run"
echo "$stranger" >"$scratch/run/pid"
run "$runner" clean
check "clean refuses while a pid with no start time is alive" '((status == 3)) && [[ -d "$scratch" ]]'
rm -rf "${scratch:?}"
kill "$stranger" 2>/dev/null || true
wait "$stranger" 2>/dev/null || true
run "$runner" run --clean -- true
check "--clean removes the scratch after the run" '((status == 0)) && [[ ! -e "$scratch" ]]'
run "$runner" list
check "list runs" '((status == 0)) && grep -q "^free: " "$out"'

# --- a run that ends during a process check -------------------------------------------------------------------------

end_during_check() {
  rm -rf "${scratch:?}/run"
  mkdir -p "$scratch/run" "$scratch/tree"
  printf 'race-run\n' >"$scratch/run/id"
  printf 'race-run %s\n' "$(date +%s)" >"$scratch/claim"
  printf 'result\n' >"$scratch/tree/result.txt"
  : >"$scratch/run/command.log"
}
for alive_status in 0 1; do
  end_during_check
  if ((alive_status == 1)); then : >"$scratch/run/cancelled"; fi
  run env FAKE_MAC_END_OP=poll FAKE_MAC_END_ALIVE="$alive_status" \
    "$runner" wait --logs "$work/race-logs-$alive_status" --fetch result.txt
  check "wait gets the exit status when the process check returns $alive_status as the run ends" \
    '((status == 7)) && grep -qx "exit: 7" "$work/race-logs-$alive_status/run.txt"'
  check "wait fetches the results and frees the claim after process check status $alive_status" \
    'grep -qx result "$work/race-logs-$alive_status/result.txt" &&
      grep -qx "the run ended during the process check" "$work/race-logs-$alive_status/command.log" &&
      [[ ! -e "$scratch/claim" ]]'
  run "$runner" run -- true
  check "the next run can claim the scratch after process check status $alive_status" '((status == 0))'
done

end_during_check
run "$runner" wait
check "wait reports a lost run when its process is gone and it has no exit file" \
  '((status == 1)) && grep -q "its job died" "$err"'
end_during_check
run env FAKE_MAC_END_OP=list FAKE_MAC_END_ALIVE=0 "$runner" list
check "list reports the exit status when the run ends during the process check" \
  '((status == 0)) && grep -q "exit 7$" "$out"'
end_during_check
run env FAKE_MAC_END_OP=cancel FAKE_MAC_END_PID=cmdpid FAKE_MAC_END_ALIVE=0 "$runner" cancel
check "cancel sends no signal when the run ends during the command process check" \
  '((status == 0)) && [[ "$(cat "$scratch/run/exit")" == 7 && ! -e "$scratch/run/signalled" && ! -e "$scratch/claim" ]]'
end_during_check
run env FAKE_MAC_PEEK_OP=cancel "$runner" cancel
check "cancel writes the exit status of a lost run in one step, never as an empty exit file" \
  '((status == 0)) && [[ "$(cat "$scratch/run/exit")" == 143 && ! -e "$scratch/run/peeked" ]] &&
    [[ -z "$(find "$scratch/run" -name "exit.*")" ]]'
run "$runner" clean

# --- the iOS build lock under an outer lockf (lib/native.sh) --------------------------------------------------------

export XDG_CACHE_HOME="$work/cache"
lock="$XDG_CACHE_HOME/ngterm/build-ios.flock"
mkdir -p "$(dirname "$lock")"
native="$source_root/scripts/lib/native.sh"
probe="$work/probe.sh"
printf 'source %q\ntake_ios_build_lock probe "$0"\necho "held=$BUILD_IOS_LOCK_HELD"\n' "$native" >"$probe"
run lockf -k "$lock" bash "$probe"
check "under an outer lockf of the build lock a script goes on, holding it" \
  '((status == 0)) && grep -qx "held=$lock" "$out" && grep -q "outer lockf holds" "$err"'
run lockf -k "$work/other.flock" bash -c "source $(printf %q "$native"); lock_held_by_ancestor $(printf %q "$lock")"
check "another lock file's lockf is no holder" '((status == 1))' || cat "$err" >&2
run env -u BUILD_IOS_LOCK_HELD bash "$probe"
check "with no holder the script takes the lock itself" '((status == 0)) && grep -qx "held=$lock" "$out"'
unset XDG_CACHE_HOME

# --- the core cache (lib/ios-core-cache.sh) --------------------------------------------------------------------------

ws="$work/ws"
for crate in core dep other devonly; do
  mkdir -p "$ws/crates/$crate/src"
  printf 'pub fn f() {}\n' >"$ws/crates/$crate/src/lib.rs"
done
printf '[workspace]\nresolver = "3"\nmembers = ["crates/*"]\n' >"$ws/Cargo.toml"
mkdir -p "$ws/shared"
printf 'one\n' >"$ws/shared/included.txt"
printf 'one\n' >"$ws/shared/unused.txt"
printf 'pub const TEXT: &str = include_str!("../../../shared/included.txt");\n' >>"$ws/crates/dep/src/lib.rs"
for crate in dep other devonly; do
  printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2024"\n' "$crate" >"$ws/crates/$crate/Cargo.toml"
done
printf '[package]\nname = "core"\nversion = "0.1.0"\nedition = "2024"\n[dependencies]\ndep = { path = "../dep" }
[dev-dependencies]\ndevonly = { path = "../devonly" }\n' >"$ws/crates/core/Cargo.toml"
(cd "$ws" && cargo generate-lockfile --offline --quiet)
key() {
  # shellcheck source=lib/native.sh
  # shellcheck source=lib/ios-core-cache.sh
  bash -c 'source "$1/scripts/lib/native.sh"; source "$1/scripts/lib/ios-core-cache.sh"; ios_core_cache_key "$2" core "$3" "rustc 1"' \
    key "$source_root" "$ws" "${1:-release}"
}
key1="$(key)"
check "a core key is made" '[[ "$key1" =~ ^[0-9a-f]{32}$ ]]'
printf '// more\n' >>"$ws/crates/other/src/lib.rs"
printf '// more\n' >>"$ws/crates/devonly/src/lib.rs"
check "a change outside of the core's dependencies keeps the key" '[[ "$(key)" == "$key1" ]]'
check "another profile is another key" '[[ "$(key debug)" != "$key1" ]]'
printf '// more\n' >>"$ws/crates/dep/src/lib.rs"
check "a change in a dependency of the core is another key" '[[ "$(key)" != "$key1" ]]'
key2="$(key)"
printf 'two\n' >"$ws/shared/unused.txt"
check "a change to a file that no source of the core includes keeps the key" '[[ "$(key)" == "$key2" ]]'
printf 'two\n' >"$ws/shared/included.txt"
check "a change to a file that a source of the core includes from outside its package is another key" \
  '[[ "$(key)" != "$key2" ]]'

cache="$work/cache-dir"
core_out="$work/core"
mkdir -p "$core_out/NgtermCoreFFI.xcframework"
head -c 614400 /dev/zero >"$core_out/NgtermCoreFFI.xcframework/lib.a"
printf 'swift\n' >"$core_out/NgtermCoreBindings.swift"
cache_call() {
  bash -c 'source "$1/scripts/lib/native.sh"; source "$1/scripts/lib/ios-core-cache.sh"; shift; "$@"' \
    cache "$source_root" "$@"
}
cache_call ios_core_cache_store "$cache" aaaa "$core_out/NgtermCoreFFI.xcframework" "$core_out/NgtermCoreBindings.swift" \
  1 0 2>"$err"
check "a stored core is found" '[[ "$(cache_call ios_core_cache_entry "$cache" aaaa)" == "$cache/aaaa" ]]'
check "a missing core is not found" '! cache_call ios_core_cache_entry "$cache" bbbb >/dev/null'
cache_call ios_core_cache_store "$cache" bbbb "$core_out/NgtermCoreFFI.xcframework" "$core_out/NgtermCoreBindings.swift" \
  1 0 2>"$err"
check "the cache stays under its size limit: the oldest core goes" \
  '[[ ! -e "$cache/aaaa" && -f "$cache/bbbb/.complete" ]] && grep -q "removed the cached core aaaa" "$err"'
cache_call ios_core_cache_store "$cache" cccc "$core_out/NgtermCoreFFI.xcframework" "$core_out/NgtermCoreBindings.swift" \
  100 99999999 2>"$err"
check "nothing is stored when the disk would have too little free" \
  '[[ ! -e "$cache/cccc" ]] && grep -q "would stay free" "$err"'
mkdir -p "$cache/.tmp-dddd" "$cache/eeee"
cache_call ios_core_cache_prune "$cache" 100000
check "a prune removes stopped stores and incomplete entries" \
  '[[ ! -e "$cache/.tmp-dddd" && ! -e "$cache/eeee" && -d "$cache/bbbb" ]]'

# --- the record of a test-plan build (sync_dir, test_build_record, lib/native.sh) ------------------------------------

generated="$work/generated"
design="$work/design"
mkdir -p "$work/core-a" "$work/core-b" "$design"
printf 'one\n' >"$work/core-a/lib.a"
printf 'two\n' >"$work/core-b/lib.a"
printf 'font\n' >"$design/mono.ttf"
native_call() {
  bash -c 'source "$1/scripts/lib/native.sh"; shift; "$@"' native "$source_root" "$@"
}
record() {
  native_call test_build_record Unit "${1-tree}" debug /xcode "${2:-$generated}" "$design"
}
# as build-ios.sh's step 7 writes Generated/
native_call sync_dir "$work/core-a" "$generated"
record1="$(record)"
sleep 0.1
native_call sync_dir "$work/core-a" "$generated"
check "a test build with the same core keeps its record" \
  '[[ "$record1" == "Unit tree debug /xcode "* && "$(record)" == "$record1" ]]'
native_call sync_dir "$work/core-b" "$generated"
check "a test build with a different core has a different record" '[[ -n "$(record)" && "$(record)" != "$record1" ]]'
check "a test build with no tree stamp has no record" '[[ -z "$(record "")" ]]'
check "a test build whose Generated/ is missing has no record" '[[ -z "$(record tree "$work/none" 2>/dev/null)" ]]'

# --- the wallpaper cache bound (lib/ios-sim-cache.sh) ----------------------------------------------------------------

sims="$work/Devices"
runtime=com.apple.CoreSimulator.SimRuntime.iOS-27-0
# A simulator $1 with $2 cache dirs of 400 KiB each, one more file of 400 KiB in its wallpaper store outside them, and
# one in its data outside the store.
make_sim() {
  local store="$sims/$1/data/Library/Application Support/PRBPosterExtensionDataStore/61/Extensions/poster" i
  for ((i = 0; i < $2; i++)); do
    mkdir -p "$store/configurations/$i/versions/1/scratch/SnapshotCache.cachedb/Snapshot.pks"
    head -c 409600 /dev/zero >"$store/configurations/$i/versions/1/scratch/SnapshotCache.cachedb/Snapshot.pks/a.atx"
  done
  head -c 409600 /dev/zero >"$store/configurations/descriptor.plist"
  head -c 409600 /dev/zero >"$sims/$1/data/app.bin"
}
cache_dirs_of() {
  find "$sims/$1/data" -type d -name 'SnapshotCache.cachedb*' | wc -l | tr -d ' '
}
make_sim PHONE 4
make_sim BOOTED 4
make_sim OTHERNAME 4
make_sim OTHERRUNTIME 4
make_sim LEFTOVER 2
make_sim SMALL 1
leftover="$sims/LEFTOVER/data/Library/Application Support/PRBPosterExtensionDataStore/61/Extensions/poster"
leftover="$leftover/descriptors/0/versions/1/scratch/SnapshotCache.cachedb.ngterm-removed"
mkdir -p "$leftover"
head -c 409600 /dev/zero >"$leftover/b.atx"
device() { printf '{"name": "%s", "udid": "%s", "state": "%s", "dataPath": "%s"}' "$1" "$2" "$3" "$sims/$2/data"; }
cat >"$work/devices.json" <<EOF
{"devices": {
  "$runtime": [$(device ngterm-phone PHONE Shutdown), $(device ngterm-phone BOOTED Booted),
    $(device ngterm-other OTHERNAME Shutdown), $(device ngterm-phone LEFTOVER Shutdown),
    $(device ngterm-phone SMALL Shutdown)],
  "com.apple.CoreSimulator.SimRuntime.iOS-26-0": [$(device ngterm-phone OTHERRUNTIME Shutdown)]
}}
EOF
# a script, not a function: run starts it under timeout(1)
sim_cache_call="$work/sim-cache.sh"
printf 'source %q\nsource %q\n"$@"\n' "$native" "$source_root/scripts/lib/ios-sim-cache.sh" >"$sim_cache_call"
phone_store="$sims/PHONE/data/Library/Application Support/PRBPosterExtensionDataStore"
phone_poster="$phone_store/61/Extensions/poster/configurations"
check "the cache dirs are found below the wallpaper store" \
  '[[ "$(bash "$sim_cache_call" ios_sim_cache_dirs "$phone_store" | wc -l | tr -d " ")" == 4 ]]'
# 1 MiB: the phone (1.6 MiB) and the leftover simulator (1.2 MiB with its renamed dir) are over it, SMALL is not
run bash "$sim_cache_call" ios_sim_cache_bound ngterm-phone "$runtime" 1 <"$work/devices.json"
check "the bound runs" '((status == 0))' || cat "$err" >&2
check "a shut-down pinned simulator over the bound loses its cache dirs" \
  '[[ "$(cache_dirs_of PHONE)" == 0 ]] &&
    grep -q "simulator PHONE: wallpaper cache 1 MiB, over the bound of 1 MiB: removed 4 dirs, 0 MiB left" "$err"'
check "the rest of its wallpaper store and its data stay" \
  '[[ -f "$sims/PHONE/data/app.bin" && -f "$phone_poster/descriptor.plist" &&
    -d "$phone_poster/0/versions/1/scratch" ]]'
check "a booted simulator keeps its cache" \
  '[[ "$(cache_dirs_of BOOTED)" == 4 ]] && grep -q "simulator BOOTED is Booted: its wallpaper cache stays" "$err"'
check "a simulator of another name keeps its cache" '[[ "$(cache_dirs_of OTHERNAME)" == 4 ]]'
check "a simulator of the pinned name on another runtime keeps its cache" '[[ "$(cache_dirs_of OTHERRUNTIME)" == 4 ]]'
check "a renamed dir that an earlier delete left is deleted with the rest" '[[ "$(cache_dirs_of LEFTOVER)" == 0 ]]'
check "a simulator under the bound keeps its cache" \
  '[[ "$(cache_dirs_of SMALL)" == 1 ]] && grep -q "simulator SMALL: wallpaper cache 0 MiB, bound 1 MiB" "$err"'
make_sim PHONE 2
run bash "$sim_cache_call" ios_sim_cache_bound ngterm-phone "$runtime" 1 <"$work/devices.json"
check "a cache at the bound stays" '((status == 0)) && [[ "$(cache_dirs_of PHONE)" == 2 ]]'
odd="$(device ngterm-phone PHONE Shutdown | sed "s#$sims/PHONE/data#$sims/PHONE#")"
printf '{"devices": {"%s": [%s]}}' "$runtime" "$odd" >"$work/odd.json"
run bash "$sim_cache_call" ios_sim_cache_bound ngterm-phone "$runtime" 0 <"$work/odd.json"
check "a data path that is not <udid>/data is left alone" '((status == 0)) && [[ "$(cache_dirs_of PHONE)" == 2 ]]'
run bash "$sim_cache_call" ios_sim_cache_bound ngterm-phone "$runtime" lots <"$work/devices.json"
check "a bound that is not a number fails" '((status == 1)) && grep -q "is not a number of MiB" "$err"'
# 08 is not an octal number: read as 8 MiB, the phone's 0.8 MiB stays
run bash "$sim_cache_call" ios_sim_cache_bound ngterm-phone "$runtime" 08 <"$work/devices.json"
check "a bound with a leading zero is a decimal number" \
  '((status == 0)) && [[ "$(cache_dirs_of PHONE)" == 2 ]] &&
    grep -q "simulator PHONE: wallpaper cache 0 MiB, bound 8 MiB" "$err"'
run bash "$sim_cache_call" ios_sim_cache_bound ngterm-phone "$runtime" 1234567890 <"$work/devices.json"
check "a bound of more than 9 digits fails" '((status == 1)) && [[ "$(cache_dirs_of PHONE)" == 2 ]]'

# --- a simulator's hardware keyboard state (ios_sim_forget_hardware_keyboard, lib/native.sh) ------------------------

kb_sims="$work/KeyboardDevices"
kb_bin="$work/kb-bin"
mkdir -p "$kb_bin"
printf '#!/usr/bin/env bash\n[[ "$*" == "-f simctl" ]] && echo %q\n' "$kb_bin/simctl" >"$kb_bin/xcrun"
cat >"$kb_bin/simctl" <<'EOF'
#!/usr/bin/env bash
case "$1" in
  list) cat "$FAKE_SIMCTL_DEVICES" ;;
  shutdown)
    echo "shutdown $2" >>"$FAKE_SIMCTL_LOG"
    exit "${FAKE_SIMCTL_SHUTDOWN_STATUS:-0}"
    ;;
  *) exit 2 ;;
esac
EOF
chmod +x "$kb_bin/xcrun" "$kb_bin/simctl"
kb_prefs() { printf '%s\n' "$kb_sims/$1/data/Library/Preferences/com.apple.keyboard.preferences.plist"; }
# A simulator $1 whose keyboard preferences hold both hardware keyboard keys and two others, in the format $2 (binary
# or xml).
make_kb_sim() {
  mkdir -p "$kb_sims/$1/data/Library/Preferences"
  python3 -I -c '
import plistlib, sys
prefs = {"AutomaticMinimizationEnabled": True, "HardwareKeyboardLastSeen": True,
         "KeyboardHardwareKeyboardsSeen": [33], "DidShowContinuousPathIntroduction": True}
fmt = plistlib.FMT_BINARY if sys.argv[2] == "binary" else plistlib.FMT_XML
with open(sys.argv[1], "wb") as file:
    plistlib.dump(prefs, file, fmt=fmt)' "$(kb_prefs "$1")" "$2"
}
# The keys of the property list $1, sorted, and its format, on one line.
plist_state() {
  python3 -I -c '
import plistlib, sys
raw = open(sys.argv[1], "rb").read()
print(" ".join(sorted(plistlib.loads(raw))), "binary" if raw.startswith(b"bplist") else "xml")' "$1"
}
kb_state() { plist_state "$(kb_prefs "$1")"; }
kb_device() { printf '{"name": "%s", "udid": "%s", "state": "%s", "dataPath": "%s"}' "$1" "$2" "$3" "$4"; }
# The listing: ngterm-phone is the simulator $1 in the state $2, beside one of another name, one of the same name on
# another runtime and one whose data path is not <udid>/data.
kb_list() {
  printf '{"devices": {"%s": [%s, %s, %s], "%s": [%s]}}' "$runtime" \
    "$(kb_device ngterm-phone "$1" "$2" "$kb_sims/$1/data")" \
    "$(kb_device ngterm-other OTHERNAME Shutdown "$kb_sims/OTHERNAME/data")" \
    "$(kb_device ngterm-odd ODD Shutdown "$kb_sims/ODD")" \
    com.apple.CoreSimulator.SimRuntime.iOS-26-0 \
    "$(kb_device ngterm-phone OTHERRUNTIME Shutdown "$kb_sims/OTHERRUNTIME/data")" >"$work/kb-devices.json"
}
kb_call="$work/kb-call.sh"
printf 'source %q\nIOS_SIMULATOR_RUNTIME=27.0\n"$@"\n' "$native" >"$kb_call"
# Runs ios_sim_forget_hardware_keyboard "$kb_name" over the stand-in simctl, with the environment $@.
kb_run() {
  : >"$work/kb-simctl.log"
  run env PATH="$kb_bin:$PATH" FAKE_SIMCTL_DEVICES="$work/kb-devices.json" FAKE_SIMCTL_LOG="$work/kb-simctl.log" \
    "$@" bash "$kb_call" ios_sim_forget_hardware_keyboard "$kb_name"
}
kb_left="DidShowContinuousPathIntroduction KeyboardHardwareKeyboardsSeen"
kb_all="AutomaticMinimizationEnabled $kb_left HardwareKeyboardLastSeen"
kb_all="$(tr ' ' '\n' <<<"$kb_all" | sort | tr '\n' ' ' | sed 's/ $//')"
make_kb_sim PHONEKB binary
make_kb_sim OTHERNAME binary
make_kb_sim OTHERRUNTIME binary
mkdir -p "$kb_sims/ODD/Library/Preferences"
odd_prefs="$kb_sims/ODD/Library/Preferences/com.apple.keyboard.preferences.plist"
cp "$(kb_prefs OTHERNAME)" "$odd_prefs"
kb_list PHONEKB Shutdown
kb_name=ngterm-phone
kb_run
check "a shut-down simulator loses both hardware keyboard keys, keeps the others and stays shut down" \
  '((status == 0)) && [[ "$(kb_state PHONEKB)" == "$kb_left binary" && ! -s "$work/kb-simctl.log" ]]' || cat "$err" >&2
check "a simulator of another name or on another runtime keeps its keys" \
  '[[ "$(kb_state OTHERNAME)" == "$kb_all binary" && "$(kb_state OTHERRUNTIME)" == "$kb_all binary" ]]'
kb_run
check "a second run finds nothing to remove" '((status == 0)) && [[ "$(kb_state PHONEKB)" == "$kb_left binary" ]]'
make_kb_sim BOOTEDKB xml
kb_list BOOTEDKB Booted
kb_run
check "a booted simulator is shut down first, and its XML file stays XML" \
  '((status == 0)) && [[ "$(cat "$work/kb-simctl.log")" == "shutdown BOOTEDKB" ]] &&
    [[ "$(kb_state BOOTEDKB)" == "$kb_left xml" ]]' || cat "$err" >&2
make_kb_sim BOOTEDKB xml
kb_run FAKE_SIMCTL_SHUTDOWN_STATUS=1
check "a shutdown that fails fails the removal, and the keys stay" \
  '((status == 1)) && [[ "$(kb_state BOOTEDKB)" == "$kb_all xml" ]]'
kb_list NEVERBOOTED Shutdown
kb_run
check "a simulator with no keyboard preferences yet is no failure" \
  '((status == 0)) && [[ ! -e "$kb_sims/NEVERBOOTED" ]]' || cat "$err" >&2
kb_name=ngterm-missing
kb_run
check "no simulator of the name is no failure" '((status == 0)) && [[ ! -s "$work/kb-simctl.log" ]]'
kb_name=ngterm-odd
kb_run
check "a data path that is not <udid>/data is left alone" \
  '((status == 0)) && [[ "$(plist_state "$odd_prefs")" == "$kb_all binary" ]]'

if ((failures)); then
  echo "$failures check(s) failed" >&2
  exit 1
fi
echo "all checks passed"
