package com.nickalie.ngterm.android.ui.terminal

import android.content.Context
import android.graphics.Typeface
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Spacer
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.SideEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.text
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.res.ResourcesCompat
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.DEFAULT_TEXT_SIZE
import com.nickalie.ngterm.android.data.FontChoice
import com.nickalie.ngterm.android.data.PaneFrames
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.TerminalPalette
import com.nickalie.ngterm.core.ffi.PaneColors
import com.nickalie.ngterm.core.ffi.PaneInputException
import com.nickalie.ngterm.core.ffi.PaneInterface
import com.nickalie.ngterm.core.ffi.PaneKey
import com.nickalie.ngterm.core.ffi.PaneSize
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import java.util.concurrent.atomic.AtomicBoolean
import kotlin.coroutines.CoroutineContext
import kotlin.math.roundToInt

/** The terminal's text size until the user picks another ([DEFAULT_TEXT_SIZE]): about 50 columns on a phone upright. */
val TerminalTextSize = DEFAULT_TEXT_SIZE.sp

/**
 * A shown pane's screen as a [TerminalView] draws it, read from the pane's packed frames (`Pane.frame`).
 *
 * Its owner routes the pane's events to it: [dirty] on `PaneDirty`, [attached] on `PaneAttached` (a fresh terminal,
 * read from scratch). The view reads at most one frame per display frame, asking for the rows changed since the
 * frame it holds, and gives the pane its grid and colors. A pane with no terminal (hidden, between attaches) keeps
 * the last screen on view.
 *
 * @param frameContext where `Pane.frame` and its decoding run, off the main thread.
 * @param textSize the text size to start at, and the one Ctrl+0 returns to ([defaultTextSize]).
 * @param face the font to draw in.
 */
@Stable
class TerminalState(
    internal val pane: PaneInterface,
    private val frameContext: CoroutineContext = Dispatchers.Default,
    textSize: TextUnit = TerminalTextSize,
    face: FontChoice = FontChoice.JetBrainsMono,
) : PaneFrames {
    /** The text size the view draws at: a pinch changes it, in whole sp within [TerminalTextSizes]. */
    var textSize by mutableStateOf(textSize)

    /** The text size Settings sets, which Ctrl+0 puts the view back to. */
    var defaultTextSize: TextUnit = textSize

    /** The font the view draws in, as Settings picks it. */
    var face by mutableStateOf(face)

    /** The input method's composition, drawn at the cursor until it commits; empty for none. */
    var composing by mutableStateOf("")
        internal set

    /** The rows read so far. Read and changed on the main thread only. */
    val screen = TerminalScreen()

    /** Bumped by each frame taken, so the view draws again. */
    internal var drawn by mutableIntStateOf(0)
        private set

    /** The grid the view last gave the pane; `null` until it has a size that holds a cell. */
    var grid by mutableStateOf<PaneSize?>(null)
        private set

    private val wakes = Channel<Unit>(Channel.CONFLATED)
    private val restart = AtomicBoolean(true)

    /** The pane has a new frame (`PaneDirty`); safe from any thread. */
    override fun dirty() {
        wakes.trySend(Unit)
    }

    /** The pane has a fresh terminal (`PaneAttached`): the next frame is read whole; safe from any thread. */
    override fun attached() {
        restart.set(true)
        wakes.trySend(Unit)
    }

    /** Reads frames as they come, one per display frame at most, until cancelled. */
    internal suspend fun run() {
        wakes.trySend(Unit)
        while (true) {
            wakes.receive()
            // one read per display frame, whatever number of wakes came since
            withFrameNanos { }
            val since = if (restart.getAndSet(false)) 0L else screen.seq
            val update = read(since) ?: continue
            screen.apply(update)
            drawn++
            if (update.header.stale) {
                // the terminal was busy: the frame made last came instead, so ask again shortly
                delay(STALE_RETRY_MS)
                wakes.trySend(Unit)
            }
        }
    }

    private suspend fun read(since: Long): FrameUpdate? =
        withContext(frameContext) {
            try {
                RowBufferReader.read(pane.frame(since.toULong()))
            } catch (_: PaneInputException) {
                // no terminal now (hidden, or between attaches): the last screen stays until the next one
                null
            }
        }

    /** Gives the pane [size]: its terminal reflows and the machine's pty follows. */
    internal fun resize(size: PaneSize) {
        if (size == grid) return
        grid = size
        try {
            pane.resize(size)
        } catch (_: PaneInputException) {
            // kept by the pane for its next terminal anyway; a size it refuses has nothing to retry
        }
    }

    /** Sends [key] to the program. */
    internal fun key(key: PaneKey) {
        sendInput { pane.key(key) }
    }

    /** Types [text] as the keyboard would. */
    internal fun type(text: String) {
        sendInput { pane.typeText(text) }
    }

    // a pane with no terminal now (hidden, between attaches) drops the key, as a closed terminal would; a full queue
    // (the program reads nothing) drops it too, as the program would not see it for a while anyway
    private inline fun sendInput(send: () -> Unit) {
        try {
            send()
        } catch (_: PaneInputException) {
        }
    }

    /** Gives the pane [palette] as the colors its terminal starts from. */
    internal fun colors(palette: TerminalPalette) {
        try {
            pane.setColors(palette.toPaneColors())
        } catch (_: PaneInputException) {
            // the terminal keeps the colors it has
        }
    }

    private companion object {
        /** How soon a stale frame is asked for again. */
        const val STALE_RETRY_MS = 16L
    }
}

