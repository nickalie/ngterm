# shellcheck shell=bash
# A size bound on the wallpaper cache of the pinned iOS simulators, for scripts/build-ios.sh. Sourced after
# lib/native.sh, not run. Portable shell (stock macOS's bash 3.2, Linux's bash), as lib/native.sh is.
#
# PosterBoard on the iOS 27 simulator writes a snapshot of each system wallpaper at each boot, up to about 350 MB, into
# SnapshotCache.cachedb dirs below the device's PRBPosterExtensionDataStore, and never removes the old ones. Many
# short test runs (many boots) thus filled a shared Mac's disk: about 46 GB on the two pinned simulators in 3 days.
# PosterBoard makes these caches again when they are gone.
#
# The bound applies only to a simulator that simctl names as shut down, with the pinned name on the pinned runtime,
# and only to the SnapshotCache.cachedb dirs below its PRBPosterExtensionDataStore. Each dir is renamed (one atomic
# mv, in its own parent) and then deleted: a boot that starts during the delete finds no cache, never a half-deleted
# one. A renamed dir that an earlier delete did not complete is deleted too. The caller holds the iOS build lock.

# The name of a cache dir after its rename, in the same parent dir.
IOS_SIM_CACHE_REMOVED=SnapshotCache.cachedb.ngterm-removed

# The wallpaper store of a simulator whose data dir is $1.
ios_sim_cache_store() {
  printf '%s\n' "$1/Library/Application Support/PRBPosterExtensionDataStore"
}

# The cache dirs (and the renamed dirs that an earlier delete left) below the store $1, one per line. Prints nothing
# when the store is missing or a symlink.
ios_sim_cache_dirs() {
  local store="$1"
  [[ -d "$store" && ! -L "$store" ]] || return 0
  find "$store" -type d \( -name SnapshotCache.cachedb -o -name "$IOS_SIM_CACHE_REMOVED" \) -prune -print
}

# The size in KiB of the dirs on stdin, one per line (0 for none).
ios_sim_cache_kib() {
  local dir total=0 kib
  while IFS= read -r dir; do
    [[ -n "$dir" ]] || continue
    kib="$(du -sk "$dir" 2>/dev/null | awk '{ print $1 }')" || kib=0
    total=$((total + ${kib:-0}))
  done
  echo "$total"
}

# Keep the wallpaper cache of the simulator with the data dir $2 (UDID $1, simctl state $3) at or below $4 MiB.
ios_sim_cache_bound_device() {
  local udid="$1" data="$2" state="$3" bound_mib="$4" store dirs dir parent kib after count=0
  store="$(ios_sim_cache_store "$data")"
  if [[ "$state" != Shutdown ]]; then
    echo "simulator $udid is $state: its wallpaper cache stays as it is" >&2
    return 0
  fi
  dirs="$(ios_sim_cache_dirs "$store")"
  kib="$(ios_sim_cache_kib <<<"$dirs")"
  if ((kib <= bound_mib * 1024)); then
    echo "simulator $udid: wallpaper cache $((kib / 1024)) MiB, bound $bound_mib MiB" >&2
    return 0
  fi
  while IFS= read -r dir; do
    [[ "$store" == /* && "$dir" == "$store"/* ]] || continue
    parent="${dir%/*}"
    if [[ "${dir##*/}" == SnapshotCache.cachedb ]]; then
      rm -rf "${parent:?}/$IOS_SIM_CACHE_REMOVED"
      mv "$dir" "${parent:?}/$IOS_SIM_CACHE_REMOVED" || continue
    fi
    rm -rf "${parent:?}/$IOS_SIM_CACHE_REMOVED"
    count=$((count + 1))
  done <<<"$dirs"
  after="$(ios_sim_cache_dirs "$store" | ios_sim_cache_kib)"
  echo "simulator $udid: wallpaper cache $((kib / 1024)) MiB, over the bound of $bound_mib MiB:" \
    "removed $count dirs, $((after / 1024)) MiB left" >&2
}

# Keep the wallpaper cache of each shut-down simulator named $1 on the runtime $2 (a simctl runtime identifier) at
# or below $3 MiB (1 to 9 decimal digits; a leading zero does not make it octal), from `simctl list -j devices` on
# stdin. Only a malformed bound or listing fails it.
ios_sim_cache_bound() {
  local name="$1" runtime="$2" bound_mib="$3" devices udid state data
  [[ "$bound_mib" =~ ^[0-9]{1,9}$ ]] || {
    echo "the wallpaper cache bound '$bound_mib' is not a number of MiB" >&2
    return 1
  }
  bound_mib=$((10#$bound_mib))
  devices="$(IOS_SIM_NAME="$name" IOS_SIM_RUNTIME="$runtime" python3 -c "$IOS_SIM_DEVICES_PY")" ||
    return 1
  while IFS=$'\t' read -r udid state data; do
    [[ -n "$udid" ]] || continue
    ios_sim_cache_bound_device "$udid" "$data" "$state" "$bound_mib"
  done <<<"$devices"
}
