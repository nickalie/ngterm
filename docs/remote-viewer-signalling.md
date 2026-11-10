# Signalling a viewer from a remote session

How a session on another machine reaches the desktop showing it, and the rules that follow for anything built on top.

## The connection runs one way

The viewer opens every connection. For each remote row it keeps one ssh process running:

```
ssh HOST ngtermctl live present <origin session id>
```

The origin answers down that open connection with presentation frames: status, context, info, name, HUD, notifications,
asks and overlays it hands over, and the layout. A program on the origin only talks to the origin's own ngterm (this is how
`session-label.sh` renames a row), and the origin passes the change on.

The origin never connects to the viewer. Viewers usually have no `sshd` running, and need none.

## Rules for new work

When something on the origin has to make a viewer act:

- Carry it over the presentation stream, as a hook on each side. The origin's hook makes a change the stream mirrors,
  usually `ngtermctl notify` on the session, and the viewer's `on notify` hook acts on the mirrored event.
- Don't propose ssh from the origin to the viewer, a reverse tunnel, or a forwarded control socket.
- Don't propose an always-running watcher or polling process on the viewer.
- Don't present those as options for the user to choose between. If the stream really can't carry something, say what is
  missing from it and propose the ngterm change.

A notify reaches only the viewers attached to that session, once, which is usually the wanted scope. It is a live event
and never part of a snapshot, so a viewer that connects later doesn't see it.

## Remote companions, without changing ngterm

The stream has no frame for a new terminal companion (a headless origin hands over only page companions), so a companion
started on the origin got no row on the viewer. These two hooks fill that in. They are user configuration, not part of
ngterm, in `~/.config/ngterm/remote-companions/`:

| machine | `hooks.conf` line | what it does |
|---|---|---|
| origin | `on session.created "$HOME/.config/ngterm/remote-companions/announce.sh"` | for a session with a parent that has viewers, posts a `Companion started` notify on the parent, with the new id in brackets at the end of the body |
| viewer | `on notify "$HOME/.config/ngterm/remote-companions/attach.sh"` | on that title, waits until the companion is listed, then runs `live attach` under the parent, puts the selection back and clears the badge it raised |

Details that are easy to break:

- `live attach` always selects the new row and has no `--no-select`, so `attach.sh` reselects the previous row.
- New Companion on a remote row attaches by itself and the origin announces it too. `attach.sh` waits three seconds, then
  leaves alone anything that already has a `live present` process or a row showing its daemon.
- `announce.sh` skips page companions and parents with no viewers, since on a desktop origin the notify would otherwise
  only add a banner there.
- `attach.sh` detaches after matching, because a hook line runs one process at a time.
- Each new companion shows one `Companion started` banner on the viewer.

Attach failures go to `~/.local/state/ngterm/remote-companions.log`.

To add a machine, copy the script and add the line for its role, then run `ngtermctl hooks reload --socket <socket>` there.
Both scripts need `jq`.
