# shellcheck shell=bash
# Shared helpers of the native build scripts (build-ghostty-vt.sh, build-weston.sh, fetch-wpe.sh, fetch-lavapipe.sh,
# fetch-fonts.sh, gen-wpe-bindings.sh, gen-ghostty-config-keys.sh, pkg-config.sh, stage.sh, lib/dev-instance.sh, and
# the iOS scripts).
# Sourced, not run.
#
# Each native dependency lives in its own vendored prefix, ${NGTERM_VENDOR:-~/.cache/ngterm/vendor}/<name>-<key>/,
# where <key> hashes that dependency's pins and patches only. libghostty-vt's prefix holds a patched source tree, not
# a library: ngterm-ghostty-vt-sys asks `build-ghostty-vt.sh --print-prefix` where the prefix is, fails when it is
# missing, and runs `zig build` over that tree itself, offline (the prefix carries the Zig packages). WPE WebKit is a
# binary package unpacked (fetch-wpe.sh) where the system has none, and reaches cargo through pkg-config.sh. A
# relative NGTERM_VENDOR is taken against the repository root, so every caller (and cargo, which runs from a crate
# directory) sees one path.
#
# Portable shell: this file and build-ghostty-vt.sh run under Linux's bash, stock macOS's bash 3.2 and Git Bash on
# Windows (MSYS2), so they use no bash-4 constructs (`${var^^}`, `mapfile`, associative arrays) and pick each tool
# per OS (sha256_of, find_zig's archive format). On Windows every path the scripts handle is MSYS's (`/c/...`): a Windows-form
# NGTERM_VENDOR is converted on the way in, and native_path converts a path on the way out to a native program.

die() {
  echo "$*" >&2
  exit 1
}

# Run $2... for at most $1 seconds. The status is the command's, or 124 (as coreutils' timeout gives) when the time
# ran out: the command then gets TERM, and KILL 10 s later. Stock macOS has no timeout(1). Give a tool's own path, not a
# launcher such as xcrun: a launcher's child would stay alive after the stop and keep the caller's output open. Safe in
# a `set -e` caller. A command that ends in time returns at once: the watchdog that fired exits 124 when it is stopped.
bounded() {
  local limit="$1" pid watchdog status=0 watched=0
  shift
  "$@" &
  pid=$!
  (
    elapsed=0
    while ((elapsed < limit)); do
      sleep 1 >/dev/null 2>&1
      elapsed=$((elapsed + 1))
    done
    trap 'exit 124' TERM
    echo "${1##*/} ${*:2} still running after ${limit}s: stopping it" >&2
    kill -TERM "$pid" 2>/dev/null || exit 0
    while ((elapsed < limit + 10)) && kill -0 "$pid" 2>/dev/null; do
      sleep 1 >/dev/null 2>&1
      elapsed=$((elapsed + 1))
    done
    kill -KILL "$pid" 2>/dev/null || true
    exit 124
  ) >/dev/null &
  watchdog=$!
  wait "$pid" || status=$?
  kill "$watchdog" 2>/dev/null || true
  wait "$watchdog" 2>/dev/null || watched=$?
  if ((watched == 124)); then
    return 124
  fi
  return "$status"
}

# How long a simctl call that prepares or frees a simulator (create, delete, shutdown, uninstall, keychain reset, the
# keyboard's `defaults` spawn) may take, in seconds.
# shellcheck disable=SC2034 # for the sourcing scripts
SIMCTL_SECONDS=120

# `simctl $2...` for at most $1 seconds (bounded), macOS only.
simctl_bounded() {
  local limit="$1" simctl
  shift
  simctl="$(xcrun -f simctl)" || return 1
  if [[ -z "$simctl" || ! -x "$simctl" ]]; then
    echo "xcrun -f simctl gave no simctl to run: '$simctl'" >&2
    return 1
  fi
  bounded "$limit" "$simctl" "$@"
}

