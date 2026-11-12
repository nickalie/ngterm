package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.PaneClipboard
import com.nickalie.ngterm.core.ffi.PaneEnd
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneRole
import com.nickalie.ngterm.core.ffi.RemoteHud
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.RemoteStatus
import com.nickalie.ngterm.core.ffi.StreamState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import java.util.concurrent.ConcurrentHashMap

/** An attached session, by its machine and its id. */
data class SessionKey(
    val machineId: String,
    val sessionId: String,
)

/** The session [pane] belongs to. */
val PaneRef.session: SessionKey get() = SessionKey(machineId, sessionId)

/**
 * A shown pane as its events left it: its [role] at the session (`null` until its attach reports one), whether it is
 * [covered] (a follower's, or one attaching again with nothing to show yet), how its attach [end]ed (`null` while it
 * runs), the [title] its program set, and how many lead reports (attaches and role changes) it has had in [leads].
 */
data class PaneLive(
    val role: PaneRole? = null,
    val covered: Boolean = false,
    val end: PaneEnd? = null,
    val title: String? = null,
    val leads: Int = 0,
)

/**
 * The agent a pane runs, as its origin announces it: [agent] (`claude`, `codex`, or a name a newer origin knows) and
 * its [conversation] (`null` until the origin knows it).
 */
data class PaneAgent(
    val agent: String,
    val conversation: String?,
)

/**
 * An attached session as its presentation stream left it: the stream's [connection], the agent's [status], its
 * [context], its panes' [layout] (`null` before the first), the [hud] over it, its shown [panes] by id, and the
 * [agents] its panes run (every pane's, shown or not, by id; an origin that predates the chat view announces none,
 * and a stream that drops keeps them, as the origin announces them all again when it is back). Whether
 * the stream has said a status or context yet is [statusSaid] and [contextSaid]: once it has, its `null` is a clear,
 * not a gap the machine's listing fills.
 */
data class LiveSession(
    val connection: StreamState = StreamState.Connecting,
    val status: RemoteStatus? = null,
    val context: String? = null,
    val layout: RemoteLayout? = null,
    val hud: RemoteHud? = null,
    val panes: Map<String, PaneLive> = emptyMap(),
    val agents: Map<String, PaneAgent> = emptyMap(),
    val statusSaid: Boolean = false,
    val contextSaid: Boolean = false,
)

/** Where a shown pane's new frames go: its view's state (`TerminalState`). Called on the core's thread. */
interface PaneFrames {
    /** The pane has a new frame. */
    fun dirty()

    /** The pane has a fresh terminal: read it from scratch. */
    fun attached()
}

/** The device's clipboard as a program sets it: [write] puts text on it (`null` clears it). */
fun interface ClipboardSink {
    fun write(text: String?)
}

/**
 * What the core tells the app about the attached sessions and their shown panes, kept per session in [sessions]: the
 * stream's state, status, context, layout and HUD, each pane's role, cover, end and title, and the agents the panes
 * run. A shown pane's frames
 * go to the [PaneFrames] [route]d to it, and a program's write to the standard clipboard goes to [clipboard] (in
 * [scope]); Android has no selection or primary clipboard, so a write to either is dropped. A detached session's
 * state is let go. A stream that fails before the session was ever shown has the machine listed again, so a session
 * ended elsewhere since its listing leaves. Events arrive on the core's thread; [sessions] takes them from any.
 */
