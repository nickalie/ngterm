#!/usr/bin/env bash
# Sign a release's SHA256SUMS with minisign: SHA256SUMS.minisig beside it, whose trusted comment is the one the
# updater requires (crates/ngterm-update: `ngterm v<VERSION> SHA256SUMS`), then verify the signature and that comment.
#
#   scripts/sign-sums.sh SUMS VERSION
#       the key from MINISIGN_SECRET_KEY (the `release` environment's secret, a key made with `minisign -G -W`),
#       verified against the committed crates/ngterm-update/trust/minisign.pub when that is non-empty, so a secret
#       that doesn't match what installed builds trust fails here, not in the updater; otherwise against the public
#       key recreated from the secret one
#   scripts/sign-sums.sh --throwaway SUMS VERSION
#       a key made for this run and verified against its own public key (dry runs, which hold no secrets)
#
# The key is written to a 0600 file in a private temporary directory removed on exit; it never reaches a command
# line or the log.
set -euo pipefail

die() { echo "sign-sums.sh: $*" >&2; exit 1; }
usage() { die "usage: scripts/sign-sums.sh [--throwaway] SUMS VERSION"; }

throwaway=false
[[ "${1:-}" != --throwaway ]] || { throwaway=true; shift; }
(($# == 2)) || usage
sums="$1"
version="$2"
[[ -s "$sums" ]] || die "$sums is empty or missing"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "version '$version' is not MAJOR.MINOR.PATCH"
command -v minisign >/dev/null || die "no minisign"
trusted="$(cd "$(dirname "$0")/.." && pwd)/crates/ngterm-update/trust/minisign.pub"
comment="ngterm v$version SHA256SUMS"

work="$(umask 077 && mktemp -d)"
trap 'rm -rf "$work"' EXIT
if [[ "$throwaway" == true ]]; then
  minisign -G -W -p "$work/minisign.pub" -s "$work/minisign.key" >/dev/null
  public="$work/minisign.pub"
else
  [[ -n "${MINISIGN_SECRET_KEY:-}" ]] || die "MINISIGN_SECRET_KEY is not set"
  (umask 077 && printf '%s\n' "$MINISIGN_SECRET_KEY" >"$work/minisign.key")
  if [[ -n "$(tr -d '[:space:]' <"$trusted" 2>/dev/null)" ]]; then
    public="$trusted"
  else
    minisign -R -s "$work/minisign.key" -p "$work/minisign.pub" >/dev/null
    public="$work/minisign.pub"
  fi
fi

minisign -S -s "$work/minisign.key" -m "$sums" -x "$sums.minisig" -t "$comment" >/dev/null
rm -f "$work/minisign.key"
signed="$(minisign -V -Q -p "$public" -m "$sums" -x "$sums.minisig")" ||
  die "the signature doesn't verify against $(basename "$public"): MINISIGN_SECRET_KEY isn't the key installed builds trust"
[[ "$signed" == "$comment" ]] || die "the trusted comment is '$signed', not '$comment'"
echo "$(basename "$sums").minisig: signed and verified ($comment)"
