package com.nickalie.ngterm.android.data

import com.nickalie.ngterm.core.ffi.PeerCapabilities
import com.nickalie.ngterm.core.ffi.PeerKind

/**
 * The app a machine runs, as its last listing said ([com.nickalie.ngterm.core.ffi.RemoteSession.peer]), and what that
 * app [offers] (the core's `peer_capabilities`). A machine with none known runs ngterm, which offers everything.
 */
data class MachinePeer(
    val kind: PeerKind,
    val offers: PeerCapabilities,
) {
    /** Whether the machine runs another app than ngterm, which the screens say with a badge and its [note]. */
    val foreign: Boolean get() = kind != PeerKind.NGTERM

    /** What the user misses on it, as one sentence; `null` for ngterm. */
    val note: String? get() = offers.note
}

/** Whether a machine with this peer (none known: ngterm) has the agent chat view, its commands and uploads. */
val MachinePeer?.offersChat: Boolean get() = this?.offers?.agentChat ?: true

/** The badge's word for a peer that isn't ngterm. */
val MachinePeer.badge: String
    get() =
        when (kind) {
            PeerKind.NGTERM -> "ngterm"
            PeerKind.AGTERM -> "agterm"
        }