# Takes from the simulator named $1 (on the pinned iOS runtime) what its keyboard kept of a hardware keyboard, before
# its boot. A hardware key event (XCUITest's typeKey) makes the system write AutomaticMinimizationEnabled and
# HardwareKeyboardLastSeen. From the next boot on, AutomaticMinimizationEnabled keeps each app's soft keyboard below
# the screen, as for an attached hardware keyboard, so a soft keyboard test that ran first after that boot found no key
# on the screen (task 706). The system reads it as it boots: a write after the boot comes too late. So both keys are
# removed from the shut-down simulator's file, as a new simulator has neither, and a booted simulator is shut down
# first. A simulator that doesn't exist yet, or never booted, has no such file.
ios_sim_forget_hardware_keyboard() {
  local devices udid state data prefs key
  devices="$(simctl_bounded "$SIMCTL_SECONDS" list -j devices)" || return 1
  IFS=$'\t' read -r udid state data < <(python3 -I -c '
import json, sys
runtime = "com.apple.CoreSimulator.SimRuntime.iOS-" + sys.argv[2].replace(".", "-")
for device in json.load(sys.stdin).get("devices", {}).get(runtime, []):
    if device.get("name") == sys.argv[1]:
        print(device["udid"] + "\t" + device.get("state", "") + "\t" + device.get("dataPath", ""))
        break' "$1" "$IOS_SIMULATOR_RUNTIME" <<<"$devices") || true
  [[ -n "${udid:-}" ]] || return 0
  if [[ "$state" != Shutdown ]]; then
    simctl_bounded "$SIMCTL_SECONDS" shutdown "$udid" || return 1
  fi
  prefs="$data/Library/Preferences/com.apple.keyboard.preferences.plist"
  [[ -f "$prefs" ]] || return 0
  for key in AutomaticMinimizationEnabled HardwareKeyboardLastSeen; do
    if plutil -extract "$key" raw "$prefs" >/dev/null 2>&1; then
      plutil -remove "$key" "$prefs" || return 1
    fi
  done
}

# Tells the keyboard of the booted simulator $1 that its slide-to-type introduction was shown. A new simulator shows
# it the first time a keyboard that can slide to type comes up, over the soft keyboard's lower half and the extra
# keys, and it stays until its Continue is tapped: the taps the UI tests make there land on it, and the keys type
# nothing (KeyboardTests). The system writes the same preference once it has shown the introduction.
ios_sim_quiet_keyboard_introduction() {
  simctl_bounded "$SIMCTL_SECONDS" spawn "$1" defaults write com.apple.keyboard.preferences \
    DidShowContinuousPathIntroduction -bool true
}

# The iOS build lock: a per-user flock(2) file, so one xcodebuild at a time runs on a Mac (build-ios.sh's header says
# why). BUILD_IOS_LOCK_HELD names it in the environment of a run that holds it.
IOS_BUILD_LOCK="${XDG_CACHE_HOME:-$HOME/.cache}/ngterm/build-ios.flock"

# Whether an ancestor of this shell is lockf(1) of the lock file $1 (`lockf [-kns] [-t <seconds>] <file> <command>`):
# lockf starts its command only once it holds the lock, so the lock is this shell's already.
lock_held_by_ancestor() {
  local lock="$1" pid="$PPID" depth=0 command word state
  while ((pid > 1 && depth < 64)); do
    command="$(ps -o command= -p "$pid" 2>/dev/null)" || return 1
    state=""
    for word in $command; do
      case "$state" in
        "") [[ "${word##*/}" != lockf ]] || state=options ;;
        timeout) state=options ;;
        options)
          case "$word" in
            -t) state=timeout ;;
            -*) ;;
            *)
              [[ "$word" != "$lock" ]] || return 0
              state=""
              ;;
          esac
          ;;
      esac
    done
    pid="$(ps -o ppid= -p "$pid" 2>/dev/null | tr -d ' ')" || return 1
    [[ -n "$pid" ]] || return 1
    depth=$((depth + 1))
  done
  return 1
}

