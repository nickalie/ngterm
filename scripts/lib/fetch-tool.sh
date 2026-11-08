# shellcheck shell=bash
# fetch_tool: a pinned tool from its GitHub release archive, unpacked only after the archive's sha256 matches the pin
# (packaging/release/pins.env). Sourced after lib/native.sh by scripts/lint.sh (actionlint, shellcheck) and
# scripts/release-wpe-store.sh (ORAS). Not run.

# fetch_tool ROOT NAME VERSION URL SHA256 MEMBER: print the directory under ROOT holding NAME, the MEMBER of the release
# archive at URL, unpacking it there first when it is not there yet
fetch_tool() (
  local root="$1" name="$2" version="$3" url="$4" sum="$5" member="$6" dir work
  dir="$root/$name-$version-$(uname -m)"
  if [[ ! -x "$dir/$name" ]]; then
    work="$(mktemp -d)"
    trap 'rm -rf "$work"' EXIT
    curl --fail --silent --show-error --location --retry 5 "$url" -o "$work/archive"
    echo "$sum  $work/archive" | sha256sum --check --quiet || die "$name $version: the archive's sha256 does not match"
    mkdir -p "$work/unpacked" "$root"
    tar -xf "$work/archive" -C "$work/unpacked" "$member"
    rm -rf "$dir.partial"
    mkdir "$dir.partial"
    install -m 755 "$work/unpacked/$member" "$dir.partial/$name"
    rm -rf "$dir"
    mv "$dir.partial" "$dir"
  fi
  echo "$dir"
)
