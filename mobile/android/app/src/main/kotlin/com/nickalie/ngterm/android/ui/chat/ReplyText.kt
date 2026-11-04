package com.nickalie.ngterm.android.ui.chat

import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mikepenz.markdown.m3.Markdown
import com.mikepenz.markdown.m3.markdownColor
import com.mikepenz.markdown.m3.markdownTypography
import com.mikepenz.markdown.model.markdownAnimations
import com.mikepenz.markdown.model.markdownDimens
import com.mikepenz.markdown.model.rememberMarkdownState
import com.nickalie.ngterm.android.ui.theme.TerminalFamily

/**
 * The agent's reply: its Markdown, full width on the ground with no bubble. Paragraphs and lists in bodyLarge,
 * headings at the title sizes (never the display ones: a reply is not a page), links copper and underlined, code in
 * the terminal's face on a `surfaceContainerLow` plate with no highlighting. It is parsed as it is first drawn, so a
 * row has its height at once and the list keeps its place; a newer text replaces it once parsed.
 */
@Composable
fun ReplyText(
    text: String,
    modifier: Modifier = Modifier,
) {
    val colors = MaterialTheme.colorScheme
    val type = MaterialTheme.typography
    val state = rememberMarkdownState(text, retainState = true, immediate = true)
    Markdown(
        markdownState = state,
        colors =
            markdownColor(
                text = colors.onSurface,
                codeBackground = colors.surfaceContainerLow,
                inlineCodeBackground = colors.surfaceContainerHigh,
                dividerColor = colors.outlineVariant,
                tableBackground = colors.surfaceContainerLow,
            ),
        typography =
            markdownTypography(
                h1 = type.titleLarge,
                h2 = type.titleMedium,
                h3 = type.titleSmall,
                h4 = type.titleSmall,
                h5 = type.titleSmall,
                h6 = type.titleSmall,
                text = type.bodyLarge,
                code = ChatType.code,
                inlineCode = type.bodyLarge.copy(fontFamily = TerminalFamily, fontSize = 14.sp),
                quote = type.bodyLarge.copy(color = colors.onSurfaceVariant),
                paragraph = type.bodyLarge,
                ordered = type.bodyLarge,
                bullet = type.bodyLarge,
                list = type.bodyLarge,
                textLink =
                    TextLinkStyles(SpanStyle(color = colors.primary, textDecoration = TextDecoration.Underline)),
                table = type.bodyMedium,
            ),
        modifier = modifier.fillMaxWidth(),
        dimens = markdownDimens(codeBackgroundCornerSize = 6.dp, tableCornerSize = 6.dp),
        // nothing in a reply loads late (images aren't fetched), so nothing resizes to animate
        animations = markdownAnimations(animateTextSize = { this }),
    )
}
