#!/usr/bin/env bash
# The store of release.yml's portable WPE WebKit prefix: one OCI artifact per runner image and build key in the public
# GHCR package ghcr.io/<owner>/ngterm-release-wpe, tagged `<runner os label>-<build-release-wpe.sh --print-key>`.
# release-wpe-cache.yml builds and publishes it on main; release.yml's `linux` job pulls it.
#
# - A tag is written once: `push` skips one that exists, `tag` refuses one, and nothing re-attests or repairs it.
# - The manifest goes up untagged and gets its tag only after its provenance is attested, so a tag never names an
#   unattested prefix.
# - `pull` takes the tag's digest once, then verifies, downloads and unpacks by that digest alone.
# - The registry's "not found" for the tag is a miss. Any other failure to read it is a miss with a warning for
#   `probe`, `pull` and `push`'s pre-check (so a package not created yet bootstraps, and a release never fails on the
#   store alone), and fails `tag`, which runs after the push. Once a digest is resolved, `pull` fails on anything that doesn't verify; it never falls
#   back to a source build then.
#
# ORAS is the version packaging/release/pins.env pins, fetched into a temporary directory per call. It keeps the
# login in the Docker credential config (~/.docker/config.json), where gh and the attest action read it too. `login`
# reads the token from GH_TOKEN; `probe` and `pull` log in and out themselves.
#
# Usage: scripts/release-wpe-store.sh ref OS                    print OS's reference in the store
#        scripts/release-wpe-store.sh probe OS                  print exists=true|false
#        scripts/release-wpe-store.sh pack ARCHIVE              tar NGTERM_RELEASE_WPE into ARCHIVE, print its sha256
#        scripts/release-wpe-store.sh login | logout
#        scripts/release-wpe-store.sh push OS ARCHIVE SHA256    push ARCHIVE untagged, print name= and digest=
#                                                               (digest= empty when OS's tag exists already)
#        scripts/release-wpe-store.sh tag OS DIGEST             give the pushed manifest OS's tag
#        scripts/release-wpe-store.sh pull OS                   unpack OS's verified prefix into NGTERM_RELEASE_WPE
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"
# shellcheck source=lib/fetch-tool.sh
source "$(dirname "$0")/lib/fetch-tool.sh"
# shellcheck source=../packaging/release/pins.env
source "$NATIVE_ROOT/packaging/release/pins.env"

REGISTRY=ghcr.io
ARCHIVE_NAME=release-wpe.tar.zst
SIGNER_WORKFLOW=.github/workflows/release-wpe-cache.yml

usage() {
  die "usage: scripts/release-wpe-store.sh ref OS | probe OS | pack ARCHIVE | login | logout | push OS ARCHIVE SHA256" \
    "| tag OS DIGEST | pull OS"
}

repository() {
  local owner="${GITHUB_REPOSITORY_OWNER:?set GITHUB_REPOSITORY_OWNER}"
  echo "$REGISTRY/${owner,,}/ngterm-release-wpe"
}

build_key() {
  "$NATIVE_ROOT/scripts/build-release-wpe.sh" --print-key
}

# tag_of OS: the store's tag of OS's prefix at this checkout's build key
tag_of() {
  local key
  [[ "$1" =~ ^[a-z0-9][a-z0-9.-]*$ ]] || die "not a runner os label: $1"
  key="$(build_key)"
  echo "$1-$key"
}

# ref_of OS: the store's reference of OS's prefix
ref_of() {
  local name tag
  name="$(repository)"
  tag="$(tag_of "$1")"
  echo "$name:$tag"
}

