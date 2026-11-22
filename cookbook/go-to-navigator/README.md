# Go to navigator

One chord opens a searchable tree of the window's workspaces, sessions and panes with each agent's status, and Enter jumps there.

Author: [@nickalie](https://github.com/nickalie), after the Go to window [@avlebedinsky](https://github.com/avlebedinsky) proposed in [pull request #28](https://github.com/nickalie/ngterm/pull/28), itself modeled on herdr's navigator.

## What it does

`go-to-navigator.sh` opens a floating page over the session you are in, as tall as its list needs. It lists every workspace of the window with the git branch of its first session, and under each one its sessions on `├─`/`└─` lines, nested as the sidebar nests them: a session's child sessions hang under it, and a split session lists its two panes as `agent · session`. Each pane row shows, in aligned columns, the git branch checked out in the pane's directory (a long one cut at its end, `feature/very-long-b…`), that directory with your home as `~` (a long one cut from the left, `…/src/components`), a `●` on the pane you are in, the agent running there (`terminal` without one), its status (`shell`, `idle`, `working`, `blocked`, `done` or `unknown`) and how long ago that status was set (`now`, `45s`, `5m`, `2h`, `3d`). A counter on top says how many terminals the rows cover, and the two lines under the rows give the selected row's `workspace / session` and its full working directory, or a workspace's branch. A search marks the selected row's match on these lines too: a match in its label on the first line, and, when the search keeps it by its title or by a match its columns cut, the second line marks it on the full working directory, or puts the title or branch in front of it, or alone when both do not fit.

The search is fuzzy, like fzf: `gtn` finds `go-to-navigator`, and the letters that matched are marked on the row. An active filter shows as a chip in the search bar, and its key lights up in the hints line.

Keys:

| key | does |
|---|---|
| ↑ ↓, `j` `k` | move one row; Ctrl+U and Ctrl+D (or Page Up/Down) move eight |
| ← → | jump to the previous or next workspace |
| Home, End or `G` | first pane row, last row |
| `/` | search: type words, and every one must fuzzy-match the same field, its letters in that order but not always side by side (session, label, cwd, branch, agent, the pane's terminal title, or the workspace name and branch); the matched letters are marked, a run of them preferred to scattered ones; a row kept by its title, or by a branch or cwd its column cuts, shows that text dimmed after its label, and on the second line under the rows once selected; Esc leaves the search |
| `b` `w` `i` `d` | show only blocked, working, idle or done panes; a chip in the search bar names the filter |
| `t` | show the top-level sessions only, a split one as one row, with its session's branch, cwd and age |
| `a`, Backspace | drop the filter (`a` also clears the search) |
| Enter, double-click | go to the row: a workspace, a session, or a pane, focused |
| Esc | close |

The rows stay live while the page is open: a status change or a new session shows up within a couple of seconds. Nothing is closed or changed; the only thing the recipe does is select a workspace or session and focus a pane.

## Requirements

- ngterm 0.1.0 or later: HTML overlay pages with `--js`, `--chromeless`, `--block` and a `--height` in points, the `htmlOverlayZoom` page zoom in `settings.json`, the page bridge's `ngterm.request`, `session.overlay.submit` and `session.overlay.resize` with a `height` in points, and the `tree` fields the page reads (`parent`, `hasSplit`, `splitFocused`, `splitCwd`, `statusPane`, `statusChangedAt`, `remoteHost`, `htmlOverlays[].zoom`) all shipped in it.
- `bash`, `jq` and `git`.
- macOS or Linux. The script is bash, so on Windows it runs only from a bash such as Git Bash, which this recipe has not been tried in.

## Setup

Copy the recipe's directory into place, keeping `assets/` beside the script, which reads the page from there:

```sh
mkdir -p ~/.config/ngterm/recipes
cp -R go-to-navigator ~/.config/ngterm/recipes/
chmod +x ~/.config/ngterm/recipes/go-to-navigator/go-to-navigator.sh
```

Add a line to `~/.config/ngterm/keymap.conf`:

```
command "Go to" ctrl+shift+k --error-hud ~/.config/ngterm/recipes/go-to-navigator/go-to-navigator.sh
```

Pick another chord if that one is taken: `ngtermctl keymap list` shows which are. Apply the file with File ▸ Reload Keymap or `ngtermctl keymap reload`. `--error-hud` puts a failure (a missing `jq`, say) on screen for ten seconds instead of only in a notification.

Fired from a chord, the script runs with the app's `PATH` plus the directory of the bundled `ngtermctl`, so a bare `ngtermctl` resolves. A `jq` or `git` your shell profile adds from elsewhere may not: write the absolute path into the script, or wrap the keymap line in `$SHELL -lc '…'`. Set `NGTERMCTL` to the CLI's full path if it sits somewhere unusual.

## Usage

Press the chord, type `/api` to narrow the list to what mentions `api`, or press `b` for the panes waiting on you, move to a row and press Enter. Esc leaves everything as it was.

From a shell inside ngterm, `go-to-navigator.sh` opens it over the selected session. `go-to-navigator.sh page` writes the page with the current tree to a temporary file and prints its path; open that in a browser to check the rows without ngterm (Enter does nothing there).

## How it works

The script reads `ngtermctl tree --json` for the window the chord fired in (`$AGT_WINDOW_ID`), runs `git symbolic-ref` in each local pane's directory (a split pane's `splitCwd` too) for the branch names, and builds one self-contained page: the template in `assets/go-to.html`, the model in `assets/go-to.js` and the data, all inlined, because a page opened without `--cwd` has no file access. The data is the tree, the branches, `$HOME` (so the page can write `~`) and `$AGT_SESSION_ID`. It opens the page with `session overlay open --html … --js --chromeless --size-percent 80 --height <points> --block` on the session the chord fired from, and blocks.

The panel opens at the height the page is about to ask for: the app draws it before the page paints, so a panel opened at 80% of the session showed up tall and empty and then shrank. The script counts the full list's rows in the tree as the page builds them: one per workspace, one per session, three for a split one (the session and its two panes), none for a page companion. It adds them up in CSS pixels as `assets/go-to.html` lays them out: 145 for the search bar, the two lines under the rows, the hints, the gaps and the padding, 21 per row and 6 above every workspace after the first, or 180 in all for the one line an empty window shows. That sum holds only because every height in it is a whole number of pixels. Text sits on 19-pixel lines and the hints on 17-pixel ones: a `line-height` of 1.45 made 18.85-pixel lines that no sum of rows hits. The search bar's frame and the rule above the two lines are inset `box-shadow`s, not borders: WebKit snaps a border to whole device pixels, so at a page zoom of 1.25 a 1-pixel border is 0.8 of a pixel. The script then multiplies by the page zoom, `htmlOverlayZoom` in `settings.json` in the app's state directory. That is first the directory of the socket the app hands the script (`$AGT_SOCKET` or `$NGTERM_SOCKET`), when the socket is the app's default `ngterm.sock` and a `settings.json` sits beside it: the app does not pass its state directory on, and a shell profile can set `XDG_DATA_HOME` to somewhere the app does not look. Otherwise it is `$NGTERM_STATE_DIR`, else `$XDG_DATA_HOME/ngterm`, else `~/.local/share/ngterm`. The zoom is held to 0.5–3 as the app holds it, and is 1 when the file or the key is missing or unreadable. The script rounds the product up as the page does, and both take a thousandth of a point off first: WebKit's measures carry float noise, such as 51.0000038 pixels, that at some zooms rounded a whole point above the script's exact sum. The numbers follow the CSS: a change to a height, padding, gap or margin in `go-to.html` changes them in the script's `jq` too, and `go-to.test.js`, which reads them from the page's `<style>`, fails until it does.

The page then fits the panel to its content, keeping the width. Once the fonts have loaded and the bridge has answered its first `tree`, it lays out the full list, every row with no search or filter, in a hidden copy, adds the search bar and the footer, and multiplies that height in CSS pixels by the page zoom: `htmlOverlays[].zoom` of the page's own entry, on the session `$AGT_SESSION_ID` names, else on the selected one. It asks for that height with `ngterm.request('session.overlay.resize', {args: {height}})`, in points: the height the panel opened at, so nothing moves, and when the two differ, this resize corrects the panel. Measure the content, never the root element: the root's `scrollHeight` is never less than the viewport, so a panel measured that way can never shrink. Because the height is the full list's, the panel stays still while you search or filter; the rows that a filter hides leave space under the list. The filter chip draws its frame with an inset `box-shadow`, not a `border`: a border made the search bar 2 pixels taller while a filter was on, and the next refresh resized the panel. The page measures again only when a fresh tree differs from the last one, and asks for a resize only when the height it needs changes. The app caps the panel at 80% of the session, and a longer list scrolls inside it. In a browser (`go-to-navigator.sh page`) there is no bridge, so the page keeps the window's height.

The age column is the page clock minus the session's `statusChangedAt`, recomputed at each refresh. It sits on the pane that holds the session's status: in a split, the other pane has none. The search matches each word as letters in order and marks the shortest stretch of the field that holds them, the first of equally short ones, so a run of letters wins over scattered ones. The cwd field it searches is the one the row shows, with `~`. Columns are as wide as their longest entry in the full list, so they do not move while you filter. A branch keeps its start and a cwd its end, cut to 20 and 28 characters at most, and to less in a narrow panel: the page measures its width in characters when it opens and on every resize, keeps what the widest pane row needs for its tree, a 16-character label, the agent, the status and the age, and gives the rest to the two columns. It cuts the longer column first and drops one left under 6 characters, the cwd before the branch. The page's script cuts both, never CSS, so it knows when a cut hides a match and shows the full text as the hint. A narrow label cuts that hint too, so the selected row's second footer line shows it: a cwd's letters are marked on the full working directory, and a title or branch goes in front of it, or alone when the two do not fit the panel's measured width. A line still too long is cut in the script too, around its first match, with `…` on each side it cuts. CSS cuts the label itself, so the first footer line marks a label match on the selected row's `workspace / session` (and a split pane's agent). Both lines are always there, so a match does not change the panel's height.

