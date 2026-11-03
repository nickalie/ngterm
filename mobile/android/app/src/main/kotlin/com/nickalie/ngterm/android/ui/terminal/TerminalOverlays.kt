package com.nickalie.ngterm.android.ui.terminal

import android.content.ClipData
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.stringResource
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.TerminalPalette
import kotlinx.coroutines.launch

/** What the toolbar and the menu do, over the clipboard. */
private class TerminalActions(
    val copy: () -> Unit,
    val paste: () -> Unit,
    val selectAll: () -> Unit,
)

@Composable
private fun rememberTerminalActions(selection: TerminalSelection): TerminalActions {
    val clipboard = LocalClipboard.current
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val label = stringResource(R.string.terminal_description)
    return remember(selection, clipboard, context, scope) {
        TerminalActions(
            copy = {
                selection.copy()?.let { text ->
                    scope.launch { clipboard.setClipEntry(ClipEntry(ClipData.newPlainText(label, text))) }
                }
            },
            paste = {
                selection.closeMenu()
                scope.launch {
                    val clip = clipboard.getClipEntry()?.clipData
                    val text =
                        clip
                            ?.takeIf { it.itemCount > 0 }
                            ?.getItemAt(0)
                            ?.coerceToText(context)
                            ?.toString()
                    if (!text.isNullOrEmpty()) selection.paste(text)
                }
            },
            selectAll = selection::selectAll,
        )
    }
}

/**
 * What a [TerminalView] shows over its text, from [selection]: a touch selection's handles and toolbar, the
 * right-click menu, and the confirmation of a paste the pane refused as unsafe. The view is [width] pixels wide.
 */
@Composable
internal fun BoxScope.TerminalOverlays(
    state: TerminalState,
    selection: TerminalSelection,
    font: TerminalFont,
    palette: TerminalPalette,
    width: Int,
) {
    val actions = rememberTerminalActions(selection)
    if (selection.touchSelection) {
        SelectionOverlay(state, selection, font, palette, actions, width, Modifier.matchParentSize())
    }
    selection.menuAt?.let { at ->
        TerminalMenu(
            at = at,
            hasSelection = state.screen.selection.isNotEmpty(),
            onCopy = actions.copy,
            onPaste = actions.paste,
            onSelectAll = actions.selectAll,
            onDismiss = selection::closeMenu,
        )
    }
    selection.unsafePaste?.let { text ->
        PasteConfirmDialog(
            text = text,
            onPaste = { selection.paste(text, allowUnsafe = true) },
            onDismiss = selection::cancelPaste,
        )
    }
}

/**
 * A touch selection's handles, drawn in the palette's cursor color, and its toolbar while no finger moves it. It
 * composes again with each frame, which only a touch selection costs.
 */
@Composable
private fun SelectionOverlay(
    state: TerminalState,
    selection: TerminalSelection,
    font: TerminalFont,
    palette: TerminalPalette,
    actions: TerminalActions,
    width: Int,
    modifier: Modifier = Modifier,
) {
    val drawn = state.drawn
    val spans = state.screen.selection
    LaunchedEffect(drawn) { selection.shown(spans) }
    val knob = with(LocalDensity.current) { KnobRadius.toPx() }
    val spots = remember(drawn, font, knob, width) { handleSpots(spans, font, knob, width.toFloat()) } ?: return
    Box(modifier) {
        Spacer(Modifier.matchParentSize().drawBehind { drawHandles(spots, palette.cursor) })
        if (!selection.adjusting) {
            SelectionToolbar(
                spots = spots,
                onCopy = actions.copy,
                onPaste = actions.paste,
                onSelectAll = actions.selectAll,
                modifier = Modifier.matchParentSize(),
            )
        }
    }
}
