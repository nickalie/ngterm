package com.nickalie.ngterm.android.ui.session

import androidx.compose.foundation.background
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
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.data.PaneLive
import com.nickalie.ngterm.android.ui.components.BusBar
import com.nickalie.ngterm.android.ui.components.underCover
import com.nickalie.ngterm.android.ui.terminal.TerminalKeyboard
import com.nickalie.ngterm.android.ui.terminal.TerminalState
import com.nickalie.ngterm.android.ui.terminal.TerminalTextSize
import com.nickalie.ngterm.android.ui.terminal.TerminalView
import com.nickalie.ngterm.android.ui.terminal.gridFor
import com.nickalie.ngterm.android.ui.terminal.rememberTerminalFont
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize

/**
 * One pane of a session as the view places it: its [ref], its view's [state] once it is shown, what its events say
 * ([live]), and why it could not be shown ([failure]).
 */
data class PaneSpot(
    val ref: PaneRef,
    val state: TerminalState?,
    val live: PaneLive?,
    val failure: String?,
)

/**
 * What a pane's slot does: [keyboard] is the one every pane types through; [onShow] shows a pane at the grid its
 * slot first has room for; [onFocus] follows the pane a tap or the keyboard focused; [onTakeLead] is Take control;
 * [onDetach] lets the session go (an exited program's cover offers it); [machine] names where the panes live.
 */
class PaneActions(
    val keyboard: TerminalKeyboard,
    val machine: String,
    val onShow: (PaneRef, PaneSize) -> Unit,
    val onFocus: (String) -> Unit,
    val onTakeLead: (PaneRef) -> Unit,
    val onDetach: () -> Unit,
)

/**
 * A pane's slot: its terminal (once shown; the slot's first size shows it), with its cover when it can't be used as
 * it is. [focused] draws its cursor as it is (the others' go hollow); [readAloud] lets TalkBack read its text;
 * [onSwipe] takes a sideways swipe (a phone's next or previous pane). A focus taken inside it goes to
 * [PaneActions.onFocus].
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
) {
    var size by remember { mutableStateOf(IntSize.Zero) }
    val font = rememberTerminalFont(spot.state?.textSize ?: TerminalTextSize)
    val grid = gridFor(size, font)
    val show by rememberUpdatedState(actions.onShow)
    val waiting = spot.state == null && spot.failure == null
    LaunchedEffect(spot.ref, grid, waiting) {
        if (waiting && grid != null) show(spot.ref, grid)
    }
    val cover = PaneCover.of(spot.live, spot.state != null, spot.failure)
    val paneId = spot.ref.paneId
    Box(
        modifier
            .onSizeChanged { size = it }
            .background(NgtermTheme.terminal.background)
            .onFocusChanged { if (it.hasFocus) actions.onFocus(paneId) },
    ) {
        spot.state?.let { state ->
            TerminalView(
                state = state,
                modifier = Modifier.fillMaxSize().underCover(cover.veils),
                focused = focused,
                keyboard = actions.keyboard,
                focusRequester = focusRequester,
                readAloud = readAloud && !cover.veils,
                onSwipe = onSwipe,
            )
        }
        if (cover != null) {
            LeadCover(
                cover = cover,
                machine = actions.machine,
                onTakeLead = { actions.onTakeLead(spot.ref) },
                onDetach = actions.onDetach,
                onRetry = { grid?.let { actions.onShow(spot.ref, it) } },
            )
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