/** A [TerminalState] for [pane], kept while the pane is the same handle. */
@Composable
fun rememberTerminalState(pane: PaneInterface): TerminalState = remember(pane) { TerminalState(pane) }

/**
 * A pane's terminal: the rows of [state]'s screen on a canvas in the bundled JetBrains Mono at the state's text size
 * (Ghostty's and desktop ngterm's default face), the platform's fonts covering what it lacks (CJK, emoji), with the
 * cursor and the selection. It fills what it is given with whole cells from the top left and gives the pane the grid
 * that fits ([TerminalState.grid]), and the colors of [palette] (the theme's by default). [focused] draws the cursor
 * as it is; an unfocused terminal shows a hollow block.
 *
 * It takes the pointer as [terminalGestures] says: scrolling, selecting with handles and a toolbar, pinching the text
 * size, the mouse and its right-click menu, and clicks a program tracking the mouse asks for ([TerminalOverlays]).
 * A tap gives it the focus and opens the soft keyboard (a mouse click only the focus): with the focus, the input
 * method and a hardware keyboard type into it through [keyboard], and a composition waits at the cursor.
 * [focusRequester] lets its host give it the focus (the session view, as the focused pane moves). With [readAloud]
 * TalkBack reads its screen's text; [onSwipe] takes a finger's sideways swipe (next pane: leftward), which otherwise
 * scrolls like any drag.
 */
@Composable
fun TerminalView(
    state: TerminalState,
    modifier: Modifier = Modifier,
    palette: TerminalPalette = NgtermTheme.terminal,
    focused: Boolean = true,
    keyboard: TerminalKeyboard = rememberTerminalKeyboard(),
    focusRequester: FocusRequester? = null,
    readAloud: Boolean = false,
    onSwipe: ((forward: Boolean) -> Unit)? = null,
) {
    val density = LocalDensity.current
    val font = rememberTerminalFont(state.textSize, state.face)
    val painter = remember(font) { TerminalPainter(font, GlyphCache(font)) }
    val scope = rememberCoroutineScope()
    val input = remember(state) { TerminalInput(state, scope) }
    val selection = remember(state) { TerminalSelection(state) }
    val ownFocus = remember { FocusRequester() }
    val focus = focusRequester ?: ownFocus
    val softKeyboard = rememberSoftKeyboardRequest(focus)
    val underline = with(density) { CompositionUnderline.toPx() }
    val composition = remember(font, underline) { CompositionPainter(font, underline) }
    SideEffect {
        input.font = font
        input.keyboardRequest = softKeyboard::request
        input.swipe = onSwipe
    }
    var viewSize by remember { mutableStateOf(IntSize.Zero) }
    LaunchedEffect(state) { state.run() }
    LaunchedEffect(state, palette) { state.colors(palette) }
    LaunchedEffect(state, viewSize, font) { gridFor(viewSize, font)?.let(state::resize) }
    val description = stringResource(R.string.terminal_description)
    val spoken = if (readAloud) rememberSpokenText(state) else null
    val ground = palette.background
    val look = TerminalLook(palette.selectionBackground.toArgb(), focused)
    val knob = with(density) { KnobRadius.toPx() }
    // the gestures' coroutine outlives a pinch (a pinch is one of its gestures): it reads the handles' geometry here
    val handleFont by rememberUpdatedState(font)
    val handleKnob by rememberUpdatedState(knob)
    val haptics = LocalHapticFeedback.current
    Box(modifier.onSizeChanged { viewSize = it }) {
        Spacer(
            Modifier
                .matchParentSize()
                .terminalSemantics(description, spoken)
                .terminalTextInput(keyboard, state, softKeyboard)
                .focusRequester(focus)
                .focusable()
                .clipToBounds()
                .drawBehind {
                    // read here, so a new frame or composition draws again without composing again
                    state.drawn
                    drawRect(ground)
                    val canvas = drawContext.canvas.nativeCanvas
                    painter.draw(canvas, state.screen, size, look)
                    composition.draw(canvas, state.composing, state, palette, size.width)
                }.terminalGestures(
                    input = input,
                    selection = selection,
                    haptics = haptics,
                    handles = {
                        val width = viewSize.width.toFloat()
                        val spans = state.screen.selection
                        if (selection.touchSelection) handleSpots(spans, handleFont, handleKnob, width) else null
                    },
                    textSize = { state.textSize.value },
                ),
        )
        TerminalOverlays(state, selection, font, palette, viewSize.width)
    }
}

