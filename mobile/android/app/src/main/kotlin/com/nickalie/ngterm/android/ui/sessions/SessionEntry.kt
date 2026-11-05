package com.nickalie.ngterm.android.ui.sessions

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.AgentStatusBadge
import com.nickalie.ngterm.android.ui.components.SessionRow
import com.nickalie.ngterm.android.ui.session.ChatViewItem
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * One session's row: its name, its context (or its working directory) or the parent it is a companion of, its agent's
 * status, and a menu with Attach or Detach, Chat view and End session ([onAskEnd]: the screen confirms it).
 * Tapping it attaches it (the session view opens from there once it lands). While its End is under way it says so
 * instead of its status.
 */
@Composable
internal fun SessionEntry(
    line: SessionLine,
    actions: SessionActions,
    onAskEnd: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val session = line.session
    SessionRow(
        name = session.name,
        onClick = { actions.onOpen(session.id) },
        modifier = modifier,
        context = session.context?.takeIf { it.isNotBlank() } ?: session.cwd.takeIf { it.isNotBlank() },
        // the Attached group's heading says it: the mark would only crowd the row
        attached = false,
        companionOf = line.companionOf,
        menu = {
            SessionMenu(
                name = session.name,
                attached = line.attached,
                ending = line.ending,
                onAttach = { actions.onOpen(session.id) },
                onDetach = { actions.onDetach(session.id) },
                chatView = line.chatView,
                onChatView = { on -> actions.onChatView(session.id, on) },
                onEnd = onAskEnd,
            )
        },
    ) {
        when {
            line.ending -> {
                Ending()
            }

            !session.status.isNullOrBlank() -> {
                AgentStatusBadge(session.status.orEmpty(), session.statusColor, session.statusShape)
            }
        }
    }
}

@Composable
private fun Ending(modifier: Modifier = Modifier) {
    Row(
        modifier = modifier.semantics { liveRegion = LiveRegionMode.Polite },
        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(modifier = Modifier.size(12.dp), strokeWidth = 2.dp)
        Text(
            text = stringResource(R.string.machine_sessions_ending),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun SessionMenu(
    name: String,
    attached: Boolean,
    ending: Boolean,
    onAttach: () -> Unit,
    onDetach: () -> Unit,
    chatView: Boolean,
    onChatView: (Boolean) -> Unit,
    onEnd: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var open by remember { mutableStateOf(false) }
    Box(modifier = modifier) {
        IconButton(onClick = { open = true }) {
            Icon(
                painterResource(R.drawable.ic_more),
                contentDescription = stringResource(R.string.action_more, name),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(
                text = {
                    Text(
                        stringResource(
                            if (attached) R.string.machine_sessions_detach else R.string.machine_sessions_attach,
                        ),
                    )
                },
                onClick = {
                    open = false
                    if (attached) onDetach() else onAttach()
                },
            )
            ChatViewItem(chatView) { on ->
                open = false
                onChatView(on)
            }
            DropdownMenuItem(
                text = { Text(stringResource(R.string.machine_sessions_end), color = MaterialTheme.colorScheme.error) },
                enabled = !ending,
                onClick = {
                    open = false
                    onEnd()
                },
            )
        }
    }
}
