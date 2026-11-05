package com.nickalie.ngterm.android.ui.session

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.focusTarget
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.data.FontChoice
import com.nickalie.ngterm.android.data.PaneLive
import com.nickalie.ngterm.android.ui.chat.ChatLoading
import com.nickalie.ngterm.android.ui.chat.ChatPane
import com.nickalie.ngterm.android.ui.chat.ChatViewModel
import com.nickalie.ngterm.android.ui.components.BusBar
import com.nickalie.ngterm.android.ui.components.underCover
import com.nickalie.ngterm.android.ui.terminal.TerminalKeyboard
import com.nickalie.ngterm.android.ui.terminal.TerminalState
import com.nickalie.ngterm.android.ui.terminal.TerminalTextSize
import com.nickalie.ngterm.android.ui.terminal.TerminalView
import com.nickalie.ngterm.android.ui.terminal.gridFor
import com.nickalie.ngterm.android.ui.terminal.move
import com.nickalie.ngterm.android.ui.terminal.rememberTerminalFont
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize

/**
 * One pane of a session as the view places it: its [ref], its view's [state] once it is shown, what its events say
 * ([live]), and why it could not be shown ([failure]); [chatView] when it shows as a chat, with its [chat] once that
 * is open.
 */
data class PaneSpot(
    val ref: PaneRef,
    val state: TerminalState?,
    val live: PaneLive?,
    val failure: String?,
    val chatView: Boolean = false,
    val chat: ChatViewModel? = null,
)

/**
 * What a pane's slot does: [keyboard] is the one every pane types through; [onShow] shows a pane at the grid its
 * slot first has room for, and [onRetry] tries one that could not be shown again; [onFocus] selects the pane a touch
 * or the input focus landed on; [onTakeLead] is Take control; [onDetach] lets the session go (an exited program's
 * cover offers it); [onShowTerminal] shows a chat's terminal in its place; [machine] names where the panes live.
 */
data class PaneActions(
    val keyboard: TerminalKeyboard,
    val machine: String,
    val onShow: (PaneRef, PaneSize) -> Unit,
    val onRetry: (PaneRef, PaneSize) -> Unit,
    val onFocus: (String) -> Unit,
    val onTakeLead: (PaneRef) -> Unit,
    val onDetach: () -> Unit,
    val onShowTerminal: (PaneRef) -> Unit = {},
)

/**
 * A pane's slot: its terminal (once shown; the slot's first size shows it), with its cover when it can't be used as
 * it is; or, in chat view, its chat ([ChatPane]) in the terminal's place, the terminal still attached behind it at the
 * grid the slot gives it less [reserve] (the extra keys' room, while they step aside for the chat), under no cover
 * but a failed show's (reading a chat never takes control).
 * [focused] draws its cursor as it is (the others' go hollow); [readAloud] lets TalkBack read its text;
 * [onSwipe] takes a sideways swipe (a phone's next or previous pane). Every touch on it, even on a pane that has the
 * input focus already, and a focus taken inside it select it ([PaneActions.onFocus]); [onHoldsFocus] says whether the
 * input focus is inside it.
 */
@Composable
fun PaneSlot(
    spot: PaneSpot,
    focused: Boolean,
    actions: PaneActions,
    modifier: Modifier = Modifier,
    focusRequester: FocusRequester? = null,
    readAloud: Boolean = false,
    onSwipe: ((forward: Boolean) -> Unit)? = null,
    onHoldsFocus: (Boolean) -> Unit = {},
    reserve: Dp = 0.dp,
) {
    var size by remember { mutableStateOf(IntSize.Zero) }
    val font =
        rememberTerminalFont(spot.state?.textSize ?: TerminalTextSize, spot.state?.face ?: FontChoice.JetBrainsMono)
    val kept = if (spot.chatView) with(LocalDensity.current) { reserve.roundToPx() } else 0
    val grid = gridFor(IntSize(size.width, (size.height - kept).coerceAtLeast(0)), font)
    val show by rememberUpdatedState(actions.onShow)
    val waiting = spot.state == null && spot.failure == null
    LaunchedEffect(spot.ref, grid, waiting) {
        if (waiting && grid != null) show(spot.ref, grid)
    }
    val cover = spot.cover()
    val paneId = spot.ref.paneId
    val select by rememberUpdatedState { actions.onFocus(paneId) }
    val holds by rememberUpdatedState(onHoldsFocus)
    Box(
        modifier
            .onSizeChanged { size = it }
            .background(NgtermTheme.terminal.background)
            .pointerInput(Unit) {
                // seen before the terminal's own gestures take it, so a touch selects the pane whatever it does next
                awaitEachGesture {
                    awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
                    select()
                }
            }.onFocusChanged {
                holds(it.hasFocus)
                if (it.hasFocus) select()
            },
    ) {
        if (spot.chatView) {
            ChatSlot(spot, focused, grid, actions, focusRequester, Modifier.fillMaxSize().underCover(cover.veils))
        } else {
            val state = spot.state
            if (state != null) {
                TerminalView(
                    state = state,
                    modifier = Modifier.fillMaxSize().underCover(cover.veils),
                    focused = focused,
                    keyboard = actions.keyboard,
                    focusRequester = focusRequester,
                    readAloud = readAloud && !cover.veils,
                    onSwipe = onSwipe,
                )
            } else {
                // no terminal yet (attaching, or it couldn't be shown): the slot still takes a hardware keyboard's
                // focus, so its chords move away
                Box(Modifier.fillMaxSize().paneKeys(actions.keyboard, focusRequester))
            }
        }
        if (cover != null) {
            LeadCover(
                cover = cover,
                machine = actions.machine,
                onTakeLead = { actions.onTakeLead(spot.ref) },
                onDetach = actions.onDetach,
                onRetry = { grid?.let { actions.onRetry(spot.ref, it) } },
            )
        }
    }
}