/** The terminal as TalkBack finds it: [description], and its screen's text when it is read aloud ([spoken]). */
private fun Modifier.terminalSemantics(
    description: String,
    spoken: String?,
): Modifier =
    semantics {
        contentDescription = description
        if (spoken != null) text = AnnotatedString(spoken)
    }

/**
 * The terminal's font at [textSize] in this density: the bundled JetBrains Mono faces, or the device's monospace in
 * its four styles ([face]).
 */
@Composable
fun rememberTerminalFont(
    textSize: TextUnit,
    face: FontChoice = FontChoice.JetBrainsMono,
): TerminalFont {
    val context = LocalContext.current
    val typefaces = remember(context, face) { terminalTypefaces(context, face) }
    val pixels = with(LocalDensity.current) { textSize.toPx() }
    return remember(typefaces, pixels) { TerminalFont(typefaces, pixels) }
}

/**
 * [state]'s screen as text for TalkBack: its rows, one per line, trailing blank rows dropped, taken at most once per
 * [SPOKEN_REFRESH_MS] while frames come, so a busy program doesn't rebuild it per frame.
 */
@Composable
private fun rememberSpokenText(state: TerminalState): String {
    var spoken by remember(state) { mutableStateOf("") }
    LaunchedEffect(state) {
        snapshotFlow { state.drawn }.collect {
            spoken = state.screen.spokenText()
            delay(SPOKEN_REFRESH_MS)
        }
    }
    return spoken
}

/** The screen's rows as lines of text, the trailing blank ones dropped. */
fun TerminalScreen.spokenText(): String =
    (0 until rowCount)
        .map(::text)
        .dropLastWhile {
            it.isEmpty()
        }.joinToString("\n")

/** How often the text TalkBack reads is taken again while the screen changes. */
private const val SPOKEN_REFRESH_MS = 1_000L

/** The terminal's [SoftKeyboardRequest]: it gives the focus to what [focus] is attached to. */
@Composable
private fun rememberSoftKeyboardRequest(focus: FocusRequester): SoftKeyboardRequest {
    val view = LocalView.current
    return remember(focus, view) { SoftKeyboardRequest(focus, view) }
}

/** The grid of whole cells of [font] that fits [size] pixels; `null` when not one cell fits. */
fun gridFor(
    size: IntSize,
    font: TerminalFont,
): PaneSize? {
    val columns = (size.width / font.cellWidth).toInt().coerceAtMost(MAX_CELLS)
    val rows = (size.height / font.cellHeight).coerceAtMost(MAX_CELLS)
    if (columns < 1 || rows < 1) return null
    return PaneSize(
        cols = columns.toUShort(),
        rows = rows.toUShort(),
        cellWidth = font.cellWidth.roundToInt().toUInt(),
        cellHeight = font.cellHeight.toUInt(),
    )
}

/** The composition's underline at the cursor. */
private val CompositionUnderline = 2.dp

/** The most columns or rows a grid has (the pane's sizes are 16-bit). */
private const val MAX_CELLS = 0xFFFF

private fun terminalTypefaces(
    context: Context,
    choice: FontChoice,
): TerminalTypefaces {
    if (choice == FontChoice.SystemMono) {
        fun system(style: Int) = Typeface.create(Typeface.MONOSPACE, style)
        return TerminalTypefaces(
            regular = Typeface.MONOSPACE,
            bold = system(Typeface.BOLD),
            italic = system(Typeface.ITALIC),
            boldItalic = system(Typeface.BOLD_ITALIC),
        )
    }

    fun face(id: Int) = requireNotNull(ResourcesCompat.getFont(context, id)) { "the terminal font is missing" }
    return TerminalTypefaces(
        regular = face(R.font.jetbrains_mono_regular),
        bold = face(R.font.jetbrains_mono_bold),
        italic = face(R.font.jetbrains_mono_italic),
        boldItalic = face(R.font.jetbrains_mono_bold_italic),
    )
}

/** The palette as the pane's colors: the ground, the text, the cursor and the 16 ANSI colors. */
fun TerminalPalette.toPaneColors(): PaneColors =
    PaneColors(
        foreground = foreground.toArgb().toUInt(),
        background = background.toArgb().toUInt(),
        cursor = cursor.toArgb().toUInt(),
        palette = ansi.map { it.toArgb().toUInt() },
    )
