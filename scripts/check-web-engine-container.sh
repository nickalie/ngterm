#!/usr/bin/env bash
# Runs scripts/check-web-engine-install.sh against a fresh install of a release package in a throwaway container: the
# package is installed first, alone, so what it pulls is what a user gets; the test tools come after. A software GPU,
# a private Xvfb and bus and a null sound server stand in for a desktop. Nothing touches the host's packages.
#
#   ubuntu-24.04    the deb, no WPE to recommend: the runtime is downloaded, then reused by a second start
#   ubuntu-24.04-cache  the deb: scripts/check-web-engine-cache.sh, offline installs and two app versions
#   debian-testing  the deb, the system's WPE
#   tumbleweed      the rpm, the system's WPE
#   arch            the Arch package, depending on the system's WPE
#
# Usage: [PACKAGE_CACHE=DIR] scripts/check-web-engine-container.sh DISTRO PACKAGES_DIR OUT_DIR [ARCHIVE]
#   PACKAGES_DIR  holds the package; ARCHIVE is the runtime (.tar.zst), needed by the ubuntu-24.04 modes
#   PACKAGE_CACHE keeps each distro's downloaded packages between runs
set -euo pipefail

(($# >= 3)) || { sed -n '2,14p' "$0" >&2; exit 2; }
distro="$1" packages="$(cd "$2" && pwd)" out="$3" archive="${4:-}"
repo="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
mkdir -p "$out"
out="$(cd "$out" && pwd)"

case "$distro" in
  ubuntu-24.04 | ubuntu-24.04-cache | debian-testing)
    image="docker.io/library/ubuntu:24.04"
    [[ "$distro" != debian-testing ]] || image="docker.io/library/debian:testing"
    install='apt-get -qq update && apt-get -o Acquire::Retries=5 -o Acquire::http::Timeout=30 install -y /packages/*.deb'
    cache=/var/cache/apt/archives
    # shellcheck disable=SC2016  # expanded by the shell inside the container
    wpe='dpkg-query -W -f="\${Package} \${Version}\n" | grep -i wpe || echo "no WPE package"'
    tools='apt-get install -y --no-install-recommends python3 procps xvfb xauth xdotool dbus mesa-vulkan-drivers libegl-mesa0 libgl1-mesa-dri pulseaudio zstd'
    ;;
  tumbleweed)
    image="registry.opensuse.org/opensuse/tumbleweed:latest"
    install='zypper --non-interactive --no-gpg-checks install --recommends --allow-unsigned-rpm /packages/*.rpm'
    cache=/var/cache/zypp/packages
    wpe='rpm -qa | grep -i wpe || echo "no WPE package"'
    tools='zypper --non-interactive install --no-recommends --force-resolution python3 procps xvfb-run xauth xdotool dbus-1-daemon Mesa-vulkan-device-select libvulkan_lvp Mesa-libEGL1 Mesa-dri pulseaudio'
    ;;
  arch)
    image="docker.io/library/archlinux:latest"
    install='pacman -Syu --noconfirm && pacman -U --noconfirm /packages/*.pkg.tar.zst'
    cache=/var/cache/pacman/pkg
    wpe='pacman -Q wpewebkit || echo "no WPE package"'
    tools='pacman -S --noconfirm --needed python procps-ng xorg-server-xvfb xorg-xauth xdotool dbus vulkan-swrast mesa pulseaudio'
    ;;
  *) echo "unknown distro: $distro" >&2; exit 2 ;;
esac

if [[ "$distro" == ubuntu-24.04-cache ]]; then
  [[ -n "$archive" ]] || { echo "$distro needs ARCHIVE" >&2; exit 2; }
  archive="$(cd "$(dirname "$archive")" && pwd)/$(basename "$archive")"
  runs='
    /repo/scripts/check-web-engine-cache.sh --archive /archive/'"$(basename "$archive")"' --out /out/cache'
elif [[ "$distro" == ubuntu-24.04 ]]; then
  [[ -n "$archive" ]] || { echo "$distro needs ARCHIVE" >&2; exit 2; }
  archive="$(cd "$(dirname "$archive")" && pwd)/$(basename "$archive")"
  runs='
    run first  --expect download --serve /archive/'"$(basename "$archive")"'
    run second --expect cached --serve /archive/'"$(basename "$archive")"
else
  archive=""
  runs='
    run first --expect system'
fi

# shellcheck disable=SC2016  # the script runs inside the container
script='set -euo pipefail
'"$install"'
{ echo "== engine packages after the install"; '"$wpe"'; } | tee /out/installed.txt
'"$tools"'
tester="$(id -nu 1000)"
install -d -o 1000 -m 755 /home/tester
install -d -o 1000 -m 700 /run/user/1000
# pam_env on openSUSE derives the XDG dirs from the passwd entry of the keep-id user (home: /)
runuser -u "$tester" -- env -u XDG_DATA_HOME -u XDG_CONFIG_HOME -u XDG_CACHE_HOME -u XDG_STATE_HOME \
  XDG_RUNTIME_DIR=/run/user/1000 HOME=/home/tester bash -euo pipefail -c "
  pulseaudio -n --load=module-native-protocol-unix --load=module-null-sink --exit-idle-time=-1 &
  for _ in \$(seq 50); do [ -S /run/user/1000/pulse/native ] && break; read -r -t 0.2 <> <(:) || true; done
  run() { name=\$1; shift
    /repo/scripts/check-web-engine-install.sh --ngterm /usr/bin/ngterm --ctl /usr/bin/ngtermctl --out /out/\$name \"\$@\" | tee /out/\$name.json; }
  '"$runs"'
"
'

volumes=(-v "$packages:/packages:ro" -v "$repo:/repo:ro" -v "$out:/out")
if [[ -n "${PACKAGE_CACHE:-}" ]]; then
  mkdir -p "$PACKAGE_CACHE/$distro"
  volumes+=(-v "$PACKAGE_CACHE/$distro:$cache")
fi
[[ -z "$archive" ]] || volumes+=(-v "$(dirname "$archive"):/archive:ro")

# seccomp and masked paths off: what bubblewrap needs for WebKit's real sandbox inside a rootless container. keep-id
# makes the caller the container's uid 1000, so the unprivileged test user writes to OUT_DIR as the caller.
exec podman run --rm --name "web-engine-check-$distro-$$" \
  --security-opt seccomp=unconfined --security-opt label=disable --security-opt unmask=ALL \
  --userns=keep-id --user 0 \
  "${volumes[@]}" "$image" bash -c "$script"
