package com.nickalie.ngterm.android.ui.session

import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.platform.LocalFocusManager
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.ExtraKeysChoice
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.ui.chat.ChatViewModel
import com.nickalie.ngterm.android.ui.chat.PaneView
import com.nickalie.ngterm.android.ui.components.ErrorState
import com.nickalie.ngterm.android.ui.components.PanelAction
import com.nickalie.ngterm.android.ui.terminal.CompositionStrip
import com.nickalie.ngterm.android.ui.terminal.ExtraKeysRow
import com.nickalie.ngterm.android.ui.terminal.TerminalKeyboard
import com.nickalie.ngterm.android.ui.terminal.TerminalShortcuts
import com.nickalie.ngterm.android.ui.terminal.TerminalState
import com.nickalie.ngterm.android.ui.terminal.extraKeysHeight
import com.nickalie.ngterm.android.ui.terminal.hardwareKeyboardAttached
import com.nickalie.ngterm.android.ui.terminal.rememberTerminalKeyboard
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize
import com.nickalie.ngterm.core.ffi.StreamState

/**
 * An attached session, live: its bar (name, status, context, the pane switcher, Detach and End session), the origin's
 * HUD, its panes, what waits to be typed ([CompositionStrip]) over their bottom edge, and the extra keys over the soft
 * keyboard. A phone ([compact]) shows the [focused] pane alone and a swipe across it moves to the next or previous one;
 * wider windows show the origin's split, or the [focused] pane [alone] once a pane tab asked for it ([tabTapAlone],
 * through [SessionViewActions.onAlone]). The shown panes' states are [panes] (by pane), and why one could not be shown
 * [failures]. One [keyboard] serves every pane and the extra keys, laid out as Settings' [extraKeys] says; its chords
 * move between the panes and the sessions. [views] says how each pane running an agent shows (by pane id), and the bar
 * switches the selected one's; a pane in chat view draws its open chat from [chats] (by pane) in its terminal's place.
 * The extra keys type into no chat, and step aside while every pane on screen is one; its terminal's grid then still
 * leaves them their room, so switching between the chat and the terminal never resizes the pane.
 *
 * The [focused] pane is the selected one, the only one anything typed reaches: a tab, a swipe, a chord and a touch
 * on a pane ask [SessionViewActions.onFocus] to select it, and the keyboard's target and the input focus follow the
 * selection, never the other way round. While the selected pane can't be shown, typing goes nowhere.
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
    extraKeys: ExtraKeysChoice = ExtraKeysChoice.Fit,
    views: Map<String, PaneView> = emptyMap(),
    chats: Map<PaneRef, ChatViewModel> = emptyMap(),
    alone: Boolean = false,
) {
    val key = session.key
    val live = session.live
    val ids = live.layout?.panes.orEmpty()
    val splittable = splittable(live.layout, compact)
    val spots = ids.map { id -> spot(key, id, panes, failures, session).inView(views[id], chats) }
    val focus by rememberUpdatedState({ id: String -> actions.onFocus(key, id) })
    val move: (Boolean) -> Unit = { forward -> ids.cyclicNeighbor(focused, forward)?.let(focus) }
    val selected = focused?.let { PaneRef(key.machineId, key.sessionId, it) }
    // the keys type into the selected pane's terminal, not into a chat
    val typed = selected?.takeIf { views[it.paneId] != PaneView.Chat }
    // with one pane the pane chords have nowhere to go: they are the program's
    SessionKeyboard(keyboard, typed?.let(panes::get), move.takeIf { ids.size > 1 }, actions)
    val onTab = tabTap(splittable, alone, focused, { actions.onAlone(key, it) }, focus)
    val chatsOnly = chatsOnly(visiblePanes(ids, focused, splittable && !alone), views)
    BoxWithConstraints(
        modifier
            .fillMaxSize()
            .background(NgtermTheme.terminal.background)
            .windowInsetsPadding(
                WindowInsets.safeDrawing.only(WindowInsetsSides.Horizontal + WindowInsetsSides.Bottom),
            ),
    ) {
        // a short window (a phone on its side with the soft keyboard up) keeps its rows for the terminal
        val roomy = maxHeight >= HudRoom
        val reserve = if (chatsOnly) extraKeysHeight(extraKeys, maxWidth) else 0.dp
        Column(Modifier.fillMaxSize()) {
            SessionBar(
                session = session,
                focused = focused,
                onBack = actions.onBack,
                onSelectPane = onTab,
                alone = alone.takeIf { splittable },
                onDetach = { actions.onDetach(key) },
                onEnd = { actions.onEnd(key) },
                view = focused?.let(views::get),
                onView = { view -> selected?.let { actions.onView(it, view) } },
                onChatView = { on -> actions.onChatView(key, on) },
                onDuplicate = { actions.onDuplicate(key) },
            )
            live.hud?.takeIf { roomy }?.let { HudBanner(it, Modifier.padding(NgtermTheme.spacing.sm)) }
            Box(Modifier.weight(1f).fillMaxWidth()) {
                val paneActions =
                    remember(keyboard, session.machine, actions, key) {
                        PaneActions(
                            keyboard = keyboard,
                            machine = session.machine,
                            onShow = actions.onShow,
                            onRetry = actions.onRetry,
                            onFocus = { actions.onFocus(key, it) },
                            onTakeLead = actions.onTakeLead,
                            onDetach = { actions.onDetach(key) },
                            onShowTerminal = { actions.onView(it, PaneView.Terminal) },
                        )
                    }
                Panes(session, spots, focused, compact, splittable && !alone, paneActions, move, reserve)
                // over the panes rather than beside them, so a held word never resizes the terminal
                CompositionStrip(keyboard, Modifier.align(Alignment.BottomEnd).padding(NgtermTheme.spacing.sm))
            }
            if (!chatsOnly) ExtraKeysRow(keyboard, layout = extraKeys)
        }
    }
}

// the origin's axis of two panes one over the other: the divider runs horizontally
private const val STACKED = "horizontal"

// the least height the session view shows the origin's HUD in: below it the HUD would take the terminal's last rows
private val HudRoom = 320.dp

// the panes, or what stands in for them before the layout arrives
@Composable
private fun Panes(
    session: AttachedSession,
    spots: List<PaneSpot>,
    focused: String?,
    compact: Boolean,
    split: Boolean,
    actions: PaneActions,
    move: (Boolean) -> Unit,
    reserve: Dp,
) {
    val layout = session.live.layout
    when {
        layout != null && spots.isNotEmpty() -> {
            val ids = spots.map { it.ref.paneId }
            val requesters = remember(ids) { ids.associateWith { FocusRequester() } }
            val selected = spots.firstOrNull { it.ref.paneId == focused }
            val ready = selected?.state != null
            // the pane that has the input focus, which may still be one the selection has left; a switch between the
            // split and a pane alone builds the panes' views anew, so none of them holds it then
            var holder by remember(split) { mutableStateOf<String?>(null) }
            FollowFocus(focused, ready, selected?.chatView == true, split, holder, requesters)
            // stacked halves share the extra keys' room between them
            val stacked = split && layout.axis == STACKED
            val slotReserve = if (stacked) reserve / 2 else reserve
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
                    reserve = slotReserve,
                    onHoldsFocus = { holds ->
                        if (holds) {
                            holder = id
                        } else if (holder == id) {
                            holder = null
                        }
                    },
                )
            }
            if (compact) {
                OnePane(spots, focused, slot)
            } else {
                SplitLayout(spots, layout.axis, split, focused, pane = slot)
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

// the keyboard types into the selected pane alone, and its chords move between the panes and the sessions
@Composable
private fun SessionKeyboard(
    keyboard: TerminalKeyboard,
    target: TerminalState?,
    move: ((Boolean) -> Unit)?,
    actions: SessionViewActions,
) {
    val moveNow by rememberUpdatedState(move)
    val moves = move != null
    // the selection sets the target, not a pane taking the focus; a selected pane with no state yet (on its way, or
    // it couldn't be shown) takes the keys too: they go nowhere
    SideEffect {
        keyboard.selectsOnFocus = false
        keyboard.target = target
    }
    LaunchedEffect(keyboard, actions, moves) {
        keyboard.shortcuts =
            TerminalShortcuts(
                nextPane = if (moves) ({ moveNow?.invoke(true) }) else null,
                previousPane = if (moves) ({ moveNow?.invoke(false) }) else null,
                nextSession = actions.onNextSession,
                previousSession = actions.onPreviousSession,
            )
    }
}

// the input focus follows the selected pane ([focused]) once its terminal is there ([ready]): while the soft keyboard
// is up or a hardware keyboard is attached, the selected pane takes it, so typing goes on there; otherwise a pane the
// selection left ([holder]) lets it go without the soft keyboard opening, so the next tap on any pane focuses it
// afresh. While the selected pane has no terminal, a hardware keyboard's focus goes to its slot, which takes only the
// chords, so typing goes nowhere and the chords still move away. A pane switched between its terminal and its chat
// ([chat]), or between the split and alone ([split]), takes it again in its new view
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun FollowFocus(
    focused: String?,
    ready: Boolean,
    chat: Boolean,
    split: Boolean,
    holder: String?,
    requesters: Map<String, FocusRequester>,
) {
    val imeUp = WindowInsets.isImeVisible
    val hardware = hardwareKeyboardAttached()
    val now by rememberUpdatedState(FocusFacts(imeUp || hardware, hardware, holder))
    val focusManager = LocalFocusManager.current
    LaunchedEffect(focused, ready, chat, split) {
        if (focused == null) return@LaunchedEffect
        // the pane's view joins the composition in this frame: its requester is attached in the next
        withFrameNanos { }
        val facts = now
        val left = facts.holder != null && facts.holder != focused
        // the focus is in the pane already (a touch put it in a field there): it stays where the touch put it
        if (facts.holder == focused) return@LaunchedEffect
        // a selected pane with no terminal takes a hardware keyboard's focus too (its chords only), never the soft one
        val takes = if (ready) facts.typing else facts.hardware
        if (takes) {
            try {
                requesters[focused]?.requestFocus()
            } catch (_: IllegalStateException) {
                // its view is not placed (a cover hides it): the keyboard still targets it
            }
        } else if (left) {
            focusManager.clearFocus()
        }
    }
}

// what the focus's move depends on as it happens: whether the user is typing (the soft keyboard up, or a hardware
// one attached), whether a hardware keyboard is, and which pane has the focus
private data class FocusFacts(
    val typing: Boolean,
    val hardware: Boolean,
    val holder: String?,
)

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
    return PaneSpot(ref, panes[ref], session.live.panes[paneId], failures[ref], session.taking)
}

// the panes on screen: both halves of a [split], else the selected one (or the first)
private fun visiblePanes(
    ids: List<String>,
    focused: String?,
    split: Boolean,
): List<String> = if (split) ids else listOfNotNull(focused?.takeIf { it in ids } ?: ids.firstOrNull())

// the spot as its view shows it: a chat in chat view, with the chat once it is open
private fun PaneSpot.inView(
    view: PaneView?,
    chats: Map<PaneRef, ChatViewModel>,
): PaneSpot = if (view == PaneView.Chat) copy(chatView = true, chat = chats[ref]) else this

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
