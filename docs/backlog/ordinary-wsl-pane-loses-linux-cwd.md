---
worth: yes
where: crates/ngterm-term/src/launch.rs:226
added: 2026-11-12
---
# Ordinary-mode WSL panes lose their Linux working directory on restore, split and duplicate

A WSL pane reports its Linux directory (`/home/u/proj`) through the bundled OSC 7 hook. Live restores it inside the
distribution (`NGTERM_WSL_RESTORE_CWD`, `shell_profiles/integration.rs`). Without Live, `Host::prepare` takes the
pane's working directory only when it `is_dir()` on Windows, which a Linux path never is, so a restored, split or
duplicated WSL pane starts in the default Windows directory (WSL translates it to `/mnt/c/...`).

Surfaced in the PR 18 revmux round (bugs+impl-5, dropped unverified by synthesis). Deferred: the obvious fix,
appending `--cd <dir>` to `wsl.exe`, makes startup fail when the directory no longer exists (Live's launcher
tolerates that, `wsl.exe --cd` does not), so it needs a tolerant form and a Windows run to check it.
