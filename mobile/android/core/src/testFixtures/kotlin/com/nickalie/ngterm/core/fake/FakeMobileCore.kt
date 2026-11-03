package com.nickalie.ngterm.core.fake

import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.ClipboardReply
import com.nickalie.ngterm.core.ffi.EventSink
import com.nickalie.ngterm.core.ffi.HostKeyStore
import com.nickalie.ngterm.core.ffi.Machine
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileCoreInterface
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.NoHandle
import com.nickalie.ngterm.core.ffi.Pane
import com.nickalie.ngterm.core.ffi.PaneSize
import com.nickalie.ngterm.core.ffi.RemoteSession
import kotlinx.coroutines.CompletableDeferred

/** One call the app made on a [FakeMobileCore]: the method's name and its arguments, in order. */
data class FakeCall(
    val method: String,
    val args: List<Any?> = emptyList(),
)

/**
 * A [MobileCoreInterface] with no native library behind it, for the app's JVM and UI tests. It records every call in
 * [calls], keeps each machine's sessions in [sessions], hands [sink] the events a test [emit]s, and throws the
 * failure a test queued with [failNext] from that method's next call. A suspending method's next call can be held open
 * with [holds]: it is recorded and takes its queued failure at once, then waits for the gate before going on.
 *
 * It follows the real core's rules (crates/ngterm-mobile, api.rs and actor.rs) where the app sees them:
 * - [connect] reports [MachineState.Connected] as a [MobileEvent.MachineState] when the machine was not connected
 *   (or was connected as another record: an edited machine connects anew); a connect that fails reports
 *   [MachineState.Unreachable] with the failure's reason before it throws [MobileException.Unreachable];
 *   [disconnect] reports nothing and forgets the machine's listing;
 * - host keys, when the fake has [hosts] and [hostKeys] names the key a machine's host offers: a pinned key that
 *   matches connects; another pinned key reports [MobileEvent.HostKeyChanged] with both and fails ([HOST_KEY_CHANGED]);
 *   no pin reports [MobileEvent.HostKeyUnknown] and the connect waits for [answerHostKey], which pins the key through
 *   [hosts] and connects when it trusts the key on offer, and fails ([HOST_KEY_REFUSED]) otherwise; a [disconnect]
 *   while it waits cancels it ([MobileException.Cancelled]);
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
    private val hosts: HostKeyStore? = null,
) : MobileCoreInterface {
    private val recorded = mutableListOf<FakeCall>()
    private val records = mutableMapOf<String, Machine>()
    private val questions = mutableMapOf<String, Question>()

    private class Question(
        val offered: String,
        // trusted or not; null when a disconnect overtook the question
        val answer: CompletableDeferred<Boolean?> = CompletableDeferred(),
    )

    /** The host key each host offers (OpenSSH's `SHA256:` fingerprint), by host name; a host not here offers none. */
    val hostKeys: MutableMap<String, String> = mutableMapOf()

    /** The machines whose connect waits for [answerHostKey], by id, with the key on offer. */
    val waitingForHostKey: Map<String, String> get() = questions.mapValues { it.value.offered }
    private val failures = mutableMapOf<String, MobileException>()

    /**
     * Gates that hold the next call of a suspending method (its Kotlin name) open until they complete: the call is
     * recorded and takes the failure [failNext] queued for it before it waits, so a later call goes on unheld.
     */
    val holds: MutableMap<String, CompletableDeferred<Unit>> = mutableMapOf()
    private var created = 0

    /** The calls made so far, oldest first. */
    val calls: List<FakeCall> get() = synchronized(recorded) { recorded.toList() }

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
        // an app test may read the calls while its coroutines make them on other threads
        synchronized(recorded) { recorded += FakeCall(method, args.toList()) }
        failures.remove(method)?.let { throw it }
    }

    // [record] for a suspending method, held open while a test's gate in [holds] is
    private suspend fun recordHeld(
        method: String,
        vararg args: Any?,
    ) {
        synchronized(recorded) { recorded += FakeCall(method, args.toList()) }
        val failure = failures.remove(method)
        holds.remove(method)?.await()
        failure?.let { throw it }
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
        try {
            recordHeld("connect", machine)
        } catch (error: MobileException.Unreachable) {
            unreachable(machine.id, error.reason)
        }
        val id = machine.id
        if (id in connected && records[id] == machine) return
        drop(id)
        checkHostKey(machine)
        connected += id
        records[id] = machine
        emit(MobileEvent.MachineState(id, MachineState.Connected))
    }

    private fun unreachable(
        machineId: String,
        reason: String,
    ): Nothing {
        emit(MobileEvent.MachineState(machineId, MachineState.Unreachable(reason)))
        throw MobileException.Unreachable(reason)
    }

    private suspend fun checkHostKey(machine: Machine) {
        val store = hosts ?: return
        val offered = hostKeys[machine.host] ?: return
        val pinned = store.pinned(machine.id)
        when {
            // pinned as offered: the handshake goes on
            pinned == offered -> {}

            pinned != null -> {
                emit(MobileEvent.HostKeyChanged(machine.id, pinned, offered))
                unreachable(machine.id, HOST_KEY_CHANGED)
            }

            else -> {
                val question = Question(offered)
                questions[machine.id] = question
                emit(MobileEvent.HostKeyUnknown(machine.id, offered))
                val trusted =
                    try {
                        question.answer.await()
                    } finally {
                        if (questions[machine.id] === question) questions -= machine.id
                    }
                if (trusted == null) throw MobileException.Cancelled()
                if (!trusted) unreachable(machine.id, HOST_KEY_REFUSED)
                store.pin(machine.id, offered)
            }
        }
    }

    private fun drop(machineId: String) {
        connected -= machineId
        records -= machineId
        listings -= machineId
    }

    override fun disconnect(machineId: String) {
        record("disconnect", machineId)
        drop(machineId)
        questions.remove(machineId)?.answer?.complete(null)
    }

    override suspend fun listSessions(machineId: String): List<RemoteSession> {
        recordHeld("listSessions", machineId)
        return keepListing(machineId)
    }

    override suspend fun createSession(
        machineId: String,
        name: String?,
    ): RemoteSession {
        recordHeld("createSession", machineId, name)
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
        recordHeld("endSession", machineId, sessionId)
        val list = sessionsOf(machineId)
        if (!list.removeAll { it.id == sessionId }) throw MobileException.Remote("no session $sessionId")
        val listing = listings[machineId] ?: return
        listings[machineId] = listing.filter { it.id != sessionId }.also { emit(MobileEvent.Tree(machineId, it)) }
    }

    override suspend fun attachSession(
        machineId: String,
        sessionId: String,
    ) {
        recordHeld("attachSession", machineId, sessionId)
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
        recordHeld("showPane", machineId, sessionId, paneId, size)
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
    ) {
        record("answerHostKey", machineId, fingerprint, trust)
        // an answer about another key than the one on offer refuses: the user never saw that key
        val question = questions.remove(machineId) ?: return
        question.answer.complete(trust && fingerprint == question.offered)
    }

    override fun networkChanged() = record("networkChanged")

    override fun suspend() = record("suspend")

    override fun resume() = record("resume")
}

/** The core's reason for a connect refused over a changed host key (transport/russh.rs's `HOST_KEY_CHANGED`). */
const val HOST_KEY_CHANGED = "the host key changed"

/** The core's reason for a connect whose unknown host key was not trusted (`HOST_KEY_REFUSED`). */
const val HOST_KEY_REFUSED = "the host key was not trusted"

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
