package com.nickalie.ngterm.android.ui.settings

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.Font
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.FontChoice
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.TerminalPalette

// the ANSI colors the sample uses, by index
private const val RED = 1
private const val GREEN = 2
private const val YELLOW = 3
private const val BLUE = 4
private const val MAGENTA = 5
private const val CYAN = 6

/** The terminal's line height for its text size: JetBrains Mono's ascent to descent, about 1.32 of the size. */
private const val LINE_HEIGHT = 1.32f

private val JetBrainsMono =
    FontFamily(
        Font(R.font.jetbrains_mono_regular),
        Font(R.font.jetbrains_mono_bold, FontWeight.Bold),
    )

/**
 * A few lines of a shell as the terminal would draw them in [palette], [font] and [textSize] (whole sp, following the
 * font scale as the terminal does): a prompt, a listing, `git status` with one line selected, and the cursor. Lines
 * wider than the plate are cut, as a terminal's are. TalkBack reads it as [description].
 */
@Composable
internal fun TerminalPreview(
    palette: TerminalPalette,
    font: FontChoice,
    textSize: Int,
    description: String,
    modifier: Modifier = Modifier,
) {
    val family = if (font == FontChoice.JetBrainsMono) JetBrainsMono else FontFamily.Monospace
    val style =
        TextStyle(
            fontFamily = family,
            fontSize = textSize.sp,
            lineHeight = (textSize * LINE_HEIGHT).sp,
            color = palette.foreground,
        )
    Text(
        text = sample(palette),
        style = style,
        softWrap = false,
        overflow = TextOverflow.Clip,
        modifier =
            modifier
                .fillMaxWidth()
                .clip(MaterialTheme.shapes.medium)
                .background(palette.background)
                .padding(NgtermTheme.spacing.md)
                .clearAndSetSemantics { contentDescription = description },
    )
}

private fun sample(palette: TerminalPalette): AnnotatedString {
    fun ansi(index: Int) = SpanStyle(color = palette.ansi[index])
    val bold = FontWeight.Bold
    return buildAnnotatedString {
        withStyle(ansi(GREEN)) { append("nick@build-01") }
        append(" ")
        withStyle(ansi(BLUE)) { append("~/docs-site") }
        append(" ")
        withStyle(ansi(MAGENTA)) { append("(main)") }
        append("\n$ ls\n")
        withStyle(ansi(BLUE).copy(fontWeight = bold)) { append("site/") }
        append("  README.md  ")
        withStyle(ansi(GREEN).copy(fontWeight = bold)) { append("build.sh") }
        append("  ")
        withStyle(ansi(CYAN)) { append("latest") }
        append("\n$ git status --short\n")
        withStyle(ansi(RED)) { append(" M") }
        append(" ")
        withStyle(SpanStyle(background = palette.selectionBackground)) { append("site/install.md") }
        append("\n")
        withStyle(ansi(YELLOW)) { append("??") }
        append(" site/android.md\n$ ")
        withStyle(SpanStyle(color = palette.background, background = palette.cursor)) { append(" ") }
    }
}

/** [palette]'s ground with its eight normal ANSI colors on it, as a strip at a palette's choice. */
@Composable
internal fun PaletteSwatch(
    palette: TerminalPalette,
    modifier: Modifier = Modifier,
) {
    Canvas(modifier.size(SwatchWidth, SwatchHeight)) {
        drawRoundRect(palette.background, cornerRadius = CornerRadius(SwatchRadius.toPx()))
        val inset = SwatchInset.toPx()
        val cell = (size.width - inset * 2) / NORMAL_COLORS
        for (index in 0 until NORMAL_COLORS) {
            drawRect(
                color = palette.ansi[index],
                topLeft = Offset(inset + index * cell, inset),
                size = Size(cell, size.height - inset * 2),
            )
        }
    }
}

private const val NORMAL_COLORS = 8
private val SwatchWidth = 88.dp
private val SwatchHeight = 20.dp
private val SwatchInset = 4.dp
private val SwatchRadius = 4.dp
