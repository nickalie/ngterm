#!/usr/bin/env bash
# The host side of `ngtermctl server install` (upstream's scripts/server-host-install.sh), shipped in the Linux release
# archive as usr/share/ngterm/server-install.sh and run on the host once the archive is swapped in under
# ~/.local/lib/ngterm: the links, the systemd user unit (written once), linger, start or restart, the running
# server's answer, the agent hooks. Exit 0 installed, 3 installed with the agent hooks incomplete, anything else failed.
set -euo pipefail
lib="$HOME/.local/lib/ngterm"
bin="$lib/usr/bin"
unit="$HOME/.config/systemd/user/ngterm-headless.service"
mkdir -p "$HOME/.local/bin" "$(dirname "$unit")"
ln -sfn "$bin/ngtermctl" "$HOME/.local/bin/ngtermctl"
ln -sfn "$bin/ngterm-headless" "$HOME/.local/bin/ngterm-headless"

# written once, so a user's edits and drop-ins survive an upgrade; the sessions live in ngterm-session, which outlives
# ngterm-headless, so a stop or restart ends this one process and leaves the sessions running
if [[ ! -e "$unit" ]]; then
  cat >"$unit" <<'UNIT'
[Unit]
Description=ngterm-headless

[Service]
ExecStart=%h/.local/lib/ngterm/usr/bin/ngterm-headless
Environment=PATH=%h/.local/bin:/usr/local/bin:/usr/bin:/bin
KillMode=process
Restart=on-failure
RestartSec=1

[Install]
WantedBy=default.target
UNIT
  echo "wrote $unit"
fi

# without linger the user manager stops at the last logout and takes the server and every session with it
user="$(id -un)"
if [[ "$(loginctl show-user "$user" -p Linger --value 2>/dev/null || true)" != yes ]]; then
  if ! loginctl enable-linger "$user" 2>/dev/null; then
    echo "server install: lingering is off for $user and enabling it was refused." >&2
    echo "Without it systemd stops ngterm-headless and every session at your last logout." >&2
    echo "Ask an administrator to run: loginctl enable-linger $user, then run this again." >&2
    exit 1
  fi
  echo "enabled lingering for $user"
fi

systemctl --user daemon-reload
systemctl --user enable ngterm-headless >/dev/null 2>&1
if systemctl --user is-active --quiet ngterm-headless; then
  systemctl --user restart ngterm-headless
else
  systemctl --user start ngterm-headless
fi

# a kept unit may move the state directory by Environment or EnvironmentFile, so the CLI runs with the running
# server's own environment and working directory, and resolves the socket the way it did
pid=0
for _ in $(seq 50); do
  pid="$(systemctl --user show -p MainPID --value ngterm-headless)"
  [[ "$pid" != 0 && -r "/proc/$pid/environ" ]] && break
  sleep 0.2
done
[[ "$pid" != 0 ]] || { echo "server install: ngterm-headless did not start" >&2; exit 1; }
mapfile -d '' environment <"/proc/$pid/environ"
ctl=(env -i "${environment[@]}" "$bin/ngtermctl" version)
cd "/proc/$pid/cwd"
for _ in $(seq 50); do "${ctl[@]}" >/dev/null 2>&1 && break; sleep 0.2; done
"${ctl[@]}" || { echo "server install: ngterm-headless is not answering" >&2; exit 1; }
cd "$HOME"

status=0
# a login shell, so the PATH and OpenCode variables are the ones an agent started in a pane sees; csh and tcsh
# take -l only as their sole flag, so they read the command from stdin
shell="${SHELL:-/bin/sh}"
install_hooks() {
  case "${shell##*/}" in
    csh | tcsh) printf '%s\n' "$bin/ngterm-headless --install-agent-hooks" | "$shell" -l ;;
    *) "$shell" -lc "$bin/ngterm-headless --install-agent-hooks" ;;
  esac
}
if ! install_hooks; then
  echo "server install: the agent hooks install is incomplete; see above, then run ~/.local/bin/ngterm-headless --install-agent-hooks" >&2
  status=3
fi

# this script runs as a plain ssh command, so its PATH is the one the note is about
if ! command -v ngtermctl >/dev/null 2>&1; then
  echo "note: a plain ssh command does not find ngtermctl, because that shell's PATH lacks ~/.local/bin."
  echo "ngterm's own remote commands add it themselves; for your scripts use ~/.local/bin/ngtermctl or add it to PATH."
fi
exit "$status"
