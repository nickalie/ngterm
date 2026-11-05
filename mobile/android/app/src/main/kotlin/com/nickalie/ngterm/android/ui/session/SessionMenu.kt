package com.nickalie.ngterm.android.ui.session

import androidx.compose.foundation.layout.Box
import androidx.compose.material3.Checkbox
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.toggleableState
import androidx.compose.ui.state.ToggleableState
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.ConfirmDialog

/**
 * An attached session's More menu, named for the session [name]: Detach ([onDetach]), Chat view ([chatView], flipped
 * through [onChatView]) and End session ([onEnd]).
 */
@Composable
internal fun SessionMenu(
    name: String,
    chatView: Boolean,
    onDetach: () -> Unit,
    onChatView: (Boolean) -> Unit,
    onEnd: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var open by rememberSaveable { mutableStateOf(false) }
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
                text = { Text(stringResource(R.string.machine_sessions_detach)) },
                onClick = {
                    open = false
                    onDetach()
                },
            )
            ChatViewItem(chatView) { on ->
                open = false
                onChatView(on)
            }
            DropdownMenuItem(
                text = { Text(stringResource(R.string.machine_sessions_end), color = MaterialTheme.colorScheme.error) },
                onClick = {
                    open = false
                    onEnd()
                },
            )
        }
    }
}

/**
 * A session's Chat view item, checked while its panes running an agent show as a chat (Settings' Agent chat view, or
 * the session's own choice); a tap asks for the other ([onChatView]).
 */
@Composable
internal fun ChatViewItem(
    on: Boolean,
    onChatView: (Boolean) -> Unit,
) {
    DropdownMenuItem(
        text = { Text(stringResource(R.string.session_chat_view)) },
        onClick = { onChatView(!on) },
        trailingIcon = { Checkbox(checked = on, onCheckedChange = null) },
        modifier =
            Modifier.semantics {
                role = Role.Checkbox
                toggleableState = ToggleableState(on)
            },
    )
}

/** Asks before ending the session [name] on [machine]: [onConfirm] ends it, [onDismiss] keeps it. */
@Composable
internal fun EndSessionDialog(
    name: String,
    machine: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    ConfirmDialog(
        title = stringResource(R.string.machine_sessions_end_title, name),
        body = stringResource(R.string.machine_sessions_end_body, machine),
        confirmLabel = stringResource(R.string.machine_sessions_end),
        onConfirm = onConfirm,
        onDismiss = onDismiss,
    )
}
