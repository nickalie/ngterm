package com.nickalie.ngterm.core.fake

import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.ClipboardReply
import com.nickalie.ngterm.core.ffi.EventSink
import com.nickalie.ngterm.core.ffi.Machine
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileCoreInterface
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.NoHandle
import com.nickalie.ngterm.core.ffi.Pane
import com.nickalie.ngterm.core.ffi.PaneSize
import com.nickalie.ngterm.core.ffi.RemoteSession

/** One call the app made on a [FakeMobileCore]: the method's name and its arguments, in order. */
data class FakeCall(
    val method: String,
    val args: List<Any?> = emptyList(),
)

/**
 * A [MobileCoreInterface] with no native library behind it, for the app's JVM and UI tests. It records every call in
 * [calls], keeps each machine's sessions in [sessions], hands [sink] the events a test [emit]s, and throws the
 * failure a test queued with [failNext] from that method's next call.
 *
 * It follows the real core's rules (crates/ngterm-mobile, api.rs and actor.rs) where the app sees them:
 * - [connect] reports [MachineState.Connected] as a [MobileEvent.MachineState] when the machine was not connected;
 *   [disconnect] reports nothing and forgets the machine's listing;
 * - [listSessions] keeps what it answers as the machine's listing and also delivers it as a [MobileEvent.Tree];
 *   [createSession] does the same with the listing that has the new session;
 * - [endSession] ends a session of [sessions] whether listed or not ([MobileException.Remote] for one the machine
 *   doesn't have) and delivers a [MobileEvent.Tree] only when a listing is kept;
 * - [attachSession] and [showPane] take only a session of the machine's last listing ([MobileException.NotListed]
 *   otherwise), not of [sessions]: seeding [sessions] without listing them is refused, as the real core refuses a
 *   session it has not listed.
 *
 * [showPane] returns what [paneFactory] makes; the default is a `Pane(NoHandle)`, a placeholder whose methods must not
 * be called (they would reach the native library): a test that drives a pane subclasses it.
 */
class FakeMobileCore(
    private val sink: EventSink? = null,
) : MobileCoreInterface {
    private val recorded = mutableListOf<FakeCall>()
    private val failures = mutableMapOf<String, MobileException>()
    private var created = 0

    /** The calls made so far, oldest first. */
    val calls: List<FakeCall> get() = recorded.toList()

    /** The machines connected (and not disconnected since), by id. */
    val connected: MutableSet<String> = mutableSetOf()

    /** Each machine's sessions, by machine id, as [listSessions] answers them. */
    val sessions: MutableMap<String, MutableList<RemoteSession>> = mutableMapOf()

    /** Each machine's last listing, by machine id: what [listSessions] or [createSession] last answered. */
    private val listings = mutableMapOf<String, List<RemoteSession>>()

    /** What [showPane] returns. */
    var paneFactory: (machineId: String, sessionId: String, paneId: String, size: PaneSize) -> Pane =
        { _, _, _, _ -> Pane(NoHandle) }

    /** Makes the next call of [method] (its Kotlin name, `connect` say) throw [error]. */
    fun failNext(
        method: String,
        error: MobileException,
    ) {
        failures[method] = error
    }

    /** Hands [event] to the sink, as the core's actor would. */
    fun emit(event: MobileEvent) {
        sink?.onEvent(event)
    }

    private fun record(
        method: String,
        vararg args: Any?,
    ) {
        recorded += FakeCall(method, args.toList())
        failures.remove(method)?.let { throw it }
    }

    private fun sessionsOf(machineId: String): MutableList<RemoteSession> {
        if (machineId !in connected) throw MobileException.NotConnected(machineId)
        return sessions.getOrPut(machineId) { mutableListOf() }
    }

    private fun listed(
        machineId: String,
        sessionId: String,
    ): RemoteSession {
        sessionsOf(machineId)
        return listings[machineId]?.firstOrNull { it.id == sessionId } ?: throw MobileException.NotListed(sessionId)
    }

    /** Keeps the machine's sessions as its listing and delivers them, as the real core does with a fresh listing. */
    private fun keepListing(machineId: String): List<RemoteSession> {
        val listing = sessionsOf(machineId).toList()
        listings[machineId] = listing
        emit(MobileEvent.Tree(machineId, listing))
        return listing
    }

    override suspend fun connect(machine: Machine) {
        record("connect", machine)
        if (connected.add(machine.id)) emit(MobileEvent.MachineState(machine.id, MachineState.Connected))
    }

    override fun disconnect(machineId: String) {
        record("disconnect", machineId)
        connected -= machineId
        listings -= machineId
    }

    override suspend fun listSessions(machineId: String): List<RemoteSession> {
        record("listSessions", machineId)
        return keepListing(machineId)
    }

    override suspend fun createSession(
        machineId: String,
        name: String?,
    ): RemoteSession {
        record("createSession", machineId, name)
        created += 1
        val session = fakeSession(id = "fake-$created", name = name?.takeIf { it.isNotBlank() } ?: "session $created")
        sessionsOf(machineId) += session
        keepListing(machineId)
        return session
    }

    override suspend fun endSession(
        machineId: String,
        sessionId: String,
    ) {
        record("endSession", machineId, sessionId)
        val list = sessionsOf(machineId)
        if (!list.removeAll { it.id == sessionId }) throw MobileException.Remote("no session $sessionId")
        val listing = listings[machineId] ?: return
        listings[machineId] = listing.filter { it.id != sessionId }.also { emit(MobileEvent.Tree(machineId, it)) }
    }

    override suspend fun attachSession(
        machineId: String,
        sessionId: String,
    ) {
        record("attachSession", machineId, sessionId)
        listed(machineId, sessionId)
    }

    override fun detachSession(
        machineId: String,
        sessionId: String,
    ) = record("detachSession", machineId, sessionId)

    override suspend fun showPane(
        machineId: String,
        sessionId: String,
        paneId: String,
        size: PaneSize,
    ): Pane {
        record("showPane", machineId, sessionId, paneId, size)
        listed(machineId, sessionId)
        return paneFactory(machineId, sessionId, paneId, size)
    }

    override fun hidePane(
        machineId: String,
        sessionId: String,
        paneId: String,
    ) = record("hidePane", machineId, sessionId, paneId)

    override fun takeLead(
        machineId: String,
        sessionId: String,
        paneId: String,
    ) = record("takeLead", machineId, sessionId, paneId)

    override fun answerAsk(
        machineId: String,
        sessionId: String,
        askId: String,
        reply: AskReply,
    ) = record("answerAsk", machineId, sessionId, askId, reply)

    override fun answerClipboardRead(
        machineId: String,
        sessionId: String,
        paneId: String,
        readId: ULong,
        reply: ClipboardReply,
    ) = record("answerClipboardRead", machineId, sessionId, paneId, readId, reply)

    override fun answerHostKey(
        machineId: String,
        fingerprint: String,
        trust: Boolean,
    ) = record("answerHostKey", machineId, fingerprint, trust)

    override fun networkChanged() = record("networkChanged")

    override fun suspend() = record("suspend")

    override fun resume() = record("resume")
}

/** A session as a machine's tree lists it, with what a test doesn't care about left plain. */
fun fakeSession(
    id: String,
    name: String = id,
    parent: String? = null,
    paneCount: UInt = 1u,
): RemoteSession =
    RemoteSession(
        id = id,
        name = name,
        windowName = "main",
        workspaceName = "default",
        context = null,
        cwd = "/home/fake",
        status = null,
        statusColor = null,
        statusShape = null,
        parent = parent,
        paneCount = paneCount,
        splitAxis = null,
    )