The page keeps everything in the browser: rows, search, filters and selection are a pure function of the tree, and every 1.5 seconds it asks `ngterm.request('tree')` for a fresh one, which the bridge answers for the page's own window. Enter answers the page with `session.overlay.submit` and the target (`w:<workspace>`, `s:<session>` or `p:<session>:left|right`), which closes it; the blocked script reads the value and runs `workspace select`, `session select` and, for a pane of a split session, `session focus left|right`. Esc closes the page with `session.overlay.close`, which ends the block with exit 2, and the script does nothing.

Selecting from the script rather than from the page is deliberate: the page sits on the session you started in, and switching away first would leave it to close a page on a session no longer in view.

The agent is read from the pane's foreground process: the first argument whose name is a known agent CLI (`claude`, `codex`, `opencode`, `gemini` and others; the list is `AGENTS` in `assets/go-to.js`), so a wrapper such as `npx claude` still counts. The status is the session's, from the agent-status hooks, and belongs to the pane `statusPane` names (the main pane when it names none). In a split, the other pane reads `unknown` when an agent runs there and `shell` when none does, since ngterm keeps one status per session.

List keys are the letters they type, so `j`, `k` and the filters follow AZERTY, Dvorak or Colemak; on a layout that types no Latin letters they fall back to the key's position on a US keyboard. The search types what the layout types.

