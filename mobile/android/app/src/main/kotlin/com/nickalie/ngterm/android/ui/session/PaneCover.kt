package com.nickalie.ngterm.android.ui.session

import com.nickalie.ngterm.android.data.PaneLive
import com.nickalie.ngterm.core.ffi.PaneEnd
import com.nickalie.ngterm.core.ffi.PaneRole

/** Why a pane can't be used as it is, and so what covers it. */
sealed interface PaneCover {
    /** Another client leads: its grid and keys are the other device's. */
    data object Follower : PaneCover

    /** No screen yet: showing, or attaching again. */
    data object Attaching : PaneCover

    /** The link dropped; the core attaches it again on its own. The last screen stays readable. */
    data object Reconnecting : PaneCover

    /** The program exited with [code] or by [signal]. */
    data class Exited(
        val code: UInt?,
        val signal: String?,
    ) : PaneCover

    /** The pane could not be shown, for [reason] (the core's words). */
    data class Failed(
        val reason: String,
    ) : PaneCover

    companion object {
        /**
         * What covers a pane: its show [failure], no terminal yet ([hasTerminal] false), its attach's end, or its
         * role's cover; `null` when it is the user's to use.
         */
        fun of(
            live: PaneLive?,
            hasTerminal: Boolean,
            failure: String?,
        ): PaneCover? {
            val end = live?.end
            return when {
                failure != null -> Failed(failure)
                !hasTerminal -> Attaching
                end is PaneEnd.Lost -> Reconnecting
                end is PaneEnd.Exited -> Exited(end.code, end.signal)
                live?.covered == true && live.role == PaneRole.FOLLOWER -> Follower
                live?.covered == true -> Attaching
                else -> null
            }
        }
    }
}

/**
 * Whether [cover] hides the pane from TalkBack (all but [PaneCover.Reconnecting], whose last screen stays readable).
 */
val PaneCover?.veils: Boolean get() = this != null && this != PaneCover.Reconnecting
