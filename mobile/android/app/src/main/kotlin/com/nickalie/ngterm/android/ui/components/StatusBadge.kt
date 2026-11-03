package com.nickalie.ngterm.android.ui.components

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.ui.theme.AgentState
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusShape
import com.nickalie.ngterm.android.ui.theme.StatusTone
import com.nickalie.ngterm.android.ui.theme.parseHexColor
import com.nickalie.ngterm.android.ui.theme.readableOn
import com.nickalie.ngterm.android.ui.theme.textGrounds

/** A status's glyph alone, in [color]. Decorative: the word next to it is what TalkBack reads. */
@Composable
fun StatusGlyph(
    shape: StatusShape,
    color: Color,
    modifier: Modifier = Modifier,
    size: Dp = 12.dp,
) {
    Icon(
        painter = painterResource(shape.drawable),
        contentDescription = null,
        tint = color,
        modifier = modifier.size(size),
    )
}

/** One of the app's statuses: its glyph and its word, both in its color, read by TalkBack as the word. */
@Composable
fun StatusBadge(
    tone: StatusTone,
    modifier: Modifier = Modifier,
    label: String = stringResource(tone.label),
) {
    GlyphAndLabel(tone.shape, tone.color(NgtermTheme.status), label, modifier)
}

/**
 * An agent's status as its origin sets it: [label] (by default the word for [status], the origin's wire name:
 * [agentStatusWord]) after the glyph [agentStatusLook] resolves. By default [backgrounds] are every surface up to
 * `surfaceContainerHigh`, so the badge reads on the ground, a [MachineCard] or a sidebar [SessionRow] alike.
 */
@Composable
fun AgentStatusBadge(
    status: String,
    colorHex: String?,
    shape: String?,
    modifier: Modifier = Modifier,
    backgrounds: List<Color> = MaterialTheme.colorScheme.textGrounds(),
    label: String = agentStatusWord(status),
) {
    val look = agentStatusLook(status, colorHex, shape, backgrounds)
    GlyphAndLabel(look.shape, look.color, label, modifier)
}

/** The word for an agent's [status] as its origin names it ([AgentState]); a status this build doesn't know as is. */
@Composable
fun agentStatusWord(status: String): String = AgentState.fromWire(status)?.let { stringResource(it.word) } ?: status

/** The glyph and color an agent status is drawn with. */
data class AgentStatusLook(
    val shape: StatusShape,
    val color: Color,
)

/**
 * How an agent's [status] is drawn: the shape and the `#rrggbb` color its origin sets ([shape], [colorHex]), the color
 * made readable on every one of [backgrounds] (an origin's color is the user's choice and may not contrast; the word
 * is always there besides). What the origin leaves unset comes from the status's [AgentState], then the circle and the
 * idle color.
 */
@Composable
fun agentStatusLook(
    status: String?,
    colorHex: String?,
    shape: String?,
    backgrounds: List<Color> = MaterialTheme.colorScheme.textGrounds(),
): AgentStatusLook {
    val state = AgentState.fromWire(status)
    val color =
        parseHexColor(colorHex)?.let { readableOn(it, backgrounds) }
            ?: (state?.color(NgtermTheme.status) ?: NgtermTheme.status.idle)
    val glyph = if (shape.isNullOrBlank()) state?.shape ?: StatusShape.Circle else StatusShape.fromWire(shape)
    return AgentStatusLook(glyph, color)
}

@Composable
private fun GlyphAndLabel(
    shape: StatusShape,
    color: Color,
    label: String,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier.clearAndSetSemantics { contentDescription = label },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.xs + NgtermTheme.spacing.xxs),
    ) {
        StatusGlyph(shape, color)
        Text(
            text = label,
            color = color,
            style = MaterialTheme.typography.labelMedium,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
