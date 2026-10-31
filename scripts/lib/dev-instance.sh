# The isolated dev instance of scripts/run-gpui-dev.sh: build, stage and launch this checkout's ngterm beside any
# installed ngterm (the entry script's header describes the instance), under Linux's bash and Git Bash on Windows.
# Sourced, not run.

# shellcheck source=native.sh
source "$(dirname "${BASH_SOURCE[0]}")/native.sh"

# run_dev_instance USAGE_SCRIPT [--release] [--state DIR] [-- NGTERM_ARGS...]
#   USAGE_SCRIPT  the entry script, whose header comment --help prints
run_dev_instance() {
  local usage_script="$1"
  shift
  local profile=debug state="" args=()
  while (($#)); do
    case "$1" in
      --release) profile=release; shift ;;
      --state) state="$2"; shift 2 ;;
      --) shift; args=("$@"); break ;;
      -h | --help) sed -n '2,/^set /{/^set /d;s/^# \{0,1\}//p}' "$usage_script"; exit 0 ;;
      *) die "unknown argument: $1 (see --help)" ;;
    esac
  done

  local windows=0 exe=""
  if [[ "$(host_os)" == windows ]]; then
    windows=1
    exe=.exe
  fi

  # Git Bash's /tmp is the user's TEMP
  root="$(mktemp -d /tmp/ngterm-dev-XXXXXX)"
  # Removed when the script fails before the app starts; exec drops the trap, so a launched instance keeps it.
  trap 'rm -rf "$root"' EXIT
  if [[ -z "$state" ]]; then
    state="$root/state"
  fi
  mkdir -p "$state"
  state="$(cd "$state" && pwd)"
  local socket="$state/ngterm.sock"
  if ((windows)); then
    # the app and ngtermctl take Windows paths; the socket path only names the control pipe, so it has no length limit
    state="$(cygpath -w "$state")"
    socket="$state\\ngterm.sock"
  else
    # sun_path holds 108 bytes on Linux, but the app refuses 104 or more, as upstream's macOS limit.
    ((${#socket} < 104)) || die "socket path too long (${#socket} bytes, the limit is 103): $socket"
  fi

  # the workspace without what links WPE WebKit (ngtermctl among it), then the app with its HTML overlays (which need a
  # WPE WebKit, README > WPE WebKit)
  local profile_args=()
  [[ "$profile" == release ]] && profile_args=(--release)
  (cd "$NATIVE_ROOT" && cargo build "${profile_args[@]}" "${WORKSPACE_WITHOUT_WPE[@]}" \
    && cargo build "${profile_args[@]}" -p ngterm-gpui)
  "$NATIVE_ROOT/scripts/stage.sh" "$root/stage" --profile "$profile"
  local app="$root/stage/bin/ngterm$exe" ctl="$root/stage/bin/ngtermctl$exe"
  [[ -x "$app" ]] || die "no bin/ngterm$exe staged in $root/stage"
  ((windows == 0)) || ctl="$(cygpath -w "$ctl")"

  local name
  while IFS= read -r name; do
    unset "$name"
  done < <(compgen -e | grep -E '^(NGTERM_|AGTERM_|ZMX_)' | grep -vx NGTERM_LOG || true)
  NGTERM_APP_ID="com.nickalie.ngterm.dev.i$(od -An -N6 -tx1 /dev/urandom | tr -d ' \n')"
  export NGTERM_APP_ID
  export NGTERM_STATE_DIR="$state"
  export NGTERM_CONTROL_SOCKET="$socket"

  cat >&2 <<EOF
→ ngterm dev instance ($profile)
  app id: $NGTERM_APP_ID
  state:  $state
  socket: $socket
  ctl:    $ctl <command> --socket $socket
EOF
  trap - EXIT
  if ((windows)); then
    # MSYS's exec would run the app as a child under a pid of its own: start it, print its Windows pid, wait for it
    "$app" "${args[@]}" &
    local child=$!
    echo "  pid:    $(cat "/proc/$child/winpid") (stop it with: taskkill /F /PID <pid>)" >&2
    wait "$child"
  else
    # exec keeps this pid
    echo "  pid:    $$ (stop it with: kill <pid>)" >&2
    exec "$app" "${args[@]}"
  fi
}