class LiveSessions(
    private val scope: CoroutineScope,
    private val connections: Connections,
    private val clipboard: ClipboardSink,
) {
    private val state = MutableStateFlow<Map<SessionKey, LiveSession>>(emptyMap())
    private val frames = ConcurrentHashMap<PaneRef, PaneFrames>()

    /** Each attached session's live state, by key; one with no event yet is missing. */
    val sessions: StateFlow<Map<SessionKey, LiveSession>> = state.asStateFlow()

    init {
        connections.listen(::onEvent)
        scope.launch {
            var before = emptySet<SessionKey>()
            connections.attached.collect { attached ->
                val now = attached.flatMap { (machine, ids) -> ids.map { SessionKey(machine, it) } }.toSet()
                // only what was attached and no longer is: an event may land before its attach is recorded
                val gone = before - now
                before = now
                if (gone.isNotEmpty()) state.update { it - gone }
            }
        }
    }

    /** Sends [pane]'s frames to [frames] from now on; `null` stops them. */
    fun route(
        pane: PaneRef,
        frames: PaneFrames?,
    ) {
        if (frames == null) this.frames.remove(pane) else this.frames[pane] = frames
    }

    private fun connectionChanged(event: MobileEvent.SessionConnection) {
        val before = state.value[SessionKey(event.machineId, event.sessionId)]
        session(event.machineId, event.sessionId) { copy(connection = event.state) }
        // a stream failing before it ever showed the session may be failing for a session ended elsewhere: the
        // machine is listed again, and a listing without it lets it go (once per run of failures)
        val first = before?.connection !is StreamState.Failed
        if (event.state is StreamState.Failed && before?.layout == null && first) {
            scope.launch { connections.listSessions(event.machineId) }
        }
    }

    private fun onEvent(event: MobileEvent) {
        when (event) {
            is MobileEvent.SessionConnection -> {
                connectionChanged(event)
            }

            is MobileEvent.SessionStatus -> {
                session(event.machineId, event.sessionId) { copy(status = event.status, statusSaid = true) }
            }

            is MobileEvent.SessionContext -> {
                session(event.machineId, event.sessionId) { copy(context = event.context, contextSaid = true) }
            }

            is MobileEvent.SessionLayout -> {
                session(event.machineId, event.sessionId) { copy(layout = event.layout) }
            }

            is MobileEvent.Hud -> {
                session(event.machineId, event.sessionId) { copy(hud = event.hud) }
            }

            is MobileEvent.PaneDirty -> {
                frames[event.pane]?.dirty()
            }

            is MobileEvent.PaneAttached -> {
                attached(event.pane)
            }

            is MobileEvent.PaneTitle -> {
                pane(event.pane) { copy(title = event.title) }
            }

            is MobileEvent.PaneLead -> {
                pane(event.pane) { copy(role = event.role, covered = event.covered, leads = leads + 1) }
            }

            is MobileEvent.PaneEnded -> {
                pane(event.pane) { copy(end = event.end, covered = false) }
            }

            is MobileEvent.ClipboardWrite -> {
                clipboardWrite(event.clipboard, event.text)
            }

            is MobileEvent.PaneAgent -> {
                agent(event)
            }

            else -> {}
        }
    }

    // a fresh terminal: its last end is behind it, and its screen is read from scratch
    private fun attached(pane: PaneRef) {
        pane(pane) { copy(end = null) }
        frames[pane]?.attached()
    }

    private fun agent(event: MobileEvent.PaneAgent) {
        val agent = event.agent?.let { PaneAgent(it, event.conversationId) }
        session(event.machineId, event.sessionId) {
            copy(agents = if (agent == null) agents - event.paneId else agents + (event.paneId to agent))
        }
    }

    private fun clipboardWrite(
        which: PaneClipboard,
        text: String?,
    ) {
        if (which == PaneClipboard.STANDARD) scope.launch { clipboard.write(text) }
    }

    private inline fun session(
        machineId: String,
        sessionId: String,
        crossinline change: LiveSession.() -> LiveSession,
    ) {
        val key = SessionKey(machineId, sessionId)
        state.update { it + (key to (it[key] ?: LiveSession()).change()) }
    }

    private inline fun pane(
        pane: PaneRef,
        crossinline change: PaneLive.() -> PaneLive,
    ) = session(pane.machineId, pane.sessionId) {
        copy(panes = panes + (pane.paneId to (panes[pane.paneId] ?: PaneLive()).change()))
    }
}
