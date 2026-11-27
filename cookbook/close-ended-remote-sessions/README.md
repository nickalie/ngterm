# Close ended remote sessions

One command closes every attached remote session whose session on the other machine has ended: the sidebar rows with a slashed cloud that sit on their disconnect line.

Author: [@nickalie](https://github.com/nickalie)

## What it does

A session attached from another machine (`live attach`, File ▸ Attach Remote…) stays in the sidebar after its session ends over there: its pane prints `ngterm: NAME (left) on HOST disconnected, exit N`, holds on *Process exited. Press any key to close the terminal.*, and the row's cloud is slashed. A machine that runs many short sessions, review agents for example, leaves a column of these behind.

`close-ended-remote-sessions.sh` finds those rows in every open window and closes them, a batch per window, then prints what it closed and how many. Fired from a chord it also shows the count in a HUD for three seconds. `--dry-run` prints the rows it would close and closes nothing.

A row counts as ended only when all of these hold:

- it is attached from another machine (`remoteHost` in `tree`);
- its presentation stream is failing, retrying or stopped (`presentation.state` is `failed`, `connecting` or `ended`, the last once its machine no longer has the session), which is what slashes the cloud;
- no pane is reconnecting, and no process runs in any pane (`foreground` and `foregroundShell` both absent, the split pane's too);
- nothing of this machine's is on the row: no scratch terminal and no overlay, hidden ones included;
- each pane's screen ends on its disconnect line and the hold prompt under it.

A row with companions closes only when every companion has ended too, or is a page; they go with it. A companion that has ended under a parent still running closes on its own.

## Requirements

- ngterm 0.1.0 or later: the `tree` fields the script reads (`remoteHost`, `presentation`, `foreground`, `foregroundShell`, `parent`, `content`, `surfaces`, `overlay`, `paneOverlays`, `htmlOverlays`, `hiddenOverlays`), `session text --lines`, `window list`, and `session close` with a repeated `--target` all shipped in it.
- `bash` and `jq`.
- macOS or Linux. The script is bash, so on Windows it runs only from a bash such as Git Bash, which this recipe has not been tried in.

## Setup

Copy the script into place:

```sh
mkdir -p ~/.config/ngterm/recipes
cp -R close-ended-remote-sessions ~/.config/ngterm/recipes/
chmod +x ~/.config/ngterm/recipes/close-ended-remote-sessions/close-ended-remote-sessions.sh
```

To run it from a chord or the command palette, add a line to `~/.config/ngterm/keymap.conf`:

```
command "Close Ended Remote Sessions" ctrl+shift+x --error-hud ~/.config/ngterm/recipes/close-ended-remote-sessions/close-ended-remote-sessions.sh
```

Pick another chord if that one is taken: `ngtermctl keymap list` shows which are. Apply the file with File ▸ Reload Keymap or `ngtermctl keymap reload`. `--error-hud` puts a failure (a missing `jq`, say) on screen for ten seconds instead of only in a notification.

Fired from a chord, the script runs with the app's `PATH` plus the directory of the bundled `ngtermctl`, so a bare `ngtermctl` resolves. A `jq` your shell profile adds from elsewhere may not: write its absolute path into the script, or wrap the keymap line in `$SHELL -lc '…'`. Set `NGTERMCTL` to the CLI's full path if it sits somewhere unusual.

## Usage

Check what it would close first:

```sh
close-ended-remote-sessions.sh --dry-run
```

```
would close Review task-3 01-initial (buildbox)
would close Review task-3 02-after-fix (buildbox)
would close fix-663 (buildbox)
```

Then run it without the flag, from a shell inside ngterm or with the chord:

```
closing Review task-3 01-initial (buildbox)
closing Review task-3 02-after-fix (buildbox)
closing fix-663 (buildbox)
Closed 3 ended remote sessions
```

## How it works

The script lists the open windows (`window list`), reads each one's `tree --json`, and keeps the attached rows whose stream is down, with nothing reconnecting, nothing running in any pane and no scratch terminal or overlay of this machine's. Only those rows have their screens read: `session text --pane left --lines 6`, and `--pane right` for a session with a split. A pane has ended when its last lines are the disconnect line ngterm prints after a remote pane's ssh exits and the hold prompt under it. The match ignores whitespace, so a long session name the pane's width wrapped onto two lines still matches.

No single field says a row has ended, so the script asks for all four signals. The slashed cloud alone is not enough: it shows whenever the stream to that machine is down, so a laptop that is asleep or a host that is rebooting slashes every row from it, while their panes sit on a reconnecting bar rather than the hold prompt. The pane's held screen alone is not enough either: a pane can stop while its session goes on over there, and then the stream stays connected and the cloud is whole. Such a row is left for you to reattach or close.

A scratch terminal or an overlay on an attached row runs here, not on the other machine, and closing the row would end it, so such a row is kept however dead its panes are. ngterm's own cleanup of ended companion rows keeps them for the same reason.

The script reads the window list and each tree into variables before looping over them, so a socket error or a missing `jq` stops it with that error rather than reading as an empty list and reporting nothing ended. It is plain bash 3.2, the version macOS ships as `/bin/bash`, which a chord's `PATH` usually finds first: nothing expands an array that can be empty, since bash before 4.4 calls one unbound under `set -u`.

Each window's rows close in one `session close` with a `--target` per row, so with the undo grace on in Settings they come back as one undo record (Reopen Closed Item). A single row is closed with `--undoable` for the same grace. A companion whose parent closes is not given its own `--target`: closing the parent takes it along.

`close-ended-remote-sessions.test.js` runs the script against a stub `ngtermctl` with a fixed set of windows and screens and checks which rows it closes: `node --test close-ended-remote-sessions.test.js`.

## Limits

**This closes sessions.** It closes rows in this ngterm without asking first, in every open window. Run `--dry-run` first if you want to see the list. Closing an attached row ends only this side: nothing is closed or killed on the other machine, and a session that still exists there can be attached again with File ▸ Attach Remote….

The scrollback of a closed row goes with it. Undo brings a batch back only while the undo grace in Settings runs, and an attached row gets no Recent Closed entry.

A row whose machine is unreachable is never closed, even when its session there is long gone: its panes sit on the reconnecting bar, not the disconnect line. Run the script again once the machine answers.

A row attached from a machine whose ngterm is too old to stream (`presentation.state` `unsupported`) is never closed: with no stream there is nothing to tell an ended session from a live one whose pane merely stopped. Close those by hand.

A row with a scratch terminal or an overlay open, even hidden, is kept. Close the scratch or the overlay and run it again.

A row that has not drawn its terminal yet (`realized: false`, a relaunch waiting for its machine) has no screen to read and is left alone.

The match relies on the exact wording of the disconnect line and of the hold prompt. If a future ngterm changes either, the script finds nothing until the pattern in `pane_ended` is updated; it never closes more.

A program on the other machine that prints the same two lines while its session is alive does not fool it: that pane still has `ssh` in its foreground.

Rows in closed windows are not touched.
