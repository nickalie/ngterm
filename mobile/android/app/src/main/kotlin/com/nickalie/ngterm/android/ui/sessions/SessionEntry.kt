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
import com.nickalie.ngterm.android.ui.components.OutlineTag
import com.nickalie.ngterm.android.ui.components.SessionRow
import com.nickalie.ngterm.android.ui.components.StatusBadge
import com.nickalie.ngterm.android.ui.session.ChatViewItem
import com.nickalie.ngterm.android.ui.session.DuplicateItem
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone
import com.nickalie.ngterm.core.ffi.RemoteSession

/**
 * One session's row on [machine]: its name, its context (or its working directory) or the parent it is a companion
 * of, its agent's status, and a menu with Attach or Detach, Duplicate session, Chat view, Rename session and End
 * session ([onAskEnd]: the screen confirms it). Tapping it attaches it (the session view opens from there once it
 * lands). While its End or its Duplicate is under way it says so instead of its status. An ended session says "Ended
 * on build-01" and a page row its kind; neither takes a tap, and their menus hold only Rename session and End session
 * (and Detach while an ended session is still attached).
 */
@Composable
internal fun SessionEntry(
    line: SessionLine,
    machine: String,
    actions: SessionActions,
    onAskEnd: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val session = line.session
    val page = session.page != null
    SessionRow(
        name = session.name,
        onClick = { actions.onOpen(session.id) },
        modifier = modifier,
        context =
            if (page) {
                stringResource(R.string.machine_sessions_page_note)
            } else {
                session.context?.takeIf { it.isNotBlank() } ?: session.cwd.takeIf { it.isNotBlank() }
            },
        enabled = session.attachable,
        // the Attached group's heading says it: the mark would only crowd the row
        attached = false,
        companionOf = line.companionOf,
        peer = line.peer,
        menu = {
            SessionMenu(
                name = session.name,
                attachable = session.attachable,
                attached = line.attached,
                ending = line.ending,
                onAttach = { actions.onOpen(session.id) },
                onDetach = { actions.onDetach(session.id) },
                canDuplicate = line.canDuplicate,
                onDuplicate = { actions.onDuplicate(session.id) },
                chatView = line.chatView,
                noChatOn = line.noChatOn,
                onChatView = { on -> actions.onChatView(session.id, on) },
                onRename = { actions.onRename(session.id) },
                onEnd = onAskEnd,
            )
        },
    ) {
        when {
            line.ending -> {
                UnderWay(stringResource(R.string.machine_sessions_ending))
            }

            line.duplicating -> {
                UnderWay(stringResource(R.string.machine_sessions_duplicating))
            }

            session.ended -> {
                StatusBadge(StatusTone.Offline, label = stringResource(R.string.machine_sessions_ended, machine))
            }

            page -> {
                OutlineTag(pageKind(session))
            }

            !session.status.isNullOrBlank() -> {
                AgentStatusBadge(session.status.orEmpty(), session.statusColor, session.statusShape)
            }
        }
    }
}

@Composable
private fun pageKind(session: RemoteSession): String =
    stringResource(
        when (session.page) {
            "file" -> R.string.machine_sessions_page_file
            "url" -> R.string.machine_sessions_page_url
            else -> R.string.machine_sessions_page
        },
    )

// what the row is doing in place of its status: a small spinner and the word for it
@Composable
internal fun UnderWay(
    label: String,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier.semantics { liveRegion = LiveRegionMode.Polite },
        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        CircularProgressIndicator(modifier = Modifier.size(12.dp), strokeWidth = 2.dp)
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun SessionMenu(
    name: String,
    attachable: Boolean,
    attached: Boolean,
    ending: Boolean,
    onAttach: () -> Unit,
    onDetach: () -> Unit,
    canDuplicate: Boolean,
    onDuplicate: () -> Unit,
    chatView: Boolean,
    noChatOn: String?,
    onChatView: (Boolean) -> Unit,
    onRename: () -> Unit,
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
            if (attachable || attached) {
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
            }
            if (attachable) {
                DuplicateItem(enabled = canDuplicate && !ending) {
                    open = false
                    onDuplicate()
                }
                ChatViewItem(chatView, noChatOn) { on ->
                    open = false
                    onChatView(on)
                }
            }
            DropdownMenuItem(
                text = { Text(stringResource(R.string.machine_sessions_rename)) },
                enabled = !ending,
                onClick = {
                    open = false
                    onRename()
                },
            )
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