`go-to.test.js` checks the rows, filters and keys over a fixed tree, the column cuts for a panel's width, when the page asks for a new height, and, against a stand-in `ngtermctl`, the height the script opens the panel at: `node --test go-to.test.js`.

## Limits

Nothing here closes or deletes anything; Enter only changes which session is selected and which pane has focus.

A session attached from another machine runs `ssh` in its pane as far as this machine can tell, so its panes read `terminal` even when an agent runs there; their status is still right. Such a session gets no branch either.

Branches are read once, when the page opens: a session created while it is open has none until the next time.

The age is how long ago the session's status was last written, not when the pane last printed: a hook that pushes the same status again resets it. ngterm does not keep `statusChangedAt` across a restart, so the age stays blank until the next status arrives.

A remote session's cwd is shown as the tree gives it: `~` stands for this machine's home only.

At page zoom 1 the panel opens at exactly the height the page then asks for. At another zoom WebKit's rounding of zoomed lengths can leave the two a point or two apart, and the page's first resize moves the panel by that much. So does a `settings.json` that parses as JSON but that the app rejects, say for a key of the wrong type: the app then shows pages at zoom 1, while the script still takes `htmlOverlayZoom` from it.

On Windows the script, run from Git Bash, does not find `settings.json` in `%LOCALAPPDATA%\ngterm`, so the panel opens at its height at actual size and the page's first resize fits it, which moves it at any page zoom other than 1.

The page opens over the selected session, so a session that already shows an overlay or page refuses it (`overlay already open`), and `--error-hud` shows that error.

The agent list is a fixed set of names; an agent CLI it does not know reads as `terminal` until you add it to `AGENTS`.
