#!/usr/bin/env bash
# Build the downloadable Linux web engine on the Ubuntu release runner.
# NGTERM_RELEASE_WPE must name the release WPE prefix. --print-key resolves the
# same inputs and package versions, without copying, patching or archiving them.
# Usage: scripts/package-web-engine.sh [--arch x86_64|aarch64] OUTPUT_DIR
#        scripts/package-web-engine.sh [--arch x86_64|aarch64] --print-key
set -euo pipefail
# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"
[[ "$(host_os)" == linux ]] || die "the web engine is packaged on Linux"
exec python3 "$NATIVE_ROOT/scripts/web-engine-closure.py" package \
  --prefix "${NGTERM_RELEASE_WPE:?set NGTERM_RELEASE_WPE}" "$@"
