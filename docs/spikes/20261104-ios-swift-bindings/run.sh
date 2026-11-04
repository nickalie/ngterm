#!/usr/bin/env bash
# The Swift bindings probe of the iOS plan (docs/plans/20261103-ios-app.md, Task 3): ngterm-mobile builds as a
# staticlib for both iOS targets, its Swift bindings (uniffi.toml's [bindings.swift]) compile under Swift 6 with
# complete concurrency checking and warnings as errors, a program over them links against each archive, and the
# simulator's runs (main.swift). Run it on a Mac after scripts/setup-ios.sh:
#
#   docs/spikes/20261104-ios-swift-bindings/run.sh
#
# Its output stays in this directory's DerivedData/ (gitignored). It exits non-zero when a step fails. Task 4's
# scripts/build-ios.sh and the NgtermCore package take this over; the probe stays as the record of the first check.
#
# Result on the user's M2 (2026-11-04, Xcode 27.2, iOS 27.0 runtime, UniFFI 0.32.2): both archives built with no
# change to the crate (native-static-libs: -liconv -lSystem -lc -lm); the bindings compiled with no warning, so no
# setting was narrowed; both programs linked (the device one LC_BUILD_VERSION platform IOS, minos 26.0), and the
# simulator's printed the core's version and a fresh ed25519 key.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
# nothing launched here may reach the user's ngterm
while IFS='=' read -r name _; do
  unset "$name"
done < <(env | grep '^NGTERM_' || true)

cd "$ROOT"
eval "$(scripts/setup-ios.sh --check)"
scripts/build-ghostty-vt.sh >/dev/null

OUT="$HERE/DerivedData"
BINDINGS="$OUT/bindings"
TARGET="${CARGO_TARGET_DIR:-$ROOT/target}"
rm -rf "$OUT"
mkdir -p "$BINDINGS"

for target in aarch64-apple-ios aarch64-apple-ios-sim; do
  cargo rustc -p ngterm-mobile --lib --crate-type staticlib --target "$target" --release
done

# library mode reads the interface from the library's metadata, the same on every target: the host build serves
cargo build -p ngterm-mobile --lib
cargo build -p ngterm-mobile --features bindgen --bin uniffi-bindgen
"$TARGET/debug/uniffi-bindgen" generate --library "$TARGET/debug/libngterm_mobile.dylib" --language swift \
  --no-format --out-dir "$BINDINGS"

# swiftc's SDK, its target triple and the Rust target of the archive, per platform
build() {
  local sdk="$1" triple="$2" rust_target="$3"
  xcrun --sdk "$sdk" swiftc -swift-version 6 -strict-concurrency=complete -warnings-as-errors -module-name Probe \
    -target "$triple" -I "$BINDINGS" -Xcc "-fmodule-map-file=$BINDINGS/NgtermCoreFFI.modulemap" \
    "$BINDINGS/NgtermCoreBindings.swift" "$HERE/main.swift" \
    "$TARGET/$rust_target/release/libngterm_mobile.a" -liconv -o "$OUT/probe-$sdk"
}
build iphoneos arm64-apple-ios26.0 aarch64-apple-ios
build iphonesimulator arm64-apple-ios26.0-simulator aarch64-apple-ios-sim
otool -l "$OUT/probe-iphoneos" | grep -A3 LC_BUILD_VERSION

output="$(scripts/ios-sim-runner.sh "$OUT/probe-iphonesimulator")"
echo "$output"
version="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -n1)"
[[ "$output" == "core $version"$'\n'"key ssh-ed25519 SHA256:" ]] || {
  echo "unexpected output from the simulator (want core $version and an ed25519 key)" >&2
  exit 1
}
echo "the Swift bindings probe passed"
