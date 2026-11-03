"""What each implementation calls the live-session control group: the shared tests speak ngterm's names.

The tests, the catalog and the `covers(...)` markers speak ngterm's vocabulary (`live.list`, `result.live`,
`backedByLive`, `ngtermctl live`). Each adapter carries a `Vocabulary` that translates every vocabulary-dependent value
a test or the harness sends or reads: the Rust build's is ngterm's own (`LIVE`), and only the Swift reference's
(impls/swift_macos.py) maps it to upstream's names for the same group.

The live sessions' host differs too: the Rust build runs one session server per instance (`ngterm-session`) under its
attach contract's `NGTERM_LIVE_*` variables in `/tmp/ngterm-live-<hash>`, its sessions named `ngterm-<pane>`; the Swift
reference runs upstream's own per-session daemons. Each vocabulary carries its host's environment names, socket
directory, daemon names, leading client and leftover processes.
"""

from __future__ import annotations

import os
import re
import sys
from collections.abc import Mapping, Sequence
from dataclasses import dataclass, field
from typing import TYPE_CHECKING, Any

from agterm_conformance import transport

if TYPE_CHECKING:
    from collections.abc import Callable

    from agterm_conformance.pty_process import PtyProcess

# How long the Rust build's session server outlives its last client and session (`IDLE_GRACE`,
# crates/ngterm-session/src/server/lifecycle.rs): the leftover check gives it that long to exit by itself.
SERVER_IDLE_GRACE = 3.0

# The group's name as the shared tests spell it: ngterm's.
GROUP = "live"
# An unlisted text naming the group, which a translating vocabulary refuses (`Vocabulary.text`).
_NAMES_GROUP = re.compile(rf"\b{GROUP}\.|\b{GROUP} session")


def fnv1a64(text: str) -> str:
    return transport.fnv1a64(text.encode())


def foundation_resolved(path: str) -> str:
    """Darwin Foundation's resolvingSymlinksInPath, which drops a leading /private when the shorter path exists."""
    if sys.platform == "darwin" and path.startswith("/private/") and os.path.exists(path[len("/private") :]):
        return path[len("/private") :]
    return path


def live_socket_directory(
    state_dir: str | os.PathLike[str], name: str = "ngterm", environ: Mapping[str, str] | None = None
) -> str:
    """`ngterm_core::live::socket_directory`: the Rust build's namespace of its session server, FNV-1a 64 of the
    canonical state dir under `/tmp/<name>-live-` on Unix (`name` is `Adapter.app_dir_name`); on Windows
    `<LOCALAPPDATA>\\<name>\\live\\<hash>` of the instance's environment `environ` (`windows_socket_directory`), the
    hash over the state dir as `realpath` spells it there."""
    digest = fnv1a64(foundation_resolved(os.path.realpath(state_dir)))
    if sys.platform == "win32":
        local = (environ if environ is not None else os.environ).get("LOCALAPPDATA", "").rstrip("\\/")
        if not local:
            raise ValueError("no LOCALAPPDATA to derive the Windows live-session namespace from")
        return f"{local}\\{name}\\live\\{digest}"
    return f"/tmp/{name}-live-{digest}"


def _live_leading_client(instance: Any, endpoint: Mapping[str, Any], daemon: str) -> PtyProcess:
    # Late: live_client imports the adapter base, which imports this module.
    from agterm_conformance import live_client

    return live_client.leading_client(instance, endpoint, daemon)


@dataclass(frozen=True)
class Vocabulary:
    """One implementation's names for what the shared tests call by ngterm's."""

    name: str
    # The control-protocol group (`<group>.list`) and the CLI's subcommand group.
    group: str
    # `result.<key>` of `<group>.list`.
    result_key: str
    # `tree.session.<key>` and `tree.session.surfaces[].<key>`.
    backed_key: str
    # The variable a UI-test launch sets to keep live sessions.
    ui_test_opt_in: str
    # The attach contract's variables a live pane's children carry, which the CLI's `clipboard set` reads.
    session_variable: str
    directory_variable: str
    # Their common prefix, which a test strips to stand outside any live pane.
    variable_prefix: str
    # The prefix of the daemon name the app gives a pane's live session (`<prefix><pane identity>`).
    daemon_prefix: str
    # ngterm's error and output texts (`str.format` templates) to this implementation's; one not listed is unchanged.
    texts: Mapping[str, str] = field(default_factory=dict)
    # `socket_directory(state_dir, app_dir_name, environ)`: the instance's namespace, named by the adapter's
    # `app_dir_name`; `environ` is the instance's launch environment, where a namespace depends on it (Windows).
    socket_directory: Callable[..., str] = live_socket_directory
    # `leading_client(instance, endpoint, daemon)`: a second client of a daemon that claims its lead.
    leading_client: Callable[[Any, Mapping[str, Any], str], PtyProcess] = _live_leading_client
    # The executable of the one server per namespace that hosts every live session (`<server> server <directory>`),
    # or None where every session is a daemon of its own; with one, every process carrying the directory variable is
    # the namespace's too, and the leftover check gives the server `server_idle` seconds more to exit.
    server: str | None = None
    server_idle: float = 0.0
    # Without a server: the process names of the per-session daemons and their host, and `hosts(words, directory)`,
    # whether a process's argv hosts the namespace without carrying the directory variable.
    daemon_names: frozenset[str] = frozenset()
    hosts: Callable[[Sequence[str], str], bool] | None = None

    def command(self, live: str) -> str:
        """The wire name of ngterm's `live.<name>`."""
        group, dot, rest = live.partition(".")
        if group != GROUP or not dot or not rest:
            raise ValueError(f"{live!r} is not a command of the {GROUP} group")
        return f"{self.group}.{rest}"

    def cli(self, *args: str) -> tuple[str, ...]:
        """The CLI argv of ngterm's `ngtermctl live ...`."""
        if not args or args[0] != GROUP:
            raise ValueError(f"{args!r} is not an `ngtermctl {GROUP}` command line")
        return (self.group, *args[1:])

    def text(self, live: str, **fields: Any) -> str:
        """This implementation's form of ngterm's text `live`, with `fields` filled in.

        A translating vocabulary refuses an unlisted text that names the group: passing it through unchanged would let
        a test assert a string the implementation does not print.
        """
        translated = self.texts.get(live)
        if translated is None:
            if self.group != GROUP and _NAMES_GROUP.search(live):
                raise KeyError(f"the {self.name} vocabulary has no text for {live!r}")
            translated = live
        return translated.format(**fields) if fields else translated


LIVE = Vocabulary(
    name="live",
    group=GROUP,
    result_key="live",
    backed_key="backedByLive",
    ui_test_opt_in="NGTERM_UITEST_ENABLE_LIVE",
    # The session server's attach contract (crates/ngterm-session/src/cli.rs).
    session_variable="NGTERM_LIVE_SESSION",
    directory_variable="NGTERM_LIVE_DIR",
    variable_prefix="NGTERM_LIVE_",
    # `ngterm_core::live::daemon_name`
    daemon_prefix="ngterm-",
    server="ngterm-session",
    server_idle=SERVER_IDLE_GRACE,
)
