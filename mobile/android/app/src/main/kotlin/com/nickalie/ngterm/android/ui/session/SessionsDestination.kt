package com.nickalie.ngterm.android.ui.session

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.SnackbarDuration
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.SnapshotStateList
import androidx.compose.runtime.toMutableStateList
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.ExtraKeysChoice
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.ui.chat.ChatViewModel
import com.nickalie.ngterm.android.ui.components.EmptyState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.terminal.TerminalState
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize

/**
 * The Sessions destination: the open session's view ([SessionScreen]), the gone state of one that ended while open,
 * or with none open the attached sessions by machine ([AttachedList]), or the empty state pointing to Machines. Back
 * closes an open session. [compact] (a phone) shows one pane at a time and a Back of its own; [notice] is a failure
 * to show once in a snackbar; [extraKeys] lays out the open session's extra keys; [chats] are the open chats of the
 * panes shown in chat view, by pane; [alone] are the sessions showing their selected pane alone
 * ([rememberAloneSessions]), held by the caller so a trip to another destination keeps them.
 */
@Composable
fun SessionsDestination(
    state: AttachedUiState,
    panes: Map<PaneRef, TerminalState>,
    failures: Map<PaneRef, String>,
    compact: Boolean,
    notice: String?,
    actions: AttachedActions,
    modifier: Modifier = Modifier,
    extraKeys: ExtraKeysChoice = ExtraKeysChoice.Fit,
    chats: Map<PaneRef, ChatViewModel> = emptyMap(),
    alone: SnapshotStateList<String> = rememberAloneSessions(),
) {
    val open = state.open
    BackHandler(enabled = open != null) { actions.onClose() }
    var confirmEnd by remember { mutableStateOf<AttachedSession?>(null) }
    val snackbar = remember { SnackbarHostState() }
    val shown by rememberUpdatedState(actions.onNoticeShown)
    LaunchedEffect(notice) {
        val text = notice ?: return@LaunchedEffect
        snackbar.showSnackbar(text, withDismissAction = true, duration = SnackbarDuration.Long)
        shown()
    }
    Box(modifier.fillMaxSize()) {
        when (open) {
            is OpenSession.Live -> {
                LiveSession(state, open, panes, failures, compact, actions, extraKeys, chats, alone)
            }

            is OpenSession.Gone -> {
                Centered {
                    EmptyState(
                        icon = R.drawable.ic_sessions,
                        title = stringResource(R.string.session_gone_title, open.name),
                        body = stringResource(R.string.session_gone_body, open.machine),
                        action = PanelAction(stringResource(R.string.session_close), onClick = actions.onClose),
                    )
                }
            }

            null -> {
                Attached(state, compact, actions, onAskEnd = { confirmEnd = it })
            }
        }
        SnackbarHost(snackbar, Modifier.align(Alignment.BottomCenter).safeDrawingPadding())
    }
    confirmEnd?.let { session ->
        EndSessionDialog(
            name = session.name,
            machine = session.machine,
            onConfirm = {
                confirmEnd = null
                actions.onEnd(session.key)
            },
            onDismiss = { confirmEnd = null },
        )
    }
}

@Composable
private fun LiveSession(
    state: AttachedUiState,
    open: OpenSession.Live,
    panes: Map<PaneRef, TerminalState>,
    failures: Map<PaneRef, String>,
    compact: Boolean,
    actions: AttachedActions,
    extraKeys: ExtraKeysChoice,
    chats: Map<PaneRef, ChatViewModel>,
    alone: SnapshotStateList<String>,
) {
    val sessions by rememberUpdatedState(state.sessions.map { it.key })
    val key = open.key
    val view =
        remember(actions, compact, key) {
            SessionViewActions(
                onBack = if (compact) actions.onClose else null,
                onFocus = actions.onFocus,
                onShow = actions.onShow,
                onRetry = actions.onRetry,
                onTakeLead = actions.onTakeLead,
                onDetach = actions.onDetach,
                onEnd = actions.onEnd,
                onNextSession = { sessions.cyclicNeighbor(key, forward = true)?.let(actions.onOpen) },
                onPreviousSession = { sessions.cyclicNeighbor(key, forward = false)?.let(actions.onOpen) },
                onView = actions.onView,
                onChatView = actions.onChatView,
                onAlone = { session, on ->
                    alone.remove(session.saved)
                    if (on) alone.add(session.saved)
                },
            )
        }
    SessionScreen(
        session = open.session,
        focused = open.focused,
        panes = panes,
        failures = failures,
        compact = compact,
        actions = view,
        extraKeys = extraKeys,
        views = open.views,
        chats = chats,
        alone = key.saved in alone,
    )
}

// a session's key as the saved state keeps it
private val SessionKey.saved: String get() = "$machineId\u0000$sessionId"

/**
 * The sessions (by key) showing their selected pane alone in a window that could show the origin's split: a choice
 * kept on this device, through rotation and the process's death. Remembered above the destinations, so a trip to
 * another one finds it.
 */
@Composable
fun rememberAloneSessions(): SnapshotStateList<String> = rememberSaveable(saver = AloneSaver) { mutableStateListOf() }

private val AloneSaver =
    listSaver<SnapshotStateList<String>, String>(save = { it.toList() }, restore = { it.toMutableStateList() })

@Composable
private fun Attached(
    state: AttachedUiState,
    compact: Boolean,
    actions: AttachedActions,
    onAskEnd: (AttachedSession) -> Unit,
) {
    if (state.groups.isEmpty()) {
        Centered {
            EmptyState(
                icon = R.drawable.ic_sessions,
                title = stringResource(R.string.sessions_empty_title),
                body = stringResource(R.string.sessions_empty_body),
                action = PanelAction(stringResource(R.string.sessions_open_machines), onClick = actions.onOpenMachines),
            )
        }
        return
    }
    val sessions = state.sessions
    Box(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.TopCenter) {
        AttachedList(
            groups = state.groups,
            onOpen = actions.onOpen,
            onDetach = actions.onDetach,
            onChatView = actions.onChatView,
            onEnd = { key -> sessions.firstOrNull { it.key == key }?.let(onAskEnd) },
            contentPadding =
                PaddingValues(
                    horizontal = if (compact) NgtermTheme.spacing.lg else NgtermTheme.spacing.xl,
                    vertical = NgtermTheme.spacing.lg,
                ),
        )
    }
}

@Composable
private fun Centered(content: @Composable () -> Unit) {
    Box(Modifier.fillMaxSize().safeDrawingPadding(), contentAlignment = Alignment.Center) { content() }
}
