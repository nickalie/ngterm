#!/bin/sh
set -eu
if [ "$(cat /sys/module/apparmor/parameters/enabled 2>/dev/null || true)" = Y ] \
  && [ "$(cat /proc/sys/kernel/apparmor_restrict_unprivileged_userns 2>/dev/null || echo 0)" != 0 ] \
  && command -v apparmor_parser >/dev/null 2>&1; then
  apparmor_parser -r /etc/apparmor.d/ngterm-release
fi
