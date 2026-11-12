package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.TerminalFamily

/**
 * What waits to be typed into [keyboard]'s terminal ([TerminalState.composing]): a word the soft keyboard holds until
 * it is done, or a dead key's accent until the next key. It is a one-line chip off the grid, so the terminal's cells,
 * its touches and its selection stay as they are; the host floats it over the panes' bottom edge, above the extra
 * keys. Nothing shows while nothing waits.
 */
@Composable
fun CompositionStrip(
    keyboard: TerminalKeyboard,
    modifier: Modifier = Modifier,
) {
    val text = keyboard.target?.composing.orEmpty()
    if (text.isNotEmpty()) HeldText(text, modifier)
}

/**
 * [text] in the terminal's face on a `surfaceContainerHigh` chip, underlined in copper as the input method's text is
 * until it goes; past the width it scrolls, its end in view.
 */
@Composable
internal fun HeldText(
    text: String,
    modifier: Modifier = Modifier,
) {
    val description = stringResource(R.string.composition_strip_description, text)
    val shape = MaterialTheme.shapes.medium
    val colors = MaterialTheme.colorScheme
    val scroll = rememberScrollState()
    LaunchedEffect(scroll.maxValue) { scroll.scrollTo(scroll.maxValue) }
    Box(
        modifier
            .clearAndSetSemantics { contentDescription = description }
            .clip(shape)
            .background(colors.surfaceContainerHigh)
            .border(EdgeWidth, colors.outlineVariant, shape)
            .horizontalScroll(scroll)
            .padding(
                start = NgtermTheme.spacing.md,
                end = NgtermTheme.spacing.md,
                top = NgtermTheme.spacing.sm,
                bottom = NgtermTheme.spacing.xs,
            ),
    ) {
        val underline = colors.primary
        Text(
            text = text,
            style = HeldStyle,
            color = colors.onSurface,
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

private val HeldStyle = TextStyle(fontFamily = TerminalFamily, fontSize = 18.sp, lineHeight = 24.sp)
private val EdgeWidth = 1.dp
private val UnderlineWidth = 2.dp
