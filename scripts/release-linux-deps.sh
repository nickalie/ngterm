#!/usr/bin/env bash
# Install what the Linux release build needs on a hosted Ubuntu 24.04 runner: the portable WPE WebKit source build
# (scripts/build-release-wpe.sh), the Rust release build and the packaging. Used by release.yml's `linux` job and by
# release-wpe-cache.yml, which builds the same WPE prefix on main for releases to pull from GHCR.
set -euo pipefail
(($# == 0)) || { echo "usage: scripts/release-linux-deps.sh" >&2; exit 2; }
# WPE and the Rust release build both need disk space on hosted runners.
sudo rm -rf /usr/local/lib/android /opt/ghc /usr/share/dotnet
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  build-essential clang-18 lld-18 cmake ninja-build pkg-config ruby perl python3 gperf \
  curl xz-utils patch zstd file patchelf desktop-file-utils \
  bubblewrap xdg-dbus-proxy apparmor dbus-daemon libseccomp-dev libfontconfig-dev libfreetype-dev libwayland-dev \
  wayland-protocols libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev \
  libglib2.0-dev libharfbuzz-dev libharfbuzz-icu0 libicu-dev libjpeg-dev \
  libepoxy-dev libgcrypt20-dev libsoup-3.0-dev libtasn1-6-dev libxml2-dev \
  libpng-dev libsqlite3-dev libwebp-dev zlib1g-dev libavif-dev liblcms2-dev \
  libjxl-dev libwoff-dev libxslt1-dev libinput-dev libudev-dev libdrm-dev \
  libgbm-dev libegl-dev libgles-dev libatk1.0-dev libatk-bridge2.0-dev \
  libhyphen-dev libenchant-2-dev flite1-dev libmanette-0.2-dev \
  libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev libgstreamer-plugins-bad1.0-dev \
  gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad \
  gstreamer1.0-libav glib-networking libvulkan1 mesa-vulkan-drivers
