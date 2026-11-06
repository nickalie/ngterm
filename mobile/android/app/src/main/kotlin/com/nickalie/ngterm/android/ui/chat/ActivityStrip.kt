package com.nickalie.ngterm.android.ui.chat

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatTool
import com.nickalie.ngterm.core.ffi.ChatToolKind

/**
 * What the agent still has on its way, as one quiet line on the ground just over the composer, so a finished reply
 * isn't read as the work being done: "Claude Code is working" while its status says it works on a turn ([working];
 * [agent] is its name), else "Still running in the background" while calls of the conversation have no result yet
 * ([running]: a subagent or a command its turn left going), and after it how many of each; with any running a tap
 * lists them under it, each as its tool line. While the turn can be interrupted ([onStop] given) Stop, a word in
 * copper, ends the line, held while an action is on its way ([stopEnabled]). No plate, no second line: the composer is
 * what the foot is for. Nothing to say shows nothing. The Running ring stands still: nothing at the foot moves on its
 * own.
 */
@Composable
fun ActivityStrip(
    working: Boolean,
    running: List<ChatTool>,
    agent: String,
    modifier: Modifier = Modifier,
    onStop: (() -> Unit)? = null,
    stopEnabled: Boolean = true,
) {
    if (!working && running.isEmpty()) return
    var expanded by rememberSaveable { mutableStateOf(false) }
    val title =
        if (working) {
            stringResource(R.string.chat_activity_working, agent)
        } else {
            stringResource(R.string.chat_activity_background)
        }
    val listed = running.isNotEmpty()
    Column(modifier) {
        Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
            ActivityLabel(
                title = title,
                counts = running.takeIf { listed }?.let { counts(it) },
                toggle =
                    if (listed) {
                        Toggle(
                            expanded,
                            stringResource(R.string.chat_activity_show),
                            stringResource(R.string.chat_activity_hide),
                        ) { expanded = !expanded }
                    } else {
                        null
                    },
                modifier = Modifier.weight(1f),
            )
            onStop?.let { StopButton(enabled = stopEnabled, onClick = it) }
        }
        AnimatedVisibility(visible = expanded && listed) {
            Column(
                modifier = Modifier.padding(start = RingInset, bottom = NgtermTheme.spacing.xs),
                verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.xs),
            ) {
                running.forEach { tool ->
                    ChatLine(
                        icon = tool.kind.icon,
                        iconLabel = stringResource(tool.kind.word),
                        title =
                            tool.title.ifBlank { tool.name }.ifBlank { stringResource(R.string.chat_tool_untitled) },
                    )
                }
            }
        }
    }
}

// the line itself: the still Running ring over the paperclip's middle, what the agent is doing and how many run; with
// a [toggle] the whole of it is a 48dp target that lists them, and TalkBack reads whether the list is open
@Composable
private fun ActivityLabel(
    title: String,
    counts: String?,
    toggle: Toggle?,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    val collapsedWord = stringResource(R.string.chat_collapsed)
    val expandedWord = stringResource(R.string.chat_expanded)
    val tap =
        if (toggle == null) {
            Modifier
        } else {
            Modifier
                .clickable(
                    onClickLabel = if (toggle.expanded) toggle.hide else toggle.show,
                    role = Role.Button,
                    onClick = toggle.onToggle,
                ).semantics { stateDescription = if (toggle.expanded) expandedWord else collapsedWord }
        }
    Row(
        modifier =
            modifier
                .heightIn(min = spacing.touchTarget)
                .clip(MaterialTheme.shapes.small)
                .then(tap)
                .semantics(mergeDescendants = true) {}
                .padding(start = RingInset, end = spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(spacing.sm),
    ) {
        Icon(
            painterResource(R.drawable.ic_status_ring),
            contentDescription = null,
            tint = NgtermTheme.status.connecting,
            modifier = Modifier.size(RingSize),
        )
        Text(
            text = title,
            style = MaterialTheme.typography.labelLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        // the title says which of the two states this is, so the counts give way to it
        counts?.let {
            Text(
                text = it,
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f, fill = false),
            )
        }
    }
}

// at the line's end while the turn can be interrupted: the word, in copper, as the line's one action
@Composable
private fun StopButton(
    enabled: Boolean,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    TextButton(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier,
        contentPadding = PaddingValues(horizontal = NgtermTheme.spacing.md),
    ) {
        Text(stringResource(R.string.chat_stop), style = MaterialTheme.typography.labelLarge)
    }
}

// the ring's size, and its inset from the foot's edge that puts its middle over the composer's paperclip (the pill
// starts 8dp in from the same edge, the paperclip's 48dp button at its start)
private val RingSize = 12.dp
private val RingInset = 18.dp

// how many subagents, commands and other tools run, in that order, the ones with none left out
@Composable
private fun counts(running: List<ChatTool>): String {
    val subagents = running.count { it.kind == ChatToolKind.TASK }
    val commands = running.count { it.kind == ChatToolKind.SHELL }
    val tools = running.size - subagents - commands
    return listOfNotNull(
        subagents.takeIf { it > 0 }?.let { pluralStringResource(R.plurals.chat_activity_subagents, it, it) },
        commands.takeIf { it > 0 }?.let { pluralStringResource(R.plurals.chat_activity_commands, it, it) },
        tools.takeIf { it > 0 }?.let { pluralStringResource(R.plurals.chat_activity_tools, it, it) },
    ).joinToString(", ")
}
