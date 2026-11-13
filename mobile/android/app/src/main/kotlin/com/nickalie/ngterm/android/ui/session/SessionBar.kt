package com.nickalie.ngterm.android.ui.session

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.badge
import com.nickalie.ngterm.android.ui.chat.PaneView
import com.nickalie.ngterm.android.ui.components.AgentStatusBadge
import com.nickalie.ngterm.android.ui.components.PeerBadge
import com.nickalie.ngterm.android.ui.components.StatusBadge
import com.nickalie.ngterm.android.ui.sessions.UnderWay
import com.nickalie.ngterm.android.ui.terminal.CompositionStrip
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusTone
import com.nickalie.ngterm.core.ffi.StreamState

/**
 * The session view's one bar, under the status bar: Back ([onBack], a phone's), the session's name over its state
 * (the agent's status while the stream is live, the stream's own state while it isn't) and its context, the pane
 * switcher ([onSelectPane]; [focused] marked; [alone] where a tab also toggles the split, see [PaneSwitcher]), the
 * view switch while the focused pane runs an agent ([view], how it shows now; [onView] asks for the other), and More:
 * Detach ([onDetach], at once), Duplicate session ([onDuplicate]; "Duplicating" stands in for the state while it
 * runs), Chat view (the session's, flipped through [onChatView]; off on an app without one) and End session ([onEnd],
 * once confirmed). A session of a machine running another app than ngterm has that app's [PeerBadge] after its name,
 * whose tap says what the app lacks. Text
 * waiting to be typed ([held]) shows in the name's place while there is no extra-keys row to hold it.
 */
@Composable
fun SessionBar(
    session: AttachedSession,
    focused: String?,
    onBack: (() -> Unit)?,
    onSelectPane: (String) -> Unit,
    onDetach: () -> Unit,
    onEnd: () -> Unit,
    modifier: Modifier = Modifier,
    view: PaneView? = null,
    onView: (PaneView) -> Unit = {},
    alone: Boolean? = null,
    onChatView: (Boolean) -> Unit = {},
    onDuplicate: () -> Unit = {},
    held: String = "",
) {
    val spacing = NgtermTheme.spacing
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surfaceContainer)
                .windowInsetsPadding(WindowInsets.safeDrawing.only(WindowInsetsSides.Top))
                .heightIn(min = BarHeight)
                .padding(start = if (onBack == null) spacing.lg else spacing.xs, end = spacing.xs),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (onBack != null) {
            IconButton(onClick = onBack) {
                Icon(painterResource(R.drawable.ic_back), contentDescription = stringResource(R.string.action_back))
            }
        }
        Box(Modifier.weight(1f)) {
            // kept in place under the held text, so the bar keeps its height and the terminal its size
            val title = if (held.isEmpty()) Modifier else Modifier.alpha(0f).clearAndSetSemantics {}
            Column(
                modifier = title.fillMaxWidth().padding(vertical = spacing.sm),
                verticalArrangement = Arrangement.spacedBy(spacing.xxs),
            ) {
                Text(
                    text = session.name,
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.semantics { heading() },
                )
                StateLine(session)
            }
            if (held.isNotEmpty()) CompositionStrip(held, Modifier.matchParentSize(), inset = 0.dp)
        }
        session.peer?.takeIf { it.foreign }?.let { PeerBadge(it.badge, note = it.note) }
        val live = session.live
        PaneSwitcher(
            panes = live.layout?.panes.orEmpty(),
            selected = focused,
            titles = live.panes.mapValues { it.value.title },
            onSelect = onSelectPane,
            alone = alone,
        )
        view?.let { ViewSwitch(it, onView) }
        BarMenu(session, onDetach, onChatView, onEnd, onDuplicate)
    }
}

// More, with End session confirmed first
@Composable
private fun BarMenu(
    session: AttachedSession,
    onDetach: () -> Unit,
    onChatView: (Boolean) -> Unit,
    onEnd: () -> Unit,
    onDuplicate: () -> Unit,
) {
    var confirming by rememberSaveable { mutableStateOf(false) }
    SessionMenu(
        name = session.name,
        chatView = session.chatView,
        onDetach = onDetach,
        onChatView = onChatView,
        onEnd = { confirming = true },
        canDuplicate = session.canDuplicate,
        onDuplicate = onDuplicate,
        noChatOn = session.noChatOn,
    )
    if (confirming) {
        EndSessionDialog(
            name = session.name,
            machine = session.machine,
            onConfirm = {
                confirming = false
                onEnd()
            },
            onDismiss = { confirming = false },
        )
    }
}

// the focused pane's other view, named for what a tap does: Show terminal over a chat, Show chat over a terminal
@Composable
private fun ViewSwitch(
    view: PaneView,
    onView: (PaneView) -> Unit,
    modifier: Modifier = Modifier,
) {
    val other = if (view == PaneView.Chat) PaneView.Terminal else PaneView.Chat
    val (icon, label) =
        when (other) {
            PaneView.Terminal -> R.drawable.ic_sessions to R.string.session_show_terminal
            PaneView.Chat -> R.drawable.ic_chat to R.string.session_show_chat
        }
    IconButton(onClick = { onView(other) }, modifier = modifier) {
        Icon(painterResource(icon), contentDescription = stringResource(label))
    }
}

// the session's state, then its context, on one line
@Composable
private fun StateLine(
    session: AttachedSession,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier,
        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        StateBadge(session)
        session.context?.let { context ->
            Text(
                text = context,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun StateBadge(
    session: AttachedSession,
    modifier: Modifier = Modifier,
) {
    val reconnecting = session.link is Link.Reconnecting
    if (session.duplicating) {
        UnderWay(stringResource(R.string.machine_sessions_duplicating), modifier)
        return
    }
    when (session.live.connection) {
        StreamState.Connected -> {
            session.status?.let { AgentStatusBadge(it.text, it.color, it.shape, modifier) }
        }

        StreamState.Connecting -> {
            val label = if (reconnecting) R.string.session_reconnecting else R.string.status_connecting
            StatusBadge(StatusTone.Connecting, modifier, stringResource(label))
        }

        is StreamState.Failed -> {
            StatusBadge(StatusTone.Connecting, modifier, stringResource(R.string.session_reconnecting))
        }

        StreamState.Unsupported -> {
            StatusBadge(StatusTone.Error, modifier, stringResource(R.string.session_cant_stream))
        }
    }
}

private val BarHeight = 64.dp
