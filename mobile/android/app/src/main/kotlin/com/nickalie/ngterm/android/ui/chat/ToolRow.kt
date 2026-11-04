package com.nickalie.ngterm.android.ui.chat

import androidx.annotation.DrawableRes
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.StatusGlyph
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatTool
import com.nickalie.ngterm.core.ffi.ChatToolKind

/**
 * One line of the chat that isn't prose: a glyph ([icon], read as [iconLabel], or decorative when that is `null`), a
 * one-line [title] in [titleStyle], an optional [subtitle] under it, and an optional [status] (glyph and word) at the
 * end. With a [toggle] the whole line is a 48dp target that opens [details] under it, indented to the title, and
 * TalkBack reads whether it is open and what a tap does.
 */
@Composable
fun ChatLine(
    @DrawableRes icon: Int,
    iconLabel: String?,
    title: String,
    modifier: Modifier = Modifier,
    titleStyle: TextStyle = ChatType.title,
    titleColor: Color = MaterialTheme.colorScheme.onSurface,
    iconTint: Color = MaterialTheme.colorScheme.onSurfaceVariant,
    subtitle: String? = null,
    status: StatusLook? = null,
    toggle: Toggle? = null,
    details: @Composable () -> Unit = {},
) {
    val spacing = NgtermTheme.spacing
    val expanded = toggle?.expanded == true
    val collapsedWord = stringResource(R.string.chat_collapsed)
    val expandedWord = stringResource(R.string.chat_expanded)
    val tap =
        if (toggle == null) {
            Modifier
        } else {
            Modifier
                .clickable(
                    onClickLabel = if (expanded) toggle.hide else toggle.show,
                    role = Role.Button,
                    onClick = toggle.onToggle,
                ).semantics { stateDescription = if (expanded) expandedWord else collapsedWord }
        }
    Column(modifier.fillMaxWidth()) {
        Row(
            modifier =
                Modifier
                    .fillMaxWidth()
                    .heightIn(min = spacing.touchTarget)
                    .clip(MaterialTheme.shapes.small)
                    .then(tap)
                    .semantics(mergeDescendants = true) {}
                    .padding(horizontal = spacing.xs, vertical = spacing.xs),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(spacing.sm),
        ) {
            Icon(
                painterResource(icon),
                contentDescription = iconLabel,
                tint = iconTint,
                modifier = Modifier.size(18.dp),
            )
            LineTitle(title, titleStyle, titleColor, subtitle, expanded, Modifier.weight(1f))
            status?.let { StatusWord(it) }
        }
        AnimatedVisibility(visible = expanded) {
            Column(
                modifier = Modifier.padding(start = DetailsIndent, bottom = spacing.sm),
                verticalArrangement = Arrangement.spacedBy(spacing.sm),
            ) {
                details()
            }
        }
    }
}

// a line's title, cut to one line until the line is open, with its subtitle under it
@Composable
private fun LineTitle(
    title: String,
    style: TextStyle,
    color: Color,
    subtitle: String?,
    expanded: Boolean,
    modifier: Modifier = Modifier,
) {
    Column(modifier) {
        Text(
            text = title,
            style = style,
            color = color,
            maxLines = if (expanded) Int.MAX_VALUE else 1,
            overflow = TextOverflow.Ellipsis,
        )
        subtitle?.let {
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

// the room before a line's title (its padding, glyph and gap), where what it opens lines up
private val DetailsIndent = 30.dp

@Composable
private fun StatusWord(
    look: StatusLook,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier = modifier,
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.xs),
    ) {
        StatusGlyph(look.shape, look.color)
        Text(stringResource(look.word), style = MaterialTheme.typography.labelMedium, color = look.color)
    }
}

/**
 * A tool call's line: its kind's glyph, its title in the terminal's face (its name when the title isn't known), and
 * its status. When it has a diff, output or a subagent's [steps] (a task), a tap opens them: the diff, the output, a
 * note when either was cut, then the steps behind a rule.
 */
@Composable
fun ToolRow(
    tool: ChatTool,
    expanded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
    steps: Int = 0,
    children: @Composable () -> Unit = {},
) {
    val title = tool.title.ifBlank { tool.name }.ifBlank { stringResource(R.string.chat_tool_untitled) }
    val opens = tool.diff != null || tool.output != null || steps > 0
    val toggle =
        if (!opens) {
            null
        } else if (tool.kind == ChatToolKind.TASK) {
            Toggle(
                expanded,
                stringResource(R.string.chat_show_steps),
                stringResource(R.string.chat_hide_steps),
                onToggle,
            )
        } else {
            Toggle(
                expanded,
                stringResource(R.string.chat_show_output),
                stringResource(R.string.chat_hide_output),
                onToggle,
            )
        }
    ChatLine(
        icon = tool.kind.icon,
        iconLabel = stringResource(tool.kind.word),
        title = title,
        modifier = modifier,
        subtitle = if (steps > 0) pluralStringResource(R.plurals.chat_steps, steps, steps) else null,
        status = tool.status.look(),
        toggle = toggle,
    ) {
        tool.diff?.let { DiffView(it) }
        tool.output?.takeIf { it.isNotEmpty() }?.let { OutputPlate(it) }
        if (tool.truncated) Note(stringResource(R.string.chat_truncated))
        if (steps > 0) Steps(content = children)
    }
}

/**
 * A subagent's steps whose task is older than what the chat has read: a task line with no status of its own, its
 * steps under it.
 */
@Composable
fun SubagentRow(
    steps: Int,
    expanded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
    children: @Composable () -> Unit = {},
) {
    ChatLine(
        icon = ChatToolKind.TASK.icon,
        iconLabel = stringResource(ChatToolKind.TASK.word),
        title = stringResource(R.string.chat_subagent_earlier),
        modifier = modifier,
        titleStyle = MaterialTheme.typography.bodyMedium,
        subtitle = pluralStringResource(R.plurals.chat_steps, steps, steps),
        toggle =
            Toggle(
                expanded,
                stringResource(R.string.chat_show_steps),
                stringResource(R.string.chat_hide_steps),
                onToggle,
            ),
    ) {
        Steps(content = children)
    }
}

// a subagent's steps, behind a rule at their start: drawn beside the steps, not measured with them, since a step can
// be a reply whose Markdown table answers no intrinsic size
@Composable
private fun Steps(
    modifier: Modifier = Modifier,
    content: @Composable () -> Unit,
) {
    val rule = MaterialTheme.colorScheme.outlineVariant
    Column(
        modifier
            .fillMaxWidth()
            .drawBehind {
                val width = RuleWidth.toPx()
                val x = if (layoutDirection == LayoutDirection.Rtl) size.width - width else 0f
                drawRect(rule, topLeft = Offset(x, 0f), size = Size(width, size.height))
            }.padding(start = RuleWidth + NgtermTheme.spacing.sm),
    ) { content() }
}

// the rule a subagent's steps sit behind
private val RuleWidth = 2.dp

/** A tool's output on a plate, in the terminal's face, scrolling sideways as the terminal would cut it. */
@Composable
fun OutputPlate(
    text: String,
    modifier: Modifier = Modifier,
) {
    Text(
        text = text.trimEnd('\n'),
        style = ChatType.output,
        color = MaterialTheme.colorScheme.onSurface,
        softWrap = false,
        modifier =
            modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surfaceContainerLow, MaterialTheme.shapes.small)
                .horizontalScroll(rememberScrollState())
                .padding(NgtermTheme.spacing.md),
    )
}

@Composable
private fun Note(
    text: String,
    modifier: Modifier = Modifier,
) {
    Text(
        text = text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier,
    )
}
