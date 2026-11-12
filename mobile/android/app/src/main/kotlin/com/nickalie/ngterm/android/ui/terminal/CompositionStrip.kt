package com.nickalie.ngterm.android.ui.terminal

import android.text.TextDirectionHeuristics
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.TerminalFamily

/**
 * What waits to be typed ([TerminalState.composing]): a word the soft keyboard holds until it is done, or a dead key's
 * accent until the next key. One line of [text] in the terminal's face, underlined in copper as an input method's text
 * is until it goes, on the ground of the place its host gives it, which is never a cell of the grid: the extra keys'
 * top line ([ExtraKeysRow]), or the session bar's title with no row. Past the width it scrolls, the text's end in view
 * whichever way the text runs, [inset] in from its sides.
 */
@Composable
fun CompositionStrip(
    text: String,
    modifier: Modifier = Modifier,
    inset: Dp = NgtermTheme.spacing.lg,
) {
    val description = stringResource(R.string.composition_strip_description, text)
    val direction = textDirection(text, LocalLayoutDirection.current)
    val scroll = rememberScrollState()
    LaunchedEffect(scroll.maxValue) { scroll.scrollTo(scroll.maxValue) }
    // a scroll's far end is the layout direction's: the text's own direction puts it where the text ends
    CompositionLocalProvider(LocalLayoutDirection provides direction) {
        Box(
            modifier
                .clearAndSetSemantics { contentDescription = description }
                .horizontalScroll(scroll)
                .padding(horizontal = inset),
            contentAlignment = Alignment.CenterStart,
        ) {
            val underline = MaterialTheme.colorScheme.primary
            Text(
                text = text,
                style = HeldStyle,
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
                softWrap = false,
                modifier =
                    Modifier
                        .drawBehind {
                            val thickness = UnderlineWidth.toPx()
                            drawRect(underline, Offset(0f, size.height - thickness), Size(size.width, thickness))
                        }
                        // under the glyphs, which an emoji fills to the line's foot
                        .padding(bottom = NgtermTheme.spacing.xs),
            )
        }
    }
}

// the first strong letter's direction; text with none (an accent alone) runs as the screen does
private fun textDirection(
    text: String,
    screen: LayoutDirection,
): LayoutDirection {
    val heuristic =
        if (screen ==
            LayoutDirection.Rtl
        ) {
            TextDirectionHeuristics.FIRSTSTRONG_RTL
        } else {
            TextDirectionHeuristics.FIRSTSTRONG_LTR
        }
    return if (heuristic.isRtl(text, 0, text.length)) LayoutDirection.Rtl else LayoutDirection.Ltr
}

private val HeldStyle = TextStyle(fontFamily = TerminalFamily, fontSize = 18.sp, lineHeight = 24.sp)
private val UnderlineWidth = 2.dp