prefix() {
  local prefix="${NGTERM_RELEASE_WPE:?set NGTERM_RELEASE_WPE to the absolute WPE prefix}"
  [[ "$prefix" == /* ]] || die "NGTERM_RELEASE_WPE must be absolute"
  echo "$prefix"
}

# is_prefix_of PREFIX KEY: whether PREFIX is a finished WPE build of KEY: its .release-pins, which build-release-wpe.sh
# writes last, holds KEY
is_prefix_of() {
  [[ -f "$1/.release-pins" && "$(cat "$1/.release-pins")" == "$2" ]]
}

work="$(mktemp -d)"
cleanup() {
  [[ "${logged_in:-}" != true ]] || "$oras" logout "$REGISTRY" >/dev/null || true
  rm -rf "$work"
}
trap cleanup EXIT

fetch_oras() {
  local arch sum url
  case "$(uname -m)" in
    x86_64) arch=amd64 sum="$ORAS_X86_64_SHA256" ;;
    aarch64 | arm64) arch=arm64 sum="$ORAS_AARCH64_SHA256" ;;
    *) die "no pinned ORAS for $(uname -m)" ;;
  esac
  url="https://github.com/oras-project/oras/releases/download/v$ORAS_VERSION/oras_${ORAS_VERSION}_linux_$arch.tar.gz"
  oras="$(fetch_tool "$work" oras "$ORAS_VERSION" "$url" "$sum" oras)/oras"
}

login() {
  "$oras" login "$REGISTRY" -u "${GITHUB_ACTOR:?set GITHUB_ACTOR}" --password-stdin \
    <<<"${GH_TOKEN:?set GH_TOKEN to a token that can read (or write) the package}" >/dev/null
}

# resolve REF: print REF's manifest digest, or nothing when the registry answers that REF's tag doesn't exist. With
# --or-unreadable, any other failure to resolve (the package not created yet, which GHCR may answer as denied; auth;
# the network) is a miss too, with a warning naming it: `probe` then builds (a push without write access still
# fails), and `pull` builds from source rather than failing the release. Without it, such a failure fails.
resolve() {
  local ref="$1" lenient="${2:-}" digest err
  if digest="$("$oras" resolve "$ref" 2>"$work/resolve.err")"; then
    [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || die "$ref: resolved to '$digest', not a sha256 digest"
    echo "$digest"
    return
  fi
  err="$(cat "$work/resolve.err")"
  [[ "$err" != "Error response from registry: failed to resolve digest: $ref: not found" ]] || return 0
  echo "$err" >&2
  [[ "$lenient" == --or-unreadable ]] || die "$ref: the registry neither has nor denies having this tag"
  echo "::warning::$ref: unreadable in the store, taken as missing: ${err//$'\n'/ }" >&2
}

probe() {
  local ref digest
  ref="$(ref_of "$1")"
  fetch_oras
  login
  logged_in=true
  digest="$(resolve "$ref" --or-unreadable)"
  echo "$ref: ${digest:-missing}" >&2
  if [[ -n "$digest" ]]; then echo exists=true; else echo exists=false; fi
}

pack() {
  local prefix key
  prefix="$(prefix)"
  key="$(build_key)"
  is_prefix_of "$prefix" "$key" || die "$prefix: not the WPE prefix of build key $key"
  tar --zstd -cf "$1" -C "$prefix" .
  sha256sum "$1" | cut -d' ' -f1
}

push() {
  local name ref archive="$2" sum="$3" existing digest
  name="$(repository)"
  ref="$(ref_of "$1")"
  [[ "$sum" =~ ^[0-9a-f]{64}$ ]] || die "not a sha256: $sum"
  echo "$sum  $archive" | sha256sum --check --quiet || die "$archive: not the archive the build job packed"
  fetch_oras
  echo "name=$name"
  # before the package's first push it may not exist yet; `tag` checks strictly after the push
  existing="$(resolve "$ref" --or-unreadable)"
  if [[ -n "$existing" ]]; then
    echo "::notice::$ref exists already ($existing): it stays as it is" >&2
    echo "digest="
    return
  fi
  cp "$archive" "$work/$ARCHIVE_NAME"
  digest="$(cd "$work" && "$oras" push "$name" "$ARCHIVE_NAME:application/vnd.oci.image.layer.v1.tar+zstd" \
    --artifact-type application/vnd.ngterm.release-wpe.v1 \
    --annotation "org.opencontainers.image.source=${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:?}" \
    --annotation "org.opencontainers.image.revision=${GITHUB_SHA:?}" \
    --format go-template='{{.digest}}')"
  [[ "$digest" =~ ^sha256:[0-9a-f]{64}$ ]] || die "$name: the push reported '$digest', not a sha256 digest"
  echo "$name@$digest: pushed untagged, for $ref" >&2
  echo "digest=$digest"
}

tag() {
  local name new_tag existing
  name="$(repository)"
  new_tag="$(tag_of "$1")"
  [[ "$2" =~ ^sha256:[0-9a-f]{64}$ ]] || die "not a sha256 digest: $2"
  fetch_oras
  existing="$(resolve "$name:$new_tag")"
  [[ -z "$existing" ]] || die "$name:$new_tag: appeared while this run pushed its own; tags are written once"
  "$oras" tag "$name@$2" "$new_tag"
}

pull() {
  local name ref digest prefix key
  name="$(repository)"
  ref="$(ref_of "$1")"
  prefix="$(prefix)"
  key="$(build_key)"
  fetch_oras
  # a login that fails is a store that can't be read: the release builds WPE from source
  if ! login; then
    echo "::warning::$REGISTRY: the login failed: WPE WebKit builds from source (hours)"
    return
  fi
  logged_in=true
  digest="$(resolve "$ref" --or-unreadable)"
  if [[ -z "$digest" ]]; then
    echo "::warning::$ref is not in the store: WPE WebKit builds from source (hours)"
    return
  fi
  echo "$ref: $digest"
  gh attestation verify "oci://$name@$digest" --repo "${GITHUB_REPOSITORY:?}" \
    --signer-workflow "$GITHUB_REPOSITORY/$SIGNER_WORKFLOW" --source-ref refs/heads/main
  "$oras" pull "$name@$digest" -o "$work/pulled"
  [[ "$(ls -A "$work/pulled")" == "$ARCHIVE_NAME" ]] || die "$name@$digest: holds more or other than $ARCHIVE_NAME"
  rm -rf "$prefix"
  mkdir -p "$prefix"
  tar --zstd -xf "$work/pulled/$ARCHIVE_NAME" -C "$prefix" --no-same-owner
  is_prefix_of "$prefix" "$key" || die "$name@$digest: not the WPE prefix of build key $key"
  echo "$prefix: unpacked from $name@$digest"
}

case "${1:-} $#" in
  "ref 2") ref_of "$2" ;;
  "probe 2") probe "$2" ;;
  "pack 2") pack "$2" ;;
  "login 1")
    fetch_oras
    login
    ;;
  "logout 1")
    fetch_oras
    "$oras" logout "$REGISTRY"
    ;;
  "push 4") push "$2" "$3" "$4" ;;
  "tag 3") tag "$2" "$3" ;;
  "pull 2") pull "$2" ;;
  *) usage ;;
esac
