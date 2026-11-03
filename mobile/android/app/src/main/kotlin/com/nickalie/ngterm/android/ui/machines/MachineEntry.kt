package com.nickalie.ngterm.android.ui.machines

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.style.TextOverflow
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.ui.components.AgentStatusBadge
import com.nickalie.ngterm.android.ui.components.MachineCard
import com.nickalie.ngterm.android.ui.components.StatusBadge
import com.nickalie.ngterm.android.ui.components.agentStatusWord
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone
import com.nickalie.ngterm.core.ffi.RemoteSession
import kotlinx.coroutines.delay

private const val SECOND_MS = 1000L

/** One machine's card: its link as the badge, and under it what the link calls for. Tapping it opens its sessions. */
@Composable
internal fun MachineEntry(
    item: MachineItem,
    actions: MachineActions,
    now: () -> Long,
    modifier: Modifier = Modifier,
) {
    val id = item.machine.id
    val link = item.link
    MachineCard(
        label = item.machine.title,
        address = item.machine.address,
        connection = link.tone(),
        onClick = { actions.onOpen(id) },
        modifier = modifier,
        connectionLabel = connectionLabel(link, now),
        menu = { MachineMenu(item.machine.title, link, id, actions) },
    ) {
        Summary(item, actions)
    }
}

private fun Link.tone(): StatusTone =
    when (this) {
        Link.Offline -> StatusTone.Offline
        Link.Connecting, is Link.Reconnecting -> StatusTone.Connecting
        Link.Connected -> StatusTone.Online
        is Link.HostKeyUnknown, is Link.SignIn -> StatusTone.Attention
        is Link.Unreachable, is Link.HostKeyChanged -> StatusTone.Error
    }

@Composable
private fun connectionLabel(
    link: Link,
    now: () -> Long,
): String? =
    when (link) {
        is Link.Reconnecting -> {
            val seconds = countdown(link.retryAt, now)
            if (seconds > 0) {
                stringResource(R.string.machine_reconnecting, seconds)
            } else {
                stringResource(R.string.machine_reconnecting_now)
            }
        }

        is Link.Unreachable -> {
            stringResource(R.string.machine_unreachable)
        }

        is Link.HostKeyChanged -> {
            stringResource(R.string.machine_key_changed)
        }

        is Link.SignIn -> {
            stringResource(R.string.machine_signing_in)
        }

        else -> {
            null
        }
    }

/** Whole seconds left until [at] by [now], ticking once a second until it is reached. */
@Composable
internal fun countdown(
    at: Long,
    now: () -> Long,
): Int {
    val clock by rememberUpdatedState(now)
    var current by remember(at) { mutableLongStateOf(now()) }
    LaunchedEffect(at) {
        val ticks = ((at - current + SECOND_MS - 1) / SECOND_MS).coerceAtLeast(0)
        repeat(ticks.toInt()) {
            delay(SECOND_MS)
            current = clock()
        }
    }
    return ((at - current + SECOND_MS - 1) / SECOND_MS).coerceAtLeast(0).toInt()
}

@Composable
private fun Summary(
    item: MachineItem,
    actions: MachineActions,
) {
    val id = item.machine.id
    when (val link = item.link) {
        Link.Offline -> {
            NoteAndAction(
                note = stringResource(R.string.machine_not_connected),
                action = stringResource(R.string.machine_connect),
                onAction = { actions.onConnect(id) },
            )
        }

        Link.Connected -> {
            item.sessions?.let { Sessions(it) }
        }

        // nothing to do while it connects: the badge says so
        Link.Connecting -> {}

        is Link.Reconnecting -> {
            NoteAndAction(
                note = stringResource(R.string.machine_connection_lost),
                action = stringResource(R.string.machine_retry_now),
                onAction = { actions.onConnect(id) },
            )
        }

        is Link.Unreachable -> {
            NoteAndAction(link.reason, stringResource(R.string.action_retry), onAction = { actions.onConnect(id) })
        }

        is Link.HostKeyUnknown -> {
            // the question itself is the sheet over the app, open while the connect waits
            Note(stringResource(R.string.machine_confirm_key))
        }

        is Link.SignIn -> {
            // the link and its Open link are the sheet over the app, open while the connect waits
            Note(stringResource(R.string.machine_sign_in_summary))
        }

        is Link.HostKeyChanged -> {
            NoteAndAction(
                note = stringResource(R.string.machine_key_changed_summary),
                action = stringResource(R.string.machine_review_key),
                onAction = { actions.onReviewHostKey(id) },
            )
        }
    }
}

// what the state means, and the one action it calls for at the end of the line
@Composable
private fun NoteAndAction(
    note: String,
    action: String,
    onAction: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(modifier = modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Note(note, Modifier.weight(1f).padding(end = NgtermTheme.spacing.sm))
        TextButton(onClick = onAction) { Text(action) }
    }
}

/** The sessions' aggregate status: a badge per status the origin reports, with how many sessions have it. */
@Composable
private fun Sessions(sessions: List<RemoteSession>) {
    if (sessions.isEmpty()) {
        Note(stringResource(R.string.machine_no_sessions))
        return
    }
    val (withStatus, idle) = sessions.partition { !it.status.isNullOrBlank() }
    withStatus.groupBy { it.status.orEmpty() }.forEach { (status, group) ->
        val first = group.first()
        AgentStatusBadge(
            status = status,
            colorHex = first.statusColor,
            shape = first.statusShape,
            label = stringResource(R.string.machine_status_count, group.size, agentStatusWord(status).lowercase()),
        )
    }
    if (idle.isNotEmpty()) {
        StatusBadge(
            StatusTone.Idle,
            label = pluralStringResource(R.plurals.machine_idle_sessions, idle.size, idle.size),
        )
    }
}

@Composable
private fun Note(
    text: String,
    modifier: Modifier = Modifier,
) {
    Box(modifier = modifier, contentAlignment = Alignment.CenterStart) {
        Text(
            text = text,
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 3,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@Composable
private fun MachineMenu(
    title: String,
    link: Link,
    id: String,
    actions: MachineActions,
    modifier: Modifier = Modifier,
) {
    var open by remember { mutableStateOf(false) }
    Box(modifier = modifier) {
        IconButton(onClick = { open = true }) {
            Icon(
                painterResource(R.drawable.ic_more),
                contentDescription = stringResource(R.string.action_more, title),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            val connected =
                link == Link.Connected || link is Link.Reconnecting || link == Link.Connecting || link is Link.SignIn
            DropdownMenuItem(
                text = {
                    Text(
                        stringResource(if (connected) R.string.machine_disconnect else R.string.machine_connect),
                    )
                },
                onClick = {
                    open = false
                    if (connected) actions.onDisconnect(id) else actions.onConnect(id)
                },
            )
            DropdownMenuItem(
                text = { Text(stringResource(R.string.machine_edit)) },
                onClick = {
                    open = false
                    actions.onEdit(id)
                },
            )
            DropdownMenuItem(
                text = { Text(stringResource(R.string.machine_remove), color = MaterialTheme.colorScheme.error) },
                onClick = {
                    open = false
                    actions.onRemove(id)
                },
            )
        }
    }
}
