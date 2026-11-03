package com.nickalie.ngterm.core.fake

import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.ClipboardReply
import com.nickalie.ngterm.core.ffi.EventSink
import com.nickalie.ngterm.core.ffi.Machine
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
    ): RemoteSession =
        sessionsOf(machineId).firstOrNull { it.id == sessionId } ?: throw MobileException.NotListed(sessionId)

    override suspend fun connect(machine: Machine) {
        record("connect", machine)
        connected += machine.id
    }

    override fun disconnect(machineId: String) {
        record("disconnect", machineId)
        connected -= machineId
    }

    override suspend fun listSessions(machineId: String): List<RemoteSession> {
        record("listSessions", machineId)
        return sessionsOf(machineId).toList()
    }

    override suspend fun createSession(
        machineId: String,
        name: String?,
    ): RemoteSession {
        record("createSession", machineId, name)
        created += 1
        val session = fakeSession(id = "fake-$created", name = name ?: "session $created")
        val list = sessionsOf(machineId)
        list += session
        emit(MobileEvent.Tree(machineId, list.toList()))
        return session
    }

    override suspend fun endSession(
        machineId: String,
        sessionId: String,
    ) {
        record("endSession", machineId, sessionId)
        val list = sessionsOf(machineId)
        list.remove(listed(machineId, sessionId))
        emit(MobileEvent.Tree(machineId, list.toList()))
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
