#!/usr/bin/env bash
# Package a release on its native runner.
#
#   scripts/package-release.sh STAGED VERSION OUTPUT_DIR
#     every package of this host from a complete scripts/stage.sh tree;
#   scripts/package-release.sh --macos-bundle STAGED VERSION COMMIT TREE
#     macOS, the build step: TREE/ngterm.app (recording VERSION and COMMIT) and TREE/archive (the loose bin and share
#     of the .tar.gz), from a staged tree, both ad-hoc signed under the hardened runtime (scripts/sign-macos.sh);
#   then `scripts/sign-macos.sh package` makes the .tar.gz, .app.zip, DMG and PKG from that TREE, where the signing
#     keys are (the release workflow's macos-sign job).
#
# The licenses and notices go into share/doc beside the staged files (on Windows, scripts/build-msi.sh does that).
set -euo pipefail
# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"
# shellcheck source=lib/fetch-tool.sh
source "$(dirname "$0")/lib/fetch-tool.sh"
# shellcheck source=../packaging/release/pins.env
source "$NATIVE_ROOT/packaging/release/pins.env"
usage() {
  die "usage: scripts/package-release.sh STAGED VERSION OUTPUT_DIR | --macos-bundle STAGED VERSION COMMIT TREE"
}
mode=all
case "${1:-}" in
  --macos-bundle)
    mode="${1#--}"
    shift
    (($# == 4)) || usage
    [[ "$(host_os)" == macos ]] || die "--$mode runs on macOS"
    ;;
  *) (($# == 3)) || usage ;;
esac
version="$2"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "expected MAJOR.MINOR.PATCH"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

# macos_bundle TREE COMMIT: the .app and the archive's loose tree from $stage
macos_bundle() {
  local tree="$1" commit="$2" app
  [[ ! -e "$tree" ]] || die "the bundle tree exists: $tree"
  mkdir -p "$tree/archive"
  app="$tree/ngterm.app"
  "$NATIVE_ROOT/scripts/bundle-macos.sh" "$app" --staged "$stage" --version "$version" --commit "$commit"
  echo macos-bundle > "$app/Contents/Resources/update-format"
  "$NATIVE_ROOT/scripts/sign-macos.sh" bundle "$app"
  echo macos-archive > "$stage/share/ngterm/update-format"
  cp -R "$stage/bin" "$stage/share" "$tree/archive/"
  "$NATIVE_ROOT/scripts/sign-macos.sh" loose "$tree/archive/bin"
}

stage="$1"
if [[ "$mode" == macos-bundle ]]; then
  commit="$3"
  tree="$4"
  [[ "$tree" == /* ]] || tree="$PWD/$tree"
else
  mkdir -p "$3"
  dist="$(cd "$3" && pwd)"
fi
if [[ "$(host_os)" == windows ]]; then stage="$(cygpath -u "$stage")"; fi
stage="$(cd "$stage" && pwd)"
exe=""
[[ "$(host_os)" != windows ]] || exe=.exe
package_binaries=(ngterm ngtermctl ngterm-session ngterm-headless)
[[ "$(host_os)" != linux ]] || package_binaries+=(ngterm-web)
for binary in "${package_binaries[@]}"; do
  [[ -f "$stage/bin/$binary$exe" ]] || die "missing staged $binary$exe"
done
[[ "$(head -n1 "$stage/share/ngterm/app-BUILD" | tr -d '\r')" == "$version" ]] \
  || die "staged version does not match $version"
cp -R "$stage" "$work/stage"
stage="$work/stage"
rm -f "$stage/share/ngterm/session-BUILD"
if [[ "$(host_os)" != windows ]]; then
  fonts="$(fonts_prefix)"
  fonts_prefix_is_complete "$fonts" || die "run scripts/fetch-fonts.sh first"
  mkdir -p "$stage/share/doc/ngterm" "$stage/share/doc/fonts"
  cp "$NATIVE_ROOT/LICENSE" "$NATIVE_ROOT/NOTICE" "$NATIVE_ROOT/resources/LICENSE.upstream" "$stage/share/doc/ngterm/"
  cp "$fonts/share/doc/fonts/"* "$stage/share/doc/fonts/"
fi
if [[ "$mode" == macos-bundle ]]; then
  macos_bundle "$tree" "$commit"
  exit 0
fi
arch="$(uname -m)"
[[ "$arch" != arm64 ]] || arch=aarch64

# Execute only checksum-verified release tools. AppImages run extracted, without FUSE.
download() {
  github_download "$1" "$2"
  echo "$3  $2" | sha256sum --check
}

case "$(host_os)" in
  linux)
    root="$work/ngterm"
    mkdir -p "$root/usr/lib"
    cp -R "$stage/bin" "$stage/share" "$root/usr/"
    cp -R /usr/share/common-licenses "$root/usr/share/"
    cp "$NATIVE_ROOT/packaging/linux/apparmor/ngterm" "$root/usr/share/doc/ngterm/apparmor.profile.in"
    mkdir -p "$root/usr/share/icons"
    cp -R "$NATIVE_ROOT/resources/icons/hicolor" "$root/usr/share/icons/"
    mkdir -p "$root/usr/share/applications"
    cp "$NATIVE_ROOT/packaging/linux/com.nickalie.ngterm.desktop" "$root/usr/share/applications/"
    key="$(upper "$arch")"
    sha_var="LINUXDEPLOY_${key}_SHA256"
    download "https://github.com/linuxdeploy/linuxdeploy/releases/download/$LINUXDEPLOY_VERSION/linuxdeploy-$arch.AppImage" \
      "$work/linuxdeploy.AppImage" "${!sha_var}"
    chmod +x "$work/linuxdeploy.AppImage"
    # Only the four app binaries bring libraries into the AppDir. The worker
    # loads either the system's engine or a separately downloaded runtime.
    deploy=(--appdir "$root" --desktop-file "$root/usr/share/applications/com.nickalie.ngterm.desktop"
      --icon-file "$root/usr/share/icons/hicolor/256x256/apps/com.nickalie.ngterm.png")
    for binary in ngterm ngtermctl ngterm-session ngterm-headless; do
      deploy+=(--executable "$root/usr/bin/$binary")
    done
    # linuxdeploy also scans an existing AppDir. Keep the worker outside it
    # until dependency collection has finished.
    mv "$root/usr/bin/ngterm-web" "$work/ngterm-web"
    env -u LD_LIBRARY_PATH APPIMAGE_EXTRACT_AND_RUN=1 "$work/linuxdeploy.AppImage" "${deploy[@]}"
    mv "$work/ngterm-web" "$root/usr/bin/ngterm-web"
    install -m755 "$NATIVE_ROOT/packaging/linux/AppRun" "$root/AppRun"
    python3 "$NATIVE_ROOT/scripts/check-linux-package.py" --prepare "$root"
    "$root/usr/bin/ngterm" --build-identity
    "$root/usr/bin/ngtermctl" --help >/dev/null
    echo linux-archive > "$root/usr/share/ngterm/update-format"
    tar -C "$work" -czf "$dist/ngterm-$version-linux-$arch.tar.gz" ngterm
    # Native package files remain owned exclusively by their package manager.
    rm "$root/usr/share/ngterm/update-format"
    export PACKAGE_ROOT="$root" PACKAGE_VERSION="$version"
    case "$arch" in x86_64) PACKAGE_ARCH=amd64 ;; aarch64) PACKAGE_ARCH=arm64 ;; *) die "unsupported arch: $arch" ;; esac
    export PACKAGE_ARCH
    python3 - "$NATIVE_ROOT/packaging/linux/nfpm.yaml" "$work/nfpm.yaml" <<'PY'
import os
import pathlib
import sys

pathlib.Path(sys.argv[2]).write_text(os.path.expandvars(pathlib.Path(sys.argv[1]).read_text()))
PY
    sha_var="NFPM_${key}_SHA256"
    nfpm_arch="$arch"
    [[ "$arch" != aarch64 ]] || nfpm_arch=arm64
    download "https://github.com/goreleaser/nfpm/releases/download/v$NFPM_VERSION/nfpm_${NFPM_VERSION}_Linux_${nfpm_arch}.tar.gz" \
      "$work/nfpm.tar.gz" "${!sha_var}"
    tar -xf "$work/nfpm.tar.gz" -C "$work" nfpm
    for format in deb rpm archlinux; do
      (cd "$NATIVE_ROOT" && "$work/nfpm" package --config "$work/nfpm.yaml" --packager "$format" --target "$dist/")
    done
    sha_var="APPIMAGETOOL_${key}_SHA256"
    download "https://github.com/AppImage/appimagetool/releases/download/$APPIMAGETOOL_VERSION/appimagetool-$arch.AppImage" \
      "$work/appimagetool.AppImage" "${!sha_var}"
    sha_var="APPIMAGE_RUNTIME_${key}_SHA256"
    download "https://github.com/AppImage/type2-runtime/releases/download/$APPIMAGE_RUNTIME_VERSION/runtime-$arch" \
      "$work/appimage-runtime" "${!sha_var}"
    chmod +x "$work/appimagetool.AppImage"
    ARCH="$arch" APPIMAGE_EXTRACT_AND_RUN=1 "$work/appimagetool.AppImage" \
      --runtime-file "$work/appimage-runtime" --no-appstream "$root" "$dist/ngterm-$version-linux-$arch.AppImage"
    APPIMAGE_EXTRACT_AND_RUN=1 "$dist/ngterm-$version-linux-$arch.AppImage" --build-identity
    ;;
  macos)
    [[ "$arch" != aarch64 ]] || arch=arm64
    macos_bundle "$work/tree" "$(sed -n 2p "$stage/share/ngterm/app-BUILD" | tr -d '\r')"
    "$NATIVE_ROOT/scripts/sign-macos.sh" package "$version" "$dist" --identity ad-hoc "$work/tree" "$arch"
    ;;
  windows)
    "$NATIVE_ROOT/scripts/build-msi.sh" "$dist/ngterm-$version-windows-x86_64.msi" \
      --staged "$stage" --version "$version" --archive "$dist/ngterm-$version-windows-x86_64.zip"
    ;;
esac
