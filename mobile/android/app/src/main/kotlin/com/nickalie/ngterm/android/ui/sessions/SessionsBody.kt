package com.nickalie.ngterm.android.ui.sessions

import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.core.ffi.RemoteSession

/** Where the far side's setup is explained: the README's section on running ngterm on a server. */
const val SERVER_SETUP_URL = "https://github.com/nickalie/ngterm#a-server-without-a-display"

// the CLI every remote call runs (ngterm-core's REMOTE_CLI)
private const val REMOTE_CLI = "ngtermctl"

/**
 * Whether [reason] is the far shell saying it has no `ngtermctl` (`/bin/sh: 1: ngtermctl: not found`, bash's
 * `command not found`, env's `No such file or directory`): the core passes the shell's own words on.
 */
fun isMissingCli(reason: String): Boolean {
    val text = reason.lowercase()
    return REMOTE_CLI in text && ("not found" in text || "no such file" in text)
}

/** A session as the list shows it: the session, its parent's name when it is a companion, and what is under way. */
data class SessionLine(
    val session: RemoteSession,
    val companionOf: String?,
    val attached: Boolean,
    val ending: Boolean,
)

/** What the body of a machine's sessions screen is, by its link and its listing. */
sealed interface SessionsBody {
    /** Connecting, or waiting for the host-key answer the sheet over the app asks for. */
    data object Connecting : SessionsBody

    /** Connected; the first listing is on its way. */
    data object Reading : SessionsBody

    /** Not connected and not trying; [deviceOffline] when the device has no network either. */
    data class NotConnected(
        val deviceOffline: Boolean,
    ) : SessionsBody

    /** The connection dropped; the core tries again at [retryAt]. */
    data class Reconnecting(
        val retryAt: Long,
    ) : SessionsBody

    /** The machine can't be reached, for [reason]. */
    data class Unreachable(
        val reason: String,
    ) : SessionsBody

    /** The machine's host key changed: nothing connects until it is reviewed. */
    data object HostKeyChanged : SessionsBody

    /** The machine has no `ngtermctl` on the PATH of ssh logins. */
    data class MissingCli(
        val reason: String,
    ) : SessionsBody

    /** The sessions could not be listed, for [reason], and there is no earlier listing. */
    data class ListFailed(
        val reason: String,
    ) : SessionsBody

    /** The machine has no sessions. [failure] is why the last refresh failed, if it did. */
    data class Empty(
        val failure: String?,
    ) : SessionsBody

    /**
     * The sessions, by family (a parent, then its companions right under it), in the machine's order of parents: the
     * families with a session this app has [attached], then the [others]. [failure] is why the last refresh failed, if
     * it did (the list is the one before).
     */
    data class Sessions(
        val attached: List<SessionLine>,
        val others: List<SessionLine>,
        val failure: String?,
    ) : SessionsBody
}

/** The body for these [MachineSessions]. */
fun MachineSessions.body(): SessionsBody =
    when (link) {
        Link.Connected -> listingBody()
        Link.Offline -> SessionsBody.NotConnected(deviceOffline = offline)
        Link.Connecting, is Link.HostKeyUnknown -> SessionsBody.Connecting
        is Link.Reconnecting -> SessionsBody.Reconnecting(link.retryAt)
        is Link.Unreachable -> SessionsBody.Unreachable(link.reason)
        is Link.HostKeyChanged -> SessionsBody.HostKeyChanged
    }

private fun MachineSessions.listingBody(): SessionsBody {
    val listed = sessions
    return when {
        listed == null && failure != null && isMissingCli(failure) -> SessionsBody.MissingCli(failure)
        listed == null && failure != null -> SessionsBody.ListFailed(failure)
        listed == null -> SessionsBody.Reading
        listed.isEmpty() -> SessionsBody.Empty(failure)
        else -> lines(listed)
    }
}

private fun MachineSessions.lines(listed: List<RemoteSession>): SessionsBody.Sessions {
    val names = listed.associate { it.id to it.name }
    val line = { session: RemoteSession ->
        SessionLine(
            session = session,
            companionOf = session.parent?.let { names[it] ?: it },
            attached = session.id in attached,
            ending = session.id in ending,
        )
    }
    val families = families(listed).map { family -> family.map(line) }
    val (mine, others) = families.partition { family -> family.any { it.attached } }
    return SessionsBody.Sessions(mine.flatten(), others.flatten(), failure)
}

/**
 * [listed] by family: each session whose parent isn't listed (the core lists parents first, companions after them
 * all) heads one, in the listing's order, followed by its companions and theirs.
 */
private fun families(listed: List<RemoteSession>): List<List<RemoteSession>> {
    val ids = listed.mapTo(mutableSetOf()) { it.id }
    val companions = listed.filter { it.parent in ids }.groupBy { it.parent }
    val placed = mutableSetOf<String>()

    fun family(head: RemoteSession): List<RemoteSession> =
        if (placed.add(head.id)) listOf(head) + companions[head.id].orEmpty().flatMap(::family) else emptyList()

    val heads = listed.filter { it.parent !in ids }.map(::family)
    // a loop of parents has no head: its sessions stand alone, so none goes missing
    val strays = listed.filter { it.id !in placed }.map { listOf(it) }
    return heads + strays
}

/** What a machine's sessions screen does, as [SessionsViewModel] and the route do it. */
data class SessionActions(
    val onBack: () -> Unit,
    val onNewSession: () -> Unit,
    val onRefresh: () -> Unit,
    val onConnect: () -> Unit,
    val onReviewHostKey: () -> Unit,
    val onOpenReadme: () -> Unit,
    val onOpen: (String) -> Unit,
    val onDetach: (String) -> Unit,
    val onEnd: (String) -> Unit,
)
