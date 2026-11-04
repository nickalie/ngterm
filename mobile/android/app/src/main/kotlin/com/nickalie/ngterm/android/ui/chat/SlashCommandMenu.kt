package com.nickalie.ngterm.android.ui.chat

import androidx.annotation.StringRes
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.AgentCommand
import com.nickalie.ngterm.core.ffi.AgentCommandKind

/**
 * The `/` menu, over the composer: the commands matching what follows the `/`, best first, about [VISIBLE_ROWS] in
 * sight and the rest a scroll away, each row the command as it goes in (`/name`, its argument hint after it), a line
 * of what it does and where it comes from; the highlighted row (Enter or Tab picks it) on a deeper tone. Under them
 * a quiet line says how many match, or that none does, or that [agent]'s commands are on their way or not listed
 * (never an error: the prompt still goes as typed), announced politely. Rows take no input focus, so a hardware
 * keyboard stays in the field; TalkBack reads each row whole and names its tap Insert.
 */
@Composable
fun SlashCommandMenu(
    menu: SlashMenu,
    agent: String?,
    onPick: (AgentCommand) -> Unit,
    modifier: Modifier = Modifier,
) {
    val list = rememberLazyListState()
    KeepHighlightInSight(list, menu)
    Surface(
        modifier = modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.medium,
    ) {
        Column {
            if (menu is SlashMenu.Commands && menu.commands.isNotEmpty()) {
                LazyColumn(
                    state = list,
                    modifier = Modifier.heightIn(max = RowHeight * VISIBLE_ROWS),
                ) {
                    itemsIndexed(menu.commands) { index, command ->
                        CommandRow(command, highlighted = index == menu.highlighted, onPick = { onPick(command) })
                    }
                }
            }
            MenuLine(menu, agent)
        }
    }
}

// about how many rows the menu shows before it scrolls, each a list row's 56dp at least
private const val VISIBLE_ROWS = 6
private val RowHeight = 56.dp

// the list starts at its best match as the query changes, and scrolls the highlighted row into sight
@Composable
private fun KeepHighlightInSight(
    list: LazyListState,
    menu: SlashMenu,
) {
    val highlighted = (menu as? SlashMenu.Commands)?.highlighted ?: 0
    LaunchedEffect(list, menu.query) { list.scrollToItem(0) }
    LaunchedEffect(list, highlighted) {
        val shown = list.layoutInfo.visibleItemsInfo
        val whole =
            shown.any { item ->
                item.index == highlighted &&
                    item.offset >= list.layoutInfo.viewportStartOffset &&
                    item.offset + item.size <= list.layoutInfo.viewportEndOffset
            }
        if (!whole && shown.isNotEmpty()) {
            val first = shown.first().index
            if (highlighted < first) {
                list.scrollToItem(highlighted)
            } else {
                list.scrollToItem((highlighted - shown.size + 2).coerceAtLeast(0))
            }
        }
    }
}

@Composable
private fun CommandRow(
    command: AgentCommand,
    highlighted: Boolean,
    onPick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    val insert = stringResource(R.string.chat_command_insert)
    val quiet = MaterialTheme.colorScheme.onSurfaceVariant
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .heightIn(min = RowHeight)
                .background(if (highlighted) MaterialTheme.colorScheme.surfaceContainerHighest else Color.Transparent)
                .focusProperties { canFocus = false }
                .clickable(onClickLabel = insert, role = Role.Button, onClick = onPick)
                .semantics { selected = highlighted }
                .padding(horizontal = spacing.lg, vertical = spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(spacing.md),
    ) {
        Column(Modifier.weight(1f)) {
            Row(horizontalArrangement = Arrangement.spacedBy(spacing.sm)) {
                Text(
                    insertion(command),
                    style = ChatType.title,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f, fill = false),
                )
                command.argumentHint?.takeIf { it.isNotBlank() }?.let {
                    Text(it, style = ChatType.title, color = quiet, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            command.description.takeIf { it.isNotBlank() }?.let {
                Text(
                    it,
                    style = MaterialTheme.typography.bodySmall,
                    color = quiet,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
        Text(stringResource(command.kind.word), style = MaterialTheme.typography.labelMedium, color = quiet)
    }
}

// the menu's foot: how many match, none, or the catalog on its way or not listed; announced politely
@Composable
private fun MenuLine(
    menu: SlashMenu,
    agent: String?,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    val name = stringResource(agentName(agent))
    val text =
        when {
            menu is SlashMenu.Commands && menu.commands.isEmpty() -> {
                stringResource(R.string.chat_commands_none_match, "/${menu.query}")
            }

            menu is SlashMenu.Commands -> {
                pluralStringResource(R.plurals.chat_commands_count, menu.commands.size, menu.commands.size)
            }

            menu is SlashMenu.Waiting && menu.fetching -> {
                stringResource(R.string.chat_commands_loading, name)
            }

            else -> {
                stringResource(R.string.chat_commands_unlisted, name)
            }
        }
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .heightIn(min = spacing.touchTarget)
                .semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite }
                .padding(horizontal = spacing.lg, vertical = spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(spacing.sm),
    ) {
        if (menu is SlashMenu.Waiting && menu.fetching) {
            CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
        }
        Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** The word a command's kind shows at the end of its row. */
@get:StringRes
val AgentCommandKind.word: Int
    get() =
        when (this) {
            AgentCommandKind.BUILTIN -> R.string.chat_command_builtin
            AgentCommandKind.SKILL -> R.string.chat_command_skill
            AgentCommandKind.COMMAND -> R.string.chat_command_custom
            AgentCommandKind.PLUGIN -> R.string.chat_command_plugin
        }
