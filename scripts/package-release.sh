#!/usr/bin/env bash
# Package a release on its native runner.
#
#   scripts/package-release.sh STAGED VERSION OUTPUT_DIR
#     every package of this host from a complete scripts/stage.sh tree;
#   scripts/package-release.sh --macos-bundle STAGED VERSION COMMIT TREE
#     macOS, the build step: TREE/ngterm.app (recording VERSION and COMMIT) and TREE/archive (the loose bin and share
#     of the .tar.gz), from a staged tree, both ad-hoc signed under the hardened runtime (scripts/sign-macos.sh);
#   scripts/package-release.sh --macos-package TREE VERSION ARCH OUTPUT_DIR
#     macOS, the packaging step: the .tar.gz, .app.zip, DMG and PKG from a --macos-bundle TREE. It executes nothing
#     from TREE, so it can run where signing keys are.
#
# The licenses and notices go into share/doc beside the staged files (on Windows, scripts/build-msi.sh does that).
set -euo pipefail
# shellcheck source=lib/native.sh
source "$(dirname "$0")/lib/native.sh"
# shellcheck source=../packaging/release/pins.env
source "$NATIVE_ROOT/packaging/release/pins.env"
usage() {
  die "usage: scripts/package-release.sh STAGED VERSION OUTPUT_DIR
  | --macos-bundle STAGED VERSION COMMIT TREE | --macos-package TREE VERSION ARCH OUTPUT_DIR"
}
mode=all
case "${1:-}" in
  --macos-bundle | --macos-package)
    mode="${1#--}"
    shift
    (($# == 4)) || usage
    [[ "$(host_os)" == macos ]] || die "--$mode runs on macOS"
    ;;
  *) (($# == 3)) || usage ;;
esac
if [[ -n "${NGTERM_WPE_SMOKE_BINARY:-}" && "${RUNNER_ENVIRONMENT:-}" != github-hosted ]]; then
  die "NGTERM_WPE_SMOKE_BINARY is reserved for disposable GitHub-hosted release jobs"
fi
version="$2"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "expected MAJOR.MINOR.PATCH"
work="$(mktemp -d)"
hidden_wpe=""
trap 'if [[ -n "$hidden_wpe" && -d "$hidden_wpe" ]]; then mv "$hidden_wpe" "$wpe"; fi; rm -rf "$work"' EXIT

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

# macos_package TREE ARCH: every macOS package from a macos_bundle tree, executing nothing from it
macos_package() {
  local tree="$1" arch="$2" app="$1/ngterm.app" name
  [[ -d "$app/Contents/MacOS" && -d "$tree/archive/bin" && -d "$tree/archive/share" ]] \
    || die "not a --macos-bundle tree: $tree"
  name="ngterm-$version-macos-$arch"
  codesign --verify --deep --strict "$app"
  tar -C "$tree/archive" -czf "$dist/$name.tar.gz" bin share
  ditto -c -k --sequesterRsrc --keepParent "$app" "$dist/$name.app.zip"
  mkdir -p "$work/dmg-$arch"
  cp -R "$app" "$work/dmg-$arch/"
  ln -s /Applications "$work/dmg-$arch/Applications"
  hdiutil create -volname ngterm -srcfolder "$work/dmg-$arch" -ov -format UDZO "$dist/$name.dmg"
  pkgbuild --component "$app" --install-location /Applications --identifier com.nickalie.ngterm \
    --version "$version" "$dist/$name.pkg"
  pkgutil --payload-files "$dist/$name.pkg" > "$work/pkg-files-$arch"
  grep -q 'Contents/MacOS/ngterm' "$work/pkg-files-$arch"
}

if [[ "$mode" == macos-package ]]; then
  [[ "$3" == x86_64 || "$3" == arm64 ]] || die "the macOS arch is x86_64 or arm64, not $3"
  mkdir -p "$4"
  dist="$(cd "$4" && pwd)"
  macos_package "$(cd "$1" && pwd)" "$3"
  exit 0
fi

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
for binary in ngterm ngtermctl ngterm-session ngterm-headless; do
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
  curl --fail --location --retry 5 "$1" -o "$2"
  echo "$3  $2" | sha256sum --check
}

case "$(host_os)" in
  linux)
    wpe="${NGTERM_RELEASE_WPE:?set NGTERM_RELEASE_WPE}"
    root="$work/ngterm"
    mkdir -p "$root/usr/lib/wpe-webkit-2.0" "$root/usr/lib/gstreamer-1.0" \
      "$root/usr/lib/gio/modules" "$root/usr/libexec/gstreamer-1.0"
    cp -R "$stage/bin" "$stage/share" "$root/usr/"
    touch "$root/usr/share/ngterm/bundled-wpe"
    cp -a "$wpe/lib/"*.so* "$root/usr/lib/"
    cp -a "$wpe/lib/wpe-webkit-2.0/"* "$root/usr/lib/wpe-webkit-2.0/"
    cp -R "$wpe/share/doc/wpewebkit" "$root/usr/share/doc/"
    # Modules copied into the AppDir before linuxdeploy no longer have a dpkg
    # owner there, so retain their original distro copyright notices explicitly.
    for package in libgstreamer1.0-0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good \
      gstreamer1.0-plugins-bad gstreamer1.0-libav glib-networking dconf-gsettings-backend; do
      if [[ -f "/usr/share/doc/$package/copyright" ]]; then
        mkdir -p "$root/usr/share/doc/$package"
        cp -L "/usr/share/doc/$package/copyright" "$root/usr/share/doc/$package/"
      fi
    done
    cp -R /usr/share/common-licenses "$root/usr/share/"
    cp "$NATIVE_ROOT/packaging/linux/apparmor/ngterm" "$root/usr/share/doc/ngterm/apparmor.profile.in"
    if [[ -n "${NGTERM_WPE_SMOKE_BINARY:-}" ]]; then
      install -m755 "$NGTERM_WPE_SMOKE_BINARY" "$root/usr/bin/wpe-release-smoke"
    fi
    mkdir -p "$root/usr/share/icons"
    cp -R "$NATIVE_ROOT/resources/icons/hicolor" "$root/usr/share/icons/"
    mkdir -p "$root/usr/share/applications"
    cp "$NATIVE_ROOT/packaging/linux/com.nickalie.ngterm.desktop" "$root/usr/share/applications/"
    multiarch="$(gcc -print-multiarch)"
    cp -a /usr/lib/"$multiarch"/gstreamer-1.0/*.so "$root/usr/lib/gstreamer-1.0/"
    cp -a /usr/lib/"$multiarch"/gio/modules/*.so "$root/usr/lib/gio/modules/"
    cp /usr/lib/"$multiarch"/gstreamer1.0/gstreamer-1.0/gst-plugin-scanner \
      "$root/usr/libexec/gstreamer-1.0/"
    key="$(upper "$arch")"
    sha_var="LINUXDEPLOY_${key}_SHA256"
    download "https://github.com/linuxdeploy/linuxdeploy/releases/download/$LINUXDEPLOY_VERSION/linuxdeploy-$arch.AppImage" \
      "$work/linuxdeploy.AppImage" "${!sha_var}"
    chmod +x "$work/linuxdeploy.AppImage"
    # Bundle dlopened modules and each helper's dependencies as well as the app's.
    deploy=(--appdir "$root" --desktop-file "$root/usr/share/applications/com.nickalie.ngterm.desktop"
      --icon-file "$root/usr/share/icons/hicolor/256x256/apps/com.nickalie.ngterm.png")
    while IFS= read -r -d '' file; do
      if [[ "$file" == *.so* ]]; then deploy+=(--library "$file"); else deploy+=(--executable "$file"); fi
    done < <(find "$root/usr/bin" "$root/usr/lib" "$root/usr/libexec" -type f -print0)
    LD_LIBRARY_PATH="$root/usr/lib${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" \
      APPIMAGE_EXTRACT_AND_RUN=1 "$work/linuxdeploy.AppImage" "${deploy[@]}"
    install -m755 "$NATIVE_ROOT/packaging/linux/AppRun" "$root/AppRun"
    # linuxdeploy relocates copies into usr/bin and usr/lib. Fix every original
    # nested ELF as well, including WebKit helpers and the GStreamer scanner.
    python3 - "$root" "$NATIVE_ROOT/packaging/linux/runtime-libraries.txt" <<'PY'
import os
import pathlib
import re
import subprocess
import sys

root = pathlib.Path(sys.argv[1])
library = root / "usr/lib"
allowed = {line for line in pathlib.Path(sys.argv[2]).read_text().splitlines() if line and not line.startswith("#")}
allowed.update({"libc.so.6", "libm.so.6", "libmvec.so.1", "libresolv.so.2", "libpthread.so.0",
                "libdl.so.2", "librt.so.1", "libutil.so.1", "libanl.so.1"})
files = []
for folder in (root / "usr/bin", library, root / "usr/libexec"):
    for path in folder.rglob("*"):
        if not path.is_file() or path.is_symlink():
            continue
        with path.open("rb") as source:
            if source.read(4) != b"\x7fELF":
                continue
        relative = os.path.relpath(library, path.parent)
        rpath = "$ORIGIN" if relative == "." else f"$ORIGIN:$ORIGIN/{relative}"
        subprocess.run(["patchelf", "--set-rpath", rpath, str(path)], check=True)
        files.append(path)
# A runner has development libraries installed: reject undeclared host fallbacks
# as well as missing libraries, rather than accepting anything ldd can find there.
environment = os.environ.copy()
environment.pop("LD_LIBRARY_PATH", None)
dependencies = {}
system_libraries = set()
for path in files:
    result = subprocess.run(["ldd", str(path)], env=environment, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, check=False)
    if result.returncode or "not found" in result.stdout:
        raise SystemExit(f"unresolved dependencies: {path}\n{result.stdout}")
    dependencies[path] = re.findall(r"\s+(\S+)\s+=>\s+(/\S+)", result.stdout)
    system_libraries.update(resolved for name, resolved in dependencies[path]
                            if name in allowed and not pathlib.Path(resolved).resolve().is_relative_to(root))
# Host packages bring their own dependency closure (for example Wayland's libffi).
# Their internal dependencies may resolve on the host even when a copy is bundled.
for library_path in sorted(system_libraries):
    result = subprocess.run(["ldd", library_path], env=environment, stdout=subprocess.PIPE,
                            stderr=subprocess.STDOUT, text=True, check=True)
    allowed.update(name for name, _ in re.findall(r"\s+(\S+)\s+=>\s+(/\S+)", result.stdout))
for path, needed in dependencies.items():
    for name, resolved in needed:
        if not pathlib.Path(resolved).resolve().is_relative_to(root) and name not in allowed:
            raise SystemExit(f"undeclared host library: {name} ({path})")
PY
    "$root/usr/bin/ngterm" --build-identity
    "$root/usr/bin/ngtermctl" --help >/dev/null
    if [[ -n "${NGTERM_WPE_SMOKE_BINARY:-}" ]]; then
      # Load and evaluate a real page with the original build prefix unreachable,
      # so helpers cannot silently fall back to their compile-time location.
      "$NATIVE_ROOT/scripts/install-apparmor-profile.sh" "$root/usr/bin/wpe-release-smoke"
      mkdir -p "$work/home" "$work/runtime"
      chmod 700 "$work/runtime"
      mv "$wpe" "$work/original-wpe-prefix"
      hidden_wpe="$work/original-wpe-prefix"
      status=0
      dbus-run-session -- timeout 180s env HOME="$work/home" XDG_RUNTIME_DIR="$work/runtime" \
        XDG_CONFIG_HOME="$work/home/config" XDG_DATA_HOME="$work/home/data" XDG_CACHE_HOME="$work/home/cache" \
        WPE_TEST_MODE=shm WPE_TEST_BACKEND=worker \
        "$root/usr/bin/wpe-release-smoke" frames_arrive_in_the_configured_mode_and_follow_the_geometry || status=$?
      mv "$hidden_wpe" "$wpe"
      hidden_wpe=""
      "$NATIVE_ROOT/scripts/install-apparmor-profile.sh" --uninstall
      rm "$root/usr/bin/wpe-release-smoke"
      ((status == 0)) || die "packaged WPE page smoke test failed ($status)"
    fi
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
    macos_package "$work/tree" "$arch"
    ;;
  windows)
    "$NATIVE_ROOT/scripts/build-msi.sh" "$dist/ngterm-$version-windows-x86_64.msi" \
      --staged "$stage" --version "$version" --archive "$dist/ngterm-$version-windows-x86_64.zip"
    ;;
esac