// a chat takes the input focus in its terminal's place (the session view moves it to the selected pane) and hands it
// on to its prompt field ([field]) when it has one; of a hardware keyboard's keys it takes only the chords that move
// between panes and sessions, ahead of a field typed in: the rest are the field's
private fun Modifier.chatKeys(
    keyboard: TerminalKeyboard,
    requester: FocusRequester?,
    field: FocusRequester,
): Modifier =
    onPreviewKeyEvent { keyboard.move(it.nativeKeyEvent) }
        .then(if (requester != null) Modifier.focusRequester(requester) else Modifier)
        .onFocusChanged { if (it.isFocused) field.requestIfShown() }
        .focusTarget()

// a slot without a terminal takes the input focus in its place; of a hardware keyboard's keys it takes only the chords
// that move between panes and sessions, typing nowhere
private fun Modifier.paneKeys(
    keyboard: TerminalKeyboard,
    requester: FocusRequester?,
): Modifier =
    onPreviewKeyEvent { keyboard.move(it.nativeKeyEvent) }
        .then(if (requester != null) Modifier.focusRequester(requester) else Modifier)
        .focusTarget()

// focuses the field if it is in the composition: a chat that ended, or isn't open yet, has none, and keeps the focus
private fun FocusRequester.requestIfShown() {
    try {
        requestFocus()
    } catch (_: IllegalStateException) {
        // no prompt field: the chat itself holds the focus
    }
}

// what covers the slot: a chat only a failed show's, since reading a chat never takes control
private fun PaneSpot.cover(): PaneCover? =
    PaneCover.of(live, state != null, failure)?.takeIf { !chatView || it is PaneCover.Failed }

// a pane in chat view: its chat once open (the backlog's wait until then), and its terminal, not drawn, still given the
// grid the slot would draw it at, so a chat action that takes control lays the program out for this device. A grid
// that holds for a frame (the extra keys stepping aside: the room they leave is counted before the slot has grown
// into it) is not given. The slot takes the input focus through [focusRequester] as the terminal would; Back closes
// the chat's `/` menu only in the [focused] (selected) pane
@Composable
private fun ChatSlot(
    spot: PaneSpot,
    focused: Boolean,
    grid: PaneSize?,
    actions: PaneActions,
    focusRequester: FocusRequester?,
    modifier: Modifier = Modifier,
) {
    val state = spot.state
    LaunchedEffect(state, grid) {
        if (state == null || grid == null) return@LaunchedEffect
        withFrameNanos { }
        state.resize(grid)
    }
    val chat = spot.chat
    val field = remember { FocusRequester() }
    Box(modifier.chatKeys(actions.keyboard, focusRequester, field)) {
        if (chat != null) {
            // keyed by the pane: a slot that shows another pane (a split laid out anew, or the app brought back after
            // it was put away) gives that pane's chat none of this one's state, live or saved
            key(spot.ref) {
                ChatPane(
                    chat,
                    actions.machine,
                    onShowTerminal = { actions.onShowTerminal(spot.ref) },
                    modifier = Modifier.fillMaxSize(),
                    focusRequester = field,
                    selected = focused,
                )
            }
        } else {
            Box(
                Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background),
                contentAlignment = Alignment.Center,
            ) {
                ChatLoading(actions.machine)
            }
        }
    }
}

/**
 * The session's split as its origin lays it out: two panes side by side (`vertical`, the divider's direction) or
 * stacked (`horizontal`), split by a rule, each with a gutter at its leading edge where the [focused] one shows the
 * bus bar. One pane, or a split the origin hides ([shown] false), shows the focused pane alone. [pane] draws a pane in
 * the modifier it is given.
 */
@Composable
fun SplitLayout(
    panes: List<PaneSpot>,
    axis: String?,
    shown: Boolean,
    focused: String?,
    modifier: Modifier = Modifier,
    pane: @Composable (PaneSpot, Modifier) -> Unit,
) {
    val split = panes.size == 2 && shown && axis != null
    if (!split) {
        val alone = panes.firstOrNull { it.ref.paneId == focused } ?: panes.firstOrNull() ?: return
        // a gutter as a split's, so the text keeps off the screen's edge
        pane(alone, modifier.fillMaxSize().padding(horizontal = NgtermTheme.spacing.xs))
        return
    }
    val rule = MaterialTheme.colorScheme.outlineVariant
    if (axis == SIDE_BY_SIDE) {
        Row(modifier.fillMaxSize()) {
            SplitHalf(panes[0], focused, Modifier.weight(1f), pane)
            Box(Modifier.width(1.dp).fillMaxHeight().background(rule))
            SplitHalf(panes[1], focused, Modifier.weight(1f), pane)
        }
    } else {
        Column(modifier.fillMaxSize()) {
            SplitHalf(panes[0], focused, Modifier.weight(1f), pane)
            Box(Modifier.height(1.dp).fillMaxWidth().background(rule))
            SplitHalf(panes[1], focused, Modifier.weight(1f), pane)
        }
    }
}

// a pane of a shown split: the gutter with the bus bar when it has the focus, then the pane
@Composable
private fun SplitHalf(
    spot: PaneSpot,
    focused: String?,
    modifier: Modifier = Modifier,
    pane: @Composable (PaneSpot, Modifier) -> Unit,
) {
    Row(modifier.background(NgtermTheme.terminal.background)) {
        BusBar(active = spot.ref.paneId == focused, length = FocusBarLength)
        pane(spot, Modifier.weight(1f).fillMaxHeight())
    }
}

/** The origin's axis of two panes side by side: the divider runs vertically. */
private const val SIDE_BY_SIDE = "vertical"

private val FocusBarLength = 32.dp
