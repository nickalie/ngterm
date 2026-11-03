package com.nickalie.ngterm.android.ui.terminal

import android.content.res.Configuration
import androidx.annotation.StringRes
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.waitForUpOrCancellation
import androidx.compose.foundation.indication
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.BasicText
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ripple
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.AwaitPointerEventScope
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.paneTitle
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.rememberTextMeasurer
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.ExtraKeysChoice

/** What an extra key does. */
internal sealed interface ExtraKeyAction {
    /** Presses the key Ghostty names [name]; [repeats] while held. */
    data class Press(
        val name: String,
        val repeats: Boolean = false,
    ) : ExtraKeyAction

    /** Types [text], as the key that types it on a US keyboard. */
    data class Type(
        val text: String,
    ) : ExtraKeyAction

    /** Arms, locks or lets go of [key]. */
    data class Hold(
        val key: StickyModifier,
    ) : ExtraKeyAction
}

/** How an extra key's legend is drawn. */
internal enum class Legend {
    /** A word, in the chrome's type. */
    Word,

    /** A character compared as itself (`|`, `/`), in mono. */
    Symbol,

    /** An arrow, turned from up by the key's quarter turns. */
    Arrow,
}

/** A key of the extra-keys row: its legend, how TalkBack names it, and what it does. */
internal enum class ExtraKey(
    val legend: String,
    val style: Legend,
    @param:StringRes val description: Int,
    val action: ExtraKeyAction,
    /** An arrow's quarter turns clockwise from up. */
    val turns: Int = 0,
) {
    Esc("Esc", Legend.Word, R.string.key_escape, ExtraKeyAction.Press("escape")),
    Tab("Tab", Legend.Word, R.string.key_tab, ExtraKeyAction.Press("tab")),
    Ctrl("Ctrl", Legend.Word, R.string.key_ctrl, ExtraKeyAction.Hold(StickyModifier.Ctrl)),
    Alt("Alt", Legend.Word, R.string.key_alt, ExtraKeyAction.Hold(StickyModifier.Alt)),
    Slash("/", Legend.Symbol, R.string.key_slash, ExtraKeyAction.Type("/")),
    Pipe("|", Legend.Symbol, R.string.key_pipe, ExtraKeyAction.Type("|")),
    Tilde("~", Legend.Symbol, R.string.key_tilde, ExtraKeyAction.Type("~")),
    Minus("-", Legend.Symbol, R.string.key_minus, ExtraKeyAction.Type("-")),
    Home("Home", Legend.Word, R.string.key_home, ExtraKeyAction.Press("home")),
    End("End", Legend.Word, R.string.key_end, ExtraKeyAction.Press("end")),
    PageUp("PgUp", Legend.Word, R.string.key_page_up, ExtraKeyAction.Press("page_up", repeats = true)),
    PageDown("PgDn", Legend.Word, R.string.key_page_down, ExtraKeyAction.Press("page_down", repeats = true)),
    Left("", Legend.Arrow, R.string.key_left, ExtraKeyAction.Press("arrow_left", repeats = true), turns = 3),
    Up("", Legend.Arrow, R.string.key_up, ExtraKeyAction.Press("arrow_up", repeats = true)),
    Down("", Legend.Arrow, R.string.key_down, ExtraKeyAction.Press("arrow_down", repeats = true), turns = 2),
    Right("", Legend.Arrow, R.string.key_right, ExtraKeyAction.Press("arrow_right", repeats = true), turns = 1),
}

/**
 * The row's two lines on a phone held upright: the arrows an inverted T under the thumb's reach at the end, Home and
 * End over Left and Right, Ctrl and Alt by Tab where the thumb rests at the start.
 */
private val TwoRows =
    listOf(
        listOf(
            ExtraKey.Esc,
            ExtraKey.Slash,
            ExtraKey.Pipe,
            ExtraKey.Tilde,
            ExtraKey.Home,
            ExtraKey.Up,
            ExtraKey.End,
            ExtraKey.PageUp,
        ),
        listOf(
            ExtraKey.Tab,
            ExtraKey.Ctrl,
            ExtraKey.Alt,
            ExtraKey.Minus,
            ExtraKey.Left,
            ExtraKey.Down,
            ExtraKey.Right,
            ExtraKey.PageDown,
        ),
    )

