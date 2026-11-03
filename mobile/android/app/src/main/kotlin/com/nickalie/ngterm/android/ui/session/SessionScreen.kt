package com.nickalie.ngterm.android.ui.session

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.isImeVisible
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.ui.components.ErrorState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.terminal.ExtraKeysRow
import com.nickalie.ngterm.android.ui.terminal.TerminalKeyboard
import com.nickalie.ngterm.android.ui.terminal.TerminalShortcuts
import com.nickalie.ngterm.android.ui.terminal.TerminalState
import com.nickalie.ngterm.android.ui.terminal.hardwareKeyboardAttached
import com.nickalie.ngterm.android.ui.terminal.rememberTerminalKeyboard
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize
import com.nickalie.ngterm.core.ffi.StreamState

/**
 * An attached session, live: its bar (name, status, context, the pane switcher, Detach and End session), the
 * origin's HUD, its panes, and the extra keys over the soft keyboard. A phone ([compact]) shows the [focused] pane
 * alone and a swipe across it moves to the next or previous one; wider windows show the origin's split. The shown
 * panes' states are [panes] (by pane), and why one could not be shown [failures]. One [keyboard] serves every pane
 * and the extra keys; its chords move between the panes and the sessions.
 */
@Composable
fun SessionScreen(
    session: AttachedSession,
    focused: String?,
    panes: Map<PaneRef, TerminalState>,
    failures: Map<PaneRef, String>,
    compact: Boolean,
    actions: SessionViewActions,
    modifier: Modifier = Modifier,
    keyboard: TerminalKeyboard = rememberTerminalKeyboard(),
) {
    val key = session.key
    val live = session.live
    val ids = live.layout?.panes.orEmpty()
    val spots = ids.map { id -> spot(key, id, panes, failures, session) }
    val focus by rememberUpdatedState({ id: String -> actions.onFocus(key, id) })
    val move: (Boolean) -> Unit = { forward -> ids.cyclicNeighbor(focused, forward)?.let(focus) }
    SessionKeyboard(keyboard, spots.firstOrNull { it.ref.paneId == focused }?.state, move, actions)
    Column(
        modifier
            .fillMaxSize()
            .background(NgtermTheme.terminal.background)
            .windowInsetsPadding(
                WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal + WindowInsetsSides.Bottom),
            ),
    ) {
        SessionBar(
            session = session,
            focused = focused,
            onBack = actions.onBack,
            onSelectPane = focus,
            onDetach = { actions.onDetach(key) },
            onEnd = { actions.onEnd(key) },
        )
        live.hud?.let { HudBanner(it, Modifier.padding(NgtermTheme.spacing.sm)) }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            val paneActions =
                remember(keyboard, session.machine, actions, key) {
                    PaneActions(
                        keyboard = keyboard,
                        machine = session.machine,
                        onShow = actions.onShow,
                        onFocus = { actions.onFocus(key, it) },
                        onTakeLead = actions.onTakeLead,
                        onDetach = { actions.onDetach(key) },
                    )
                }
            Panes(session, spots, focused, compact, paneActions, move)
        }
        ExtraKeysRow(keyboard)
    }
}

// the panes, or what stands in for them before the layout arrives
@Composable
private fun Panes(
    session: AttachedSession,
    spots: List<PaneSpot>,
    focused: String?,
    compact: Boolean,
    actions: PaneActions,
    move: (Boolean) -> Unit,
) {
    val layout = session.live.layout
    when {
        layout != null && spots.isNotEmpty() -> {
            val ids = spots.map { it.ref.paneId }
            val requesters = remember(ids) { ids.associateWith { FocusRequester() } }
            val ready = spots.firstOrNull { it.ref.paneId == focused }?.state != null
            FollowFocus(focused, ready, requesters)
            val slot: @Composable (PaneSpot, Modifier) -> Unit = { spot, slotModifier ->
                val id = spot.ref.paneId
                PaneSlot(
                    spot = spot,
                    focused = id == focused,
                    actions = actions,
                    modifier = slotModifier,
                    focusRequester = requesters[id],
                    readAloud = id == focused,
                    onSwipe = if (compact && spots.size > 1) move else null,
                )
            }
            if (compact) {
                OnePane(spots, focused, slot)
            } else {
                SplitLayout(spots, layout.axis, layout.shown, focused, pane = slot)
            }
        }

        session.live.connection is StreamState.Unsupported -> {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                ErrorState(
                    title = stringResource(R.string.session_unsupported_title, session.machine),
                    body = stringResource(R.string.session_unsupported_body, session.machine),
                    action = PanelAction(stringResource(R.string.machine_sessions_detach), onClick = actions.onDetach),
                )
            }
        }

        else -> {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { Opening(session.name) }
        }
    }
}