# Takes the iOS build lock for the script $2 (named $1 in messages), run again under lockf(1) with the arguments $3...,
# unless it is held already: BUILD_IOS_LOCK_HELD names it, or an ancestor lockf holds it (an outer `lockf -k <lock>
# <script>` whose child would else wait for its own parent for ever). Returns only in the run that holds the lock, with
# BUILD_IOS_LOCK_HELD exported.
take_ios_build_lock() {
  local name="$1" script="$2"
  shift 2
  if [[ "${BUILD_IOS_LOCK_HELD:-}" != "$IOS_BUILD_LOCK" ]]; then
    if lock_held_by_ancestor "$IOS_BUILD_LOCK"; then
      echo "$name: an outer lockf holds the build lock ($IOS_BUILD_LOCK): this run goes on under its hold" >&2
    else
      mkdir -p "$(dirname "$IOS_BUILD_LOCK")"
      lockf -ks -t 0 "$IOS_BUILD_LOCK" true || echo "$name: waiting for the build lock ($IOS_BUILD_LOCK)" >&2
      BUILD_IOS_LOCK_HELD="$IOS_BUILD_LOCK" exec lockf -k "$IOS_BUILD_LOCK" "$BASH" "$script" "$@"
    fi
  fi
  export BUILD_IOS_LOCK_HELD="$IOS_BUILD_LOCK"
}

# The host OS as Zig and ziglang.org's archives name it: linux, macos or windows (Git Bash, MSYS2, Cygwin).
host_os() {
  case "$(uname -s)" in
    Linux) echo linux ;;
    Darwin) echo macos ;;
    MINGW* | MSYS* | CYGWIN*) echo windows ;;
    *) die "no native build for $(uname -s)" ;;
  esac
}

# PATH in the form a native program of this host takes: on Windows the mixed form (`C:/Users/...`, which both
# Windows programs and MSYS read), elsewhere PATH itself.
native_path() {
  if [[ "$(host_os)" == windows ]]; then
    cygpath -m "$1"
  else
    printf '%s\n' "$1"
  fi
}

NATIVE_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PINS="$NATIVE_ROOT/build/pins.env"
VENDOR_ROOT="${NGTERM_VENDOR:-$HOME/.cache/ngterm/vendor}"
# a Windows-form path (`C:\...`, `C:/...`: what cargo and the tests pass on Windows) in MSYS's form
if [[ "$VENDOR_ROOT" == [A-Za-z]:[\\/]* ]] && command -v cygpath >/dev/null; then
  VENDOR_ROOT="$(cygpath -u "$VENDOR_ROOT")"