/** The row in one line once every key has 48dp: the keys that hold and send first, the moves last. */
private val OneRow =
    listOf(
        ExtraKey.Esc,
        ExtraKey.Tab,
        ExtraKey.Ctrl,
        ExtraKey.Alt,
        ExtraKey.Slash,
        ExtraKey.Pipe,
        ExtraKey.Tilde,
        ExtraKey.Minus,
        ExtraKey.Home,
        ExtraKey.End,
        ExtraKey.PageUp,
        ExtraKey.PageDown,
        ExtraKey.Left,
        ExtraKey.Up,
        ExtraKey.Down,
        ExtraKey.Right,
    )

/** A key's height, and the least width that puts the row in one line. */
private val KeyHeight = 48.dp
private val OneRowWidth = KeyHeight * OneRow.size

/** The inset of a key's face in its cell, which takes the touch whole. */
private val KeyInset = 2.dp

/** How long a key is held before it repeats, and how often it repeats then. */
private const val REPEAT_DELAY_MS = 400L
private const val REPEAT_INTERVAL_MS = 50L

/** Whether a hardware keyboard is attached and open: it has every key the row has, so the row stands aside. */
@Composable
@ReadOnlyComposable
fun hardwareKeyboardAttached(): Boolean {
    val configuration = LocalConfiguration.current
    return configuration.keyboard != Configuration.KEYBOARD_NOKEYS &&
        configuration.hardKeyboardHidden == Configuration.HARDKEYBOARDHIDDEN_NO
}

/**
 * The keys a phone's soft keyboard lacks, over it: Esc, Tab, a sticky Ctrl and Alt, the arrows, Home and End, PgUp
 * and PgDn, and `/`, `|`, `~` and `-`, going to [keyboard]'s terminal. Two lines of eight on a phone held upright, one
 * line of sixteen once the width gives each 48dp. A key acts as it goes down, with a tick; the arrows and the page
 * keys repeat while held. Ctrl and Alt hold for the next key (outlined in copper), lock on a second tap (filled), and
 * let go on a third. While a hardware keyboard is attached the row is not shown. Settings' [layout] keeps two lines
 * at every width ([ExtraKeysChoice.TwoLines]) or hides the row ([ExtraKeysChoice.Hidden]).
 */
@Composable
fun ExtraKeysRow(
    keyboard: TerminalKeyboard,
    modifier: Modifier = Modifier,
    layout: ExtraKeysChoice = ExtraKeysChoice.Fit,
) {
    if (layout == ExtraKeysChoice.Hidden || hardwareKeyboardAttached()) return
    val description = stringResource(R.string.extra_keys_description)
    val measurer = rememberTextMeasurer()
    val wordStyle = MaterialTheme.typography.titleSmall
    val symbolStyle = TextStyle(fontFamily = TerminalFace, fontSize = SymbolSize, lineHeight = SymbolLine)
    val density = LocalDensity.current
    BoxWithConstraints(
        modifier
            .fillMaxWidth()
            .background(MaterialTheme.colorScheme.surfaceContainer)
            .semantics { paneTitle = description },
    ) {
        val rows = if (layout == ExtraKeysChoice.Fit && maxWidth >= OneRowWidth) listOf(OneRow) else TwoRows
        // every word at one size: the widest legend's, shrunk only as far as its key needs (a large font scale)
        val room = with(density) { (maxWidth / rows[0].size - (KeyInset + LegendPadding) * 2).toPx() }
        val widest =
            OneRow.filter { it.style == Legend.Word }.maxOf {
                measurer
                    .measure(
                        it.legend,
                        wordStyle,
                    ).size.width
            }
        val scale = (room * FIT / widest).coerceIn(0f, 1f)
        val legends = Legends(wordStyle.copy(fontSize = wordStyle.fontSize * scale), symbolStyle)
        Column {
            for (keys in rows) {
                Row(Modifier.fillMaxWidth()) {
                    for (key in keys) ExtraKeyButton(key, keyboard, legends, Modifier.weight(1f))
                }
            }
        }
    }
}

/** The legends' type: words in the chrome's face, at one size across the row; symbols in mono. */
private class Legends(
    val word: TextStyle,
    val symbol: TextStyle,
)

/** How a key looks for where its sticky modifier stands: a face, an edge and the legend's ink. */
private class KeyLook(
    val face: Color,
    val edge: Color,
    val ink: Color,
)

@Composable
private fun keyLook(sticky: Sticky): KeyLook {
    val colors = MaterialTheme.colorScheme
    return when (sticky) {
        Sticky.Off -> KeyLook(Color.Transparent, Color.Transparent, colors.onSurface)
        Sticky.Once -> KeyLook(Color.Transparent, colors.primary, colors.primary)
        Sticky.Locked -> KeyLook(colors.primary, colors.primary, colors.onPrimary)
    }
}