// a phone's one pane at a time, sliding in from the side it was swiped from
@Composable
private fun OnePane(
    spots: List<PaneSpot>,
    focused: String?,
    slot: @Composable (PaneSpot, Modifier) -> Unit,
) {
    val shown = spots.firstOrNull { it.ref.paneId == focused } ?: spots.first()
    val motion = NgtermTheme.motion
    AnimatedContent(
        targetState = shown,
        contentKey = { it.ref.paneId },
        transitionSpec = {
            val forward = spots.indexOf(targetState) >= spots.indexOf(initialState)
            val sign = if (forward) 1 else -1
            slideInHorizontally(motion.standard()) { it * sign } togetherWith
                slideOutHorizontally(motion.standard()) { -it * sign }
        },
        label = "pane",
    ) { spot ->
        slot(spot, Modifier.fillMaxSize().padding(horizontal = NgtermTheme.spacing.xs))
    }
}

// the keyboard follows the focused pane, and its chords move between the panes and the sessions
@Composable
private fun SessionKeyboard(
    keyboard: TerminalKeyboard,
    target: TerminalState?,
    move: (Boolean) -> Unit,
    actions: SessionViewActions,
) {
    val moveNow by rememberUpdatedState(move)
    // a focused pane with no state yet (on its way, or it couldn't be shown) takes the keys too: they go nowhere
    SideEffect { keyboard.target = target }
    LaunchedEffect(keyboard, actions) {
        keyboard.shortcuts =
            TerminalShortcuts(
                nextPane = { moveNow(true) },
                previousPane = { moveNow(false) },
                nextSession = actions.onNextSession,
                previousSession = actions.onPreviousSession,
            )
    }
}

// while the soft keyboard is up or a hardware keyboard is attached, the pane the focus moves to takes it once its
// terminal is there ([ready]), so typing goes on there
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun FollowFocus(
    focused: String?,
    ready: Boolean,
    requesters: Map<String, FocusRequester>,
) {
    val typing = WindowInsets.isImeVisible || hardwareKeyboardAttached()
    val typingNow by rememberUpdatedState(typing)
    LaunchedEffect(focused, ready) {
        if (!typingNow || focused == null || !ready) return@LaunchedEffect
        // the pane's view joins the composition in this frame: its requester is attached in the next
        withFrameNanos { }
        try {
            requesters[focused]?.requestFocus()
        } catch (_: IllegalStateException) {
            // its view is not placed (a cover hides it, or it is not shown yet): the keyboard still targets it
        }
    }
}

@Composable
private fun Opening(
    name: String,
    modifier: Modifier = Modifier,
) {
    val label = stringResource(R.string.session_opening, name)
    Column(
        modifier = modifier.padding(NgtermTheme.spacing.xl),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.lg),
    ) {
        CircularProgressIndicator()
        Text(
            text = label,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier =
                Modifier.semantics {
                    contentDescription = label
                    liveRegion = LiveRegionMode.Polite
                },
        )
    }
}

private fun spot(
    key: SessionKey,
    paneId: String,
    panes: Map<PaneRef, TerminalState>,
    failures: Map<PaneRef, String>,
    session: AttachedSession,
): PaneSpot {
    val ref = PaneRef(key.machineId, key.sessionId, paneId)
    return PaneSpot(ref, panes[ref], session.live.panes[paneId], failures[ref])
}

/**
 * The item after (or before) [current], wrapping around (from the first when [current] isn't here); `null` with fewer
 * than two: the next pane of a session, the next attached session.
 */
internal fun <T> List<T>.cyclicNeighbor(
    current: T?,
    forward: Boolean,
): T? {
    if (size < 2) return null
    val at = indexOf(current).coerceAtLeast(0)
    val step = if (forward) 1 else -1
    return this[(at + step + size) % size]
}
