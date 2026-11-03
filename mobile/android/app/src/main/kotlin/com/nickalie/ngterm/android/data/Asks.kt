package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.RemoteAsk
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.getAndUpdate
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** An ask an attached session's origin handed over, waiting for this app's answer. */
data class OpenAsk(
    val key: SessionKey,
    val ask: RemoteAsk,
)

/**
 * The asks the attached sessions' origins handed over (the ask rule: any attached session's), open in [open] until
 * they are answered here or the core takes them down (answered elsewhere, ended on the origin, the stream gone), or
 * their session is detached. Where one shows is the ask rule's: a dialog while its session is [onScreen], otherwise a
 * notification. Events arrive on the core's thread; [open] takes them from any.
 */
class Asks(
    scope: CoroutineScope,
    private val connections: Connections,
) {
    private val state = MutableStateFlow<List<OpenAsk>>(emptyList())
    private val screen = MutableStateFlow<SessionKey?>(null)

    /** The open asks, oldest first. */
    val open: StateFlow<List<OpenAsk>> = state.asStateFlow()

    /** The session the app shows right now (its view open and the app in the foreground); `null` for none. */
    val onScreen: StateFlow<SessionKey?> = screen.asStateFlow()

    init {
        connections.listen(::onEvent)
        scope.launch {
            var before = emptySet<SessionKey>()
            connections.attached.collect { attached ->
                val now = attached.flatMap { (machine, ids) -> ids.map { SessionKey(machine, it) } }.toSet()
                val gone = before - now
                before = now
                if (gone.isNotEmpty()) state.update { asks -> asks.filterNot { it.key in gone } }
            }
        }
    }

    private fun onEvent(event: MobileEvent) {
        when (event) {
            is MobileEvent.Ask -> {
                val key = SessionKey(event.machineId, event.sessionId)
                // a session holds one ask at a time; one with the same id again is the same ask, restated
                state.update { asks ->
                    asks.filterNot { it.key == key && it.ask.id == event.ask.id } +
                        OpenAsk(key, event.ask)
                }
            }

            is MobileEvent.AskDismiss -> {
                remove(SessionKey(event.machineId, event.sessionId), event.askId)
            }

            else -> {}
        }
    }

    /** The app now shows [key]'s session ([key] `null`: none, or the app went to the background). */
    fun showing(key: SessionKey?) {
        screen.value = key
    }

    /** The open ask [askId] of [key], if it is still open. */
    fun find(
        key: SessionKey,
        askId: String,
    ): OpenAsk? = state.value.firstOrNull { it.key == key && it.ask.id == askId }

    /**
     * Answers the open ask [askId] of [key] with [reply], once: an ask no longer open is left be (the origin took it
     * back). True when it was open.
     */
    fun answer(
        key: SessionKey,
        askId: String,
        reply: AskReply,
    ): Boolean {
        if (!remove(key, askId)) return false
        try {
            connections.core.answerAsk(key.machineId, key.sessionId, askId, reply)
        } catch (_: MobileException) {
            // the core has stopped: nothing waits for the answer any more
        }
        return true
    }

    private fun remove(
        key: SessionKey,
        askId: String,
    ): Boolean {
        val before = state.getAndUpdate { asks -> asks.filterNot { it.key == key && it.ask.id == askId } }
        return before.any { it.key == key && it.ask.id == askId }
    }
}
