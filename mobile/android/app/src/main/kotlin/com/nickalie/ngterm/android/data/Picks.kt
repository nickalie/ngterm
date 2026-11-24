package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.PickReply
import com.nickalie.ngterm.core.ffi.PickRow
import com.nickalie.ngterm.core.ffi.RemotePick
import com.nickalie.ngterm.core.ffi.RemotePickItem
import com.nickalie.ngterm.core.ffi.pickRows
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.getAndUpdate
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch

/** A pick an attached session's origin bound to it (`ngtermctl pick --session`), waiting for this app's answer. */
data class OpenPick(
    val key: SessionKey,
    val pick: RemotePick,
)

/** Ranks a pick's items for the search text, as desktop's picker does (the core's [pickRows]). */
typealias PickRanker = (items: List<RemotePickItem>, query: String, allowCustom: Boolean) -> List<PickRow>

/**
 * The picks the attached sessions' origins handed over, open in [open] until they are answered here or the core takes
 * them down (answered elsewhere, their timeout ran out on the origin, the stream gone), or their session is detached.
 * One shows as a sheet while its session is on screen ([Asks.onScreen]) and no ask of it is open; out of sight it
 * waits. Events arrive on the core's thread; [open] takes them from any.
 */
class Picks(
    scope: CoroutineScope,
    private val connections: Connections,
    private val ranker: PickRanker = ::pickRows,
) {
    private val state = MutableStateFlow<List<OpenPick>>(emptyList())

    /** The open picks, oldest first. */
    val open: StateFlow<List<OpenPick>> = state.asStateFlow()

    init {
        connections.listen(::onEvent)
        scope.launch {
            var before = emptySet<SessionKey>()
            connections.attached.collect { attached ->
                val now = attached.flatMap { (machine, ids) -> ids.map { SessionKey(machine, it) } }.toSet()
                val gone = before - now
                before = now
                if (gone.isNotEmpty()) state.update { picks -> picks.filterNot { it.key in gone } }
            }
        }
    }

    private fun onEvent(event: MobileEvent) {
        when (event) {
            is MobileEvent.Pick -> {
                val key = SessionKey(event.machineId, event.sessionId)
                // a session holds one pick at a time; one with the same id again is the same pick, restated
                state.update { picks ->
                    picks.filterNot { it.key == key && it.pick.id == event.pick.id } + OpenPick(key, event.pick)
                }
            }

            is MobileEvent.PickDismiss -> {
                remove(SessionKey(event.machineId, event.sessionId), event.pickId)
            }

            else -> {}
        }
    }

    /** The rows [pick] shows for [query]. */
    fun rows(
        pick: RemotePick,
        query: String,
    ): List<PickRow> = ranker(pick.items, query, pick.allowCustom)

    /**
     * Answers the open pick [pickId] of [key] with [reply], once: a pick no longer open is left be (the origin took it
     * back). True when it was open.
     */
    fun answer(
        key: SessionKey,
        pickId: String,
        reply: PickReply,
    ): Boolean {
        if (!remove(key, pickId)) return false
        try {
            connections.core.answerPick(key.machineId, key.sessionId, pickId, reply)
        } catch (_: MobileException) {
            // the core has stopped: nothing waits for the answer any more
        }
        return true
    }

    private fun remove(
        key: SessionKey,
        pickId: String,
    ): Boolean {
        val before = state.getAndUpdate { picks -> picks.filterNot { it.key == key && it.pick.id == pickId } }
        return before.any { it.key == key && it.pick.id == pickId }
    }
}