fi
[[ "$VENDOR_ROOT" == /* ]] || VENDOR_ROOT="$NATIVE_ROOT/$VENDOR_ROOT"
# shellcheck source=../../build/pins.env
source "$PINS"

# The workspace minus everything that links WPE WebKit (the -sys crates, the ngterm-wpe host over them, and the GPUI
# frontend, whose default `html` feature runs its HTML overlays on it), as cargo arguments, so a machine without WPE
# WebKit (Ubuntu packages none) still builds and tests the rest; GPUI_WITHOUT_HTML adds the GPUI frontend back without
# the feature (the core then refuses every page). The app with its pages, and its conformance, need a WPE WebKit
# (docs/development.md > WPE WebKit): run-linux.sh and run-gpui-dev.sh build it so, and CI runs them in its Arch jobs.
# shellcheck disable=SC2034 # for the sourcing scripts
WORKSPACE_WITHOUT_WPE=(--workspace --exclude ngterm-wpe --exclude ngterm-wpe-client --exclude ngterm-wpe-sys --exclude ngterm-wpe-platform-sys
  --exclude ngterm-wpe-javascriptcore-sys --exclude ngterm-gpui)
# shellcheck disable=SC2034 # for the sourcing scripts
# Linux frontend builds stage the separately linked HTML worker beside the app.
WEB_WORKER_BUILD=()
if [[ "$(uname -s)" == Linux ]]; then
  # shellcheck disable=SC2034 # for dev-instance.sh
  WEB_WORKER_BUILD=(-p ngterm-wpe --bin ngterm --bin ngterm-web)
fi

# shellcheck disable=SC2034 # for the sourcing scripts
GPUI_WITHOUT_HTML=(-p ngterm-gpui --no-default-features)

# The sha256 of stdin: coreutils' sha256sum (Linux, Git Bash), else shasum (stock macOS).
sha256_of() {
  if command -v sha256sum >/dev/null; then
    sha256sum | cut -d' ' -f1
  else
    shasum -a 256 | cut -d' ' -f1
  fi
}

# FILE's size in bytes and modification time in nanoseconds since the Unix epoch, as `<size> <mtime-ns>`: GNU stat
# (Linux, Git Bash), else BSD stat (stock macOS). The GNU form takes the whole seconds (%Y) and the nine digits of
# the readable time (%y), not `%.9Y`: the uutils stat (Ubuntu 26.04's) passes that one through a float and loses
# the last digits. Its %Y goes through the same float, which rounds a fraction just under a whole second up into the
# seconds, so the seconds of the readable time (in UTC, where they are the epoch's own) take that one back. In the C
# locale, whose decimal point is the dot (a comma-decimal locale's stat writes a comma).
size_and_mtime_ns() {
  local stat size seconds time fraction carried
  if stat --version >/dev/null 2>&1; then
    # `<size> <seconds> <date> <hours>:<minutes>:<seconds>.<nine digits> <zone>`
    stat="$(LC_ALL=C TZ=UTC stat -c '%s %Y %y' "$1")" || return 1
    read -r size seconds _ time _ <<<"$stat"
    [[ "$seconds" =~ ^-?[0-9]+$ && "$time" =~ ^[0-9]{2}:[0-9]{2}:([0-9]{2})\.([0-9]{9})$ ]] || return 1
    fraction="${BASH_REMATCH[2]}"
    # how far %Y is past the readable time's seconds: nothing, or the one second a float rounded up
    carried=$((((seconds - 10#${BASH_REMATCH[1]}) % 60 + 60) % 60))
    ((carried <= 1)) || return 1
    seconds=$((seconds - carried))
    stat="$size $seconds.$fraction"
  else
    stat="$(LC_ALL=C stat -f '%z %.9Fm' "$1")" || return 1
  fi
  # `<seconds>.<nine digits>` as one number of nanoseconds
  printf '%s\n' "$stat" | sed -e 's/\.//' -e 's/ 0*\([0-9]\)/ \1/'
}

# A fingerprint of the files and links below the directories given: the sha256 of the path, size and modification time
# of each one, in path order. A file that is written again gets a new time, so a change of content changes the
# fingerprint, but the cost is one stat for each file, not a read of each file.
# It fails when a directory is not there.
files_fingerprint() {
  local paths path line text=""
  paths="$(find "$@" \( -type f -o -type l \) -print)" || return 1
  paths="$(LC_ALL=C sort <<<"$paths")"
  while IFS= read -r path; do
    [[ -n "$path" ]] || continue
    line="$(size_and_mtime_ns "$path")" || return 1
    text="$text$line $path"$'\n'
  done <<<"$paths"
  printf '%s' "$text" | sha256_of
}

# Make directory $2 hold what directory $1 holds, rewriting only the files whose content differs. No times are
# copied, so an unchanged file keeps its own time, and a changed file gets a new one (files_fingerprint sees it).
sync_dir() {
  mkdir -p "$2"
  rsync -r --checksum --delete "$1/" "$2/"
}

# The record of a test-plan build (build-ios.sh's build_test_plan): plan $1, tree stamp $2, profile $3, Xcode $4 and
# the fingerprint of the Generated/ directories $5.... Empty when there is no tree stamp or the fingerprint fails:
# then no record matches, and the plan builds.
test_build_record() {
  local plan="$1" tree="$2" profile="$3" xcode="$4" generated
  shift 4
  [[ -n "$tree" ]] || return 0
  generated="$(files_fingerprint "$@")" || return 0
  printf '%s\n' "$plan $tree $profile $xcode $generated"
}

# Whether FILE's sha256 is SHA.
sha256_matches() {
  local sha="$1" file="$2" actual
  actual="$(sha256_of <"$file")" || return 1
  [[ "$actual" == "$sha" ]]
}

# STRING in upper case (bash 3.2 has no `${var^^}`).
upper() {
  printf '%s\n' "$1" | tr '[:lower:]' '[:upper:]'
}

# The pin lines of one dependency (every line starting with PREFIX_), as recorded in its prefix.
pin_lines() {
  grep -E "^$1_" "$PINS"
}

# A key over the pin lines of one dependency, the patches of one or more directories (top level of each), the host
# arch and the directory the prefix keeps its manifests in (share/ngterm, upstream's and earlier builds' share/agterm),
# so a prefix of the other layout, which another checkout may still use from the same vendor root, is never taken for
# this one's. A patch of the first directory is named by its file name alone, one of a later directory by
# `<directory name>/<file name>`. A leading `--layout N` adds the layout of what the prefix holds, so a checkout
# expecting another layout (other branches share the vendor root) never takes the prefix for its own.
prefix_key() {
  local layout="" pins_prefix first patches patch label
  if [[ "$1" == --layout ]]; then
    layout="$2"
    shift 2
  fi
  pins_prefix="$1"
  first="$2"
  shift
  {
    echo "arch $(uname -m)"
    echo "manifests share/ngterm"
    [[ -z "$layout" ]] || echo "layout $layout"
    pin_lines "$pins_prefix"
    for patches in "$@"; do
      label=""
      [[ "$patches" == "$first" ]] || label="$(basename "$patches")/"
      for patch in "$patches"/*.patch; do
        [[ -f "$patch" ]] || continue
        echo "patch $label$(basename "$patch")"
        cat "$patch"
      done
    done
  } | sha256_of | cut -c1-16
}

# libghostty-vt's key hashes the LIBGHOSTTY_VT_ pins, build/patches/ghostty-vt/ and the prefix's layout: 2 is the
# patched source tree ngterm-ghostty-vt-sys/build.rs builds (the prebuilt library before it had no layout line).
GHOSTTY_VT_PREFIX_LAYOUT=2
ghostty_vt_prefix() {
  echo "$VENDOR_ROOT/ghostty-vt-$(prefix_key --layout "$GHOSTTY_VT_PREFIX_LAYOUT" LIBGHOSTTY_VT \
    "$NATIVE_ROOT/build/patches/ghostty-vt")"
}

# Zig's global cache of the libghostty-vt builds, one per Zig version, in the vendor root: never the user's own, and
# shared by the script's package fetches and every cargo build (Zig's cache is safe for concurrent processes).
ghostty_vt_zig_cache() {
  echo "$VENDOR_ROOT/zig-cache-$LIBGHOSTTY_VT_ZIG_VERSION"
}

# The value of NAME in the stamp of the libghostty-vt prefix PREFIX (share/ngterm/ghostty-vt-STAMP, `name=value`).
ghostty_vt_stamp_value() {
  sed -n "s/^$2=//p" "$1/share/ngterm/ghostty-vt-STAMP" 2>/dev/null | head -n1
}

# A libghostty-vt prefix is complete when the patched tree with its prefetched packages, the headers, the pin manifest
# and the stamp are there, the manifest matches the LIBGHOSTTY_VT_ pins, the stamp's zig (the one build.rs builds
# with) still runs as the pinned version, and Ghostty's resources verify.
ghostty_vt_prefix_is_complete() {
  local prefix="$1" zig
  [[ -s "$prefix/src/build.zig" && -d "$prefix/src/zig-pkg" && -s "$prefix/include/ghostty/vt.h" \
    && -s "$prefix/include/ghostty/vt/terminal.h" && -s "$prefix/share/ngterm/ghostty-vt-PINS" \
    && -s "$prefix/share/ngterm/ghostty-vt-STAMP" ]] \
    && pin_lines LIBGHOSTTY_VT | cmp -s - "$prefix/share/ngterm/ghostty-vt-PINS" \
    && zig="$(ghostty_vt_stamp_value "$prefix" zig)" && [[ -n "$zig" && -x "$zig" ]] \
    && [[ "$("$zig" version 2>/dev/null)" == "$LIBGHOSTTY_VT_ZIG_VERSION" ]] \
    && verify_ghostty_resources "$prefix/share" >/dev/null 2>&1
}

# The WPE WebKit package carries no patches: its key hashes the WPE_ pins only.
wpe_prefix() {
  echo "$VENDOR_ROOT/wpe-$(prefix_key WPE /nonexistent)"
}

# A WPE WebKit prefix is complete when the library, the web process, the headers, the pkg-config file, the .gir files
# and the pin manifest are there and the manifest matches the WPE_ pins.
wpe_prefix_is_complete() {
  local prefix="$1"
  [[ -s "$prefix/lib/libWPEWebKit-2.0.so.1" && -x "$prefix/lib/wpe-webkit-2.0/WPEWebProcess" \
    && -s "$prefix/include/wpe-webkit-2.0/wpe/webkit.h" && -s "$prefix/lib/pkgconfig/wpe-webkit-2.0.pc" \
    && -s "$prefix/share/gir-1.0/WPEWebKit-2.0.gir" && -s "$prefix/share/ngterm/wpe-PINS" ]] \
    && pin_lines WPE | cmp -s - "$prefix/share/ngterm/wpe-PINS"
}

# The lavapipe package carries no patches: its key hashes the LAVAPIPE_ pins only.
lavapipe_prefix() {
  echo "$VENDOR_ROOT/lavapipe-$(prefix_key LAVAPIPE /nonexistent)"
}

# A lavapipe prefix is complete when the driver, its ICD manifest (at the prefix's top, naming the driver by absolute
# path) and the pin manifest are there and the manifest matches the LAVAPIPE_ pins.
lavapipe_prefix_is_complete() {
  local prefix="$1"
  [[ -s "$prefix/lib/libvulkan_lvp.so" && -s "$prefix/lvp_icd.json" && -s "$prefix/share/ngterm/lavapipe-PINS" ]] \
    && pin_lines LAVAPIPE | cmp -s - "$prefix/share/ngterm/lavapipe-PINS"
}

# weston carries no patches: its key hashes the WESTON_ pins only.
weston_prefix() {
  echo "$VENDOR_ROOT/weston-$(prefix_key WESTON /nonexistent)"
}

# Whether the weston at PATH takes --fake-seat (weston 15.0 and later): the seat the GPUI build needs. The help is
# read whole first: piped into `grep -q` under pipefail, weston dies of SIGPIPE once grep has matched and the check
# fails for a weston that has the option.
weston_has_fake_seat() {
  grep -q -- '--fake-seat' <<<"$(env -u WAYLAND_DISPLAY -u DISPLAY "$1" --help 2>/dev/null || true)"
}

# A weston prefix is complete when the compositor, its headless backend, GL renderer and desktop shell (module and
# helper client) are there, the pin manifest matches the WESTON_ pins, and the compositor runs and takes --fake-seat.
weston_prefix_is_complete() {
  local prefix="$1"
  [[ -x "$prefix/bin/weston" && -x "$prefix/libexec/weston-desktop-shell" && -s "$prefix/share/ngterm/weston-PINS" ]] \
    && compgen -G "$prefix/lib/libweston-*/headless-backend.so" >/dev/null \
    && compgen -G "$prefix/lib/libweston-*/gl-renderer.so" >/dev/null \
    && [[ -s "$prefix/lib/weston/desktop-shell.so" ]] \
    && pin_lines WESTON | cmp -s - "$prefix/share/ngterm/weston-PINS" \
    && weston_has_fake_seat "$prefix/bin/weston"
}

# The libraries a shared object needs that this host cannot resolve, space-separated (empty when all resolve).
unresolved_libraries() {
  ldd "$1" 2>&1 | awk '/not found/ {print $1}' | sort -u | tr '\n' ' ' | sed 's/ $//'
}

# The terminal fonts carry no patches: their key hashes the FONTS_ pins only.
fonts_prefix() {
  echo "$VENDOR_ROOT/fonts-$(prefix_key FONTS /nonexistent)"
}

# The faces ngterm-gpui embeds, under the prefix's share/fonts/ (its build script reads the same list).
FONT_FILES=(JetBrainsMono-Regular.ttf JetBrainsMono-Bold.ttf JetBrainsMono-Italic.ttf JetBrainsMono-BoldItalic.ttf
  SymbolsNerdFontMono-Regular.ttf)

# A fonts prefix is complete when every face, both licenses and the pin manifest are there and the manifest matches
# the FONTS_ pins.
fonts_prefix_is_complete() {
  local prefix="$1" file
  for file in "${FONT_FILES[@]}"; do
    [[ -s "$prefix/share/fonts/$file" ]] || return 1
  done
  [[ -s "$prefix/share/doc/fonts/JetBrainsMono-OFL.txt" && -s "$prefix/share/doc/fonts/SymbolsNerdFont-LICENSE" \
    && -s "$prefix/share/ngterm/fonts-PINS" ]] \
    && pin_lines FONTS | cmp -s - "$prefix/share/ngterm/fonts-PINS"
}

# The gir binary's key hashes the GIR_ pins and build/patches/gir/.
gir_prefix() {
  echo "$VENDOR_ROOT/gir-$(prefix_key GIR "$NATIVE_ROOT/build/patches/gir")"
}

# A gir prefix is complete when the binary runs and its pin manifest matches the GIR_ pins.
gir_prefix_is_complete() {
  local prefix="$1"
  [[ -x "$prefix/bin/gir" && -s "$prefix/share/ngterm/gir-PINS" ]] \
    && pin_lines GIR | cmp -s - "$prefix/share/ngterm/gir-PINS"
}

# The Zig triple arch of this host, as ziglang.org's archives and the pinned sha256 variables name it.
host_zig_arch() {
  case "$(uname -m)" in
    x86_64 | amd64) echo x86_64 ;;
    aarch64 | arm64) echo aarch64 ;;
    *) die "no pinned Zig for $(uname -m)" ;;
  esac
}

