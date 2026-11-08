#!/usr/bin/env bash
# Lint the GitHub workflows and the scripts, locally as in CI (.github/workflows/lint.yml):
#
# - actionlint over .github/workflows/ (settings: .github/actionlint.yaml), with the pinned shellcheck checking every
#   `run:` block;
# - shellcheck over scripts/*.sh and scripts/lib/*.sh (settings: .shellcheckrc);
# - the self-test of every scripts/*.py that has one (`--self-test`).
#
# Both are the versions packaging/release/pins.env pins: each release archive is downloaded once into the vendor root
# and unpacked only after its sha256 matches (scripts/lib/fetch-tool.sh). Runs on Linux, x86_64 or aarch64.
#
# Usage: scripts/lint.sh
set -euo pipefail

# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"
# shellcheck source=lib/fetch-tool.sh
source "$(dirname "$0")/lib/fetch-tool.sh"
# shellcheck source=../packaging/release/pins.env
source "$NATIVE_ROOT/packaging/release/pins.env"

(($# == 0)) || die "usage: scripts/lint.sh"
[[ "$(host_os)" == linux ]] || die "scripts/lint.sh runs on Linux"
machine="$(uname -m)"
case "$machine" in
  x86_64) arch=X86_64 actionlint_arch=amd64 shellcheck_arch=x86_64 ;;
  aarch64 | arm64) arch=AARCH64 actionlint_arch=arm64 shellcheck_arch=aarch64 ;;
  *) die "no pinned linters for $machine" ;;
esac

sum="ACTIONLINT_${arch}_SHA256"
url="https://github.com/rhysd/actionlint/releases/download/v$ACTIONLINT_VERSION"
url+="/actionlint_${ACTIONLINT_VERSION}_linux_$actionlint_arch.tar.gz"
actionlint="$(fetch_tool "$VENDOR_ROOT" actionlint "$ACTIONLINT_VERSION" "$url" "${!sum}" actionlint)/actionlint"
sum="SHELLCHECK_${arch}_SHA256"
url="https://github.com/koalaman/shellcheck/releases/download/v$SHELLCHECK_VERSION"
url+="/shellcheck-v$SHELLCHECK_VERSION.linux.$shellcheck_arch.tar.xz"
shellcheck="$(fetch_tool "$VENDOR_ROOT" shellcheck "$SHELLCHECK_VERSION" "$url" "${!sum}" \
  "shellcheck-v$SHELLCHECK_VERSION/shellcheck")/shellcheck"

cd "$NATIVE_ROOT"
status=0
echo "actionlint $ACTIONLINT_VERSION: .github/workflows/"
"$actionlint" -shellcheck "$shellcheck" || status=1
echo "shellcheck $SHELLCHECK_VERSION: scripts/*.sh, scripts/lib/*.sh"
# one file per process, in parallel: each follows the sources it names, scripts/lib/native.sh above all
printf '%s\0' scripts/*.sh scripts/lib/*.sh | xargs -0 -n 1 -P "$(nproc)" "$shellcheck" || status=1
for script in scripts/*.py; do
  grep -q -- '--self-test' "$script" || continue
  echo "self-test: $script"
  python3 "$script" --self-test || status=1
done
exit "$status"