@Composable
private fun ExtraKeyButton(
    key: ExtraKey,
    keyboard: TerminalKeyboard,
    legends: Legends,
    modifier: Modifier = Modifier,
) {
    val haptics = LocalHapticFeedback.current
    val action = key.action
    val sticky = if (action is ExtraKeyAction.Hold) keyboard.sticky(action.key) else Sticky.Off
    val look = keyLook(sticky)
    val ink = look.ink
    val description = stringResource(key.description)
    val stickyWords =
        when (sticky) {
            Sticky.Off -> stringResource(R.string.sticky_off)
            Sticky.Once -> stringResource(R.string.sticky_once)
            Sticky.Locked -> stringResource(R.string.sticky_locked)
        }
    val press by rememberUpdatedState {
        when (action) {
            is ExtraKeyAction.Press -> {
                haptics.performHapticFeedback(HapticFeedbackType.KeyboardTap)
                keyboard.press(action.name)
            }

            is ExtraKeyAction.Type -> {
                haptics.performHapticFeedback(HapticFeedbackType.KeyboardTap)
                keyboard.type(action.text)
            }

            is ExtraKeyAction.Hold -> {
                val now = keyboard.toggle(action.key)
                haptics.performHapticFeedback(
                    if (now == Sticky.Off) HapticFeedbackType.ToggleOff else HapticFeedbackType.ToggleOn,
                )
            }
        }
    }
    val interactions = remember { MutableInteractionSource() }
    val shape = MaterialTheme.shapes.small
    Box(
        modifier
            .height(KeyHeight)
            .clearAndSetSemantics {
                contentDescription = description
                role = Role.Button
                if (action is ExtraKeyAction.Hold) stateDescription = stickyWords
                onClick {
                    press()
                    true
                }
            }.keyPress(interactions, (action as? ExtraKeyAction.Press)?.repeats == true) { press() }
            .padding(KeyInset)
            .clip(shape)
            .background(look.face)
            .border(KeyInset, look.edge, shape)
            .indication(interactions, ripple()),
        contentAlignment = Alignment.Center,
    ) {
        KeyLegend(key, legends, ink)
    }
}

@Composable
private fun KeyLegend(
    key: ExtraKey,
    legends: Legends,
    ink: Color,
) {
    when (key.style) {
        Legend.Arrow -> {
            Icon(
                painter = painterResource(R.drawable.ic_key_arrow),
                contentDescription = null,
                tint = ink,
                modifier = Modifier.size(ArrowSize).rotate(QUARTER_TURN * key.turns),
            )
        }

        Legend.Word, Legend.Symbol -> {
            val style = if (key.style == Legend.Symbol) legends.symbol else legends.word
            BasicText(
                text = key.legend,
                style = style.copy(color = ink, textAlign = TextAlign.Center),
                maxLines = 1,
                softWrap = false,
                modifier = Modifier.padding(horizontal = LegendPadding),
            )
        }
    }
}

private val ArrowSize = 20.dp
private val LegendPadding = 4.dp

/**
 * A symbol's legend: the character as the terminal draws it (its JetBrains Mono), a size up from the words since it
 * is one character.
 */
private val TerminalFace = FontFamily(Font(R.font.jetbrains_mono_regular))
private val SymbolSize = 18.sp
private val SymbolLine = 24.sp

/** The share of a key's room the widest word takes, leaving its glyphs' overhang room. */
private const val FIT = 0.9f
private const val QUARTER_TURN = 90f

/**
 * Runs [onPress] as a finger or a button goes down on the key, showing the press through [interactions]; while held,
 * a key that [repeats] runs it again after a moment and then steadily until let go.
 */
private fun Modifier.keyPress(
    interactions: MutableInteractionSource,
    repeats: Boolean,
    onPress: () -> Unit,
): Modifier =
    pointerInput(interactions, repeats) {
        awaitEachGesture {
            val down = awaitFirstDown()
            down.consume()
            val pressed = PressInteraction.Press(down.position)
            interactions.tryEmit(pressed)
            onPress()
            var lifted = liftedWithin(REPEAT_DELAY_MS)
            while (!lifted) {
                if (repeats) onPress()
                lifted = liftedWithin(REPEAT_INTERVAL_MS)
            }
            interactions.tryEmit(PressInteraction.Release(pressed))
        }
    }

/** Whether the press ends (the finger lifts, or the press is canceled) within [millis]. */
private suspend fun AwaitPointerEventScope.liftedWithin(millis: Long): Boolean =
    withTimeoutOrNull(millis) {
        waitForUpOrCancellation()
        true
    } ?: false