# The pinned sha256 of the Zig archive of BUILD (a pin prefix: LIBGHOSTTY_VT) for this host:
# BUILD_ZIG_SHA256_<OS>_<ARCH>.
zig_pin_sha256() {
  local build="$1" os arch name
  os="$(upper "$(host_os)")" || exit
  arch="$(upper "$(host_zig_arch)")" || exit
  name="${build}_ZIG_SHA256_${os}_${arch}"
  [[ -n "${!name:-}" ]] || die "build/pins.env pins no Zig for $build on $(host_os) $(host_zig_arch) ($name)"
  echo "${!name}"
}

# Print a zig of exactly VERSION: $OVERRIDE_VAR, a matching zig on PATH, or the pinned ziglang.org archive
# of this OS and arch (sha256 SHA) unpacked once into the vendor root: a .tar.xz on Linux and macOS (tar with xz), a
# .zip on Windows (Git Bash's unzip).
#
# Callers run this as ZIG="$(find_zig ...)", where bash clears errexit, so every step checks its own failure: a
# failed fetch or unpack must never leave a partial tree in the final directory, which later runs would reuse.
find_zig() {
  local version="$1" sha="$2" override="$3" candidate os arch name exe format dir archive
  for candidate in "$override" "$(command -v zig 2>/dev/null || true)"; do
    if [[ -n "$candidate" && -x "$candidate" && "$("$candidate" version 2>/dev/null)" == "$version" ]]; then
      echo "$candidate"
      return
    fi
  done
  [[ -z "$override" ]] || die "$override is not zig $version"
  os="$(host_os)" || exit
  arch="$(host_zig_arch)" || exit
  name="zig-$arch-$os-$version"
  exe=zig
  format=tar.xz
  if [[ "$os" == windows ]]; then
    exe=zig.exe
    format=zip
  fi
  dir="$VENDOR_ROOT/$name"
  if [[ ! -x "$dir/$exe" ]]; then
    command -v curl >/dev/null || die "curl is required to fetch zig $version"
    if [[ "$format" == zip ]]; then
      command -v unzip >/dev/null || die "unzip is required to unpack zig $version (Git Bash ships it)"
    elif ! command -v xz >/dev/null && ! tar --version 2>/dev/null | grep -q bsdtar; then
      # GNU tar runs xz; macOS's bsdtar unpacks .xz itself
      die "xz is required to unpack zig $version"
    fi
    archive="$(mktemp "$VENDOR_ROOT/.zig-$version.XXXXXX")" || die "cannot create a download file in $VENDOR_ROOT"
    echo "fetching zig $version..." >&2
    curl -fsSLo "$archive" "https://ziglang.org/download/$version/$name.$format" \
      || { rm -f "$archive"; die "cannot download zig $version"; }
    sha256_matches "$sha" "$archive" || { rm -f "$archive"; die "zig $version sha256 mismatch"; }
    { rm -rf "$dir.partial" "$dir.unpacked" && mkdir -p "$dir.unpacked"; } \
      || { rm -f "$archive"; die "cannot create $dir.unpacked"; }
    if [[ "$format" == zip ]]; then
      # the zip's one top directory, $name/, becomes the partial tree (unzip strips no components)
      if ! { unzip -q "$archive" -d "$dir.unpacked" && mv "$dir.unpacked/$name" "$dir.partial"; }; then
        rm -rf "$dir.unpacked" "$dir.partial" "$archive"
        die "cannot unpack zig $version"
      fi
      rm -rf "$dir.unpacked"
    else
      if ! { tar -xJf "$archive" -C "$dir.unpacked" --strip-components=1 && mv "$dir.unpacked" "$dir.partial"; }; then
        rm -rf "$dir.unpacked" "$dir.partial" "$archive"
        die "cannot unpack zig $version"
      fi
    fi
    rm -f "$archive"
    { rm -rf "$dir" && mv "$dir.partial" "$dir"; } || { rm -rf "$dir.partial"; die "cannot move zig into $dir"; }
  fi
  [[ "$("$dir/$exe" version)" == "$version" ]] || die "$dir/$exe is not zig $version"
  echo "$dir/$exe"
}

# The compiled xterm-ghostty entry of the terminfo database DIR, in either layout `tic` writes: a directory per first
# letter (`x/`, ncurses on Linux) or per its hex code (`78/`, where the file system ignores case: macOS, Windows). Fails
# when neither is there.
ghostty_terminfo_entry() {
  local entry
  for entry in "$1/x/xterm-ghostty" "$1/78/xterm-ghostty"; do
    if [[ -s "$entry" ]]; then
      echo "$entry"
      return 0
    fi
  done
  return 1
}

# Verify the Ghostty resources below SHARE: the ones
# build-ghostty-vt.sh vendors at LIBGHOSTTY_VT_REV. The terminfo source (`ghostty.terminfo`) is there on every OS; the
# compiled entry is required, and read back with infocmp, where `tic` exists (build-ghostty-vt.sh compiles it then).
verify_ghostty_resources() {
  local share="$1" ghostty themes terminfo theme_count theme file
  ghostty="$share/ghostty"
  themes="$ghostty/themes"
  terminfo="$share/terminfo"
  for file in "$ghostty/shell-integration/bash/ghostty.bash" "$ghostty/shell-integration/zsh/ghostty-integration" \
    "$themes/.ngterm-resource-manifest" "$terminfo/ghostty.terminfo"; do
    [[ -s "$file" ]] || { echo "missing or empty Ghostty resource: $file" >&2; return 1; }
  done
  if command -v tic >/dev/null; then
    command -v infocmp >/dev/null || { echo "infocmp is required to verify Ghostty resources" >&2; return 1; }
    ghostty_terminfo_entry "$terminfo" >/dev/null || {
      echo "missing compiled Ghostty terminfo: neither $terminfo/x/xterm-ghostty nor $terminfo/78/xterm-ghostty" >&2
      return 1
    }
  fi
  pin_lines LIBGHOSTTY_VT | cmp -s - "$themes/.ngterm-resource-manifest" || {
    echo "Ghostty resource provenance does not match the LIBGHOSTTY_VT_ pins in $PINS" >&2
    return 1
  }
  theme_count="$(find "$themes" -maxdepth 1 -type f ! -name '.ngterm-resource-manifest' | wc -l)"
  if ((theme_count != LIBGHOSTTY_VT_THEME_COUNT)); then
    echo "Ghostty theme catalog has $theme_count files; expected $LIBGHOSTTY_VT_THEME_COUNT" >&2
    return 1
  fi
  for theme in "${LIBGHOSTTY_VT_KNOWN_THEMES[@]}"; do
    [[ -s "$themes/$theme" ]] || { echo "missing or empty Ghostty theme: $themes/$theme" >&2; return 1; }
  done
  if command -v tic >/dev/null; then
    infocmp -A "$terminfo" xterm-ghostty >/dev/null || {
      echo "xterm-ghostty is not readable from $terminfo" >&2
      return 1
    }
  fi
  echo "→ verified Ghostty resources from $LIBGHOSTTY_VT_REV ($theme_count themes + xterm-ghostty)"
}
