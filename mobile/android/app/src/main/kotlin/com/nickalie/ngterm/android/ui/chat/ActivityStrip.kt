package com.nickalie.ngterm.android.ui.chat

import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatTool
import com.nickalie.ngterm.core.ffi.ChatToolKind

/**
 * What the agent still has on its way, pinned at the chat's foot, so a finished reply isn't read as the work being
 * done: "Claude Code is working" while its status says it works on a turn ([working]; [agent] is its name), else
 * "Still running in the background" while calls of the conversation have no result yet ([running]: a subagent or a
 * command its turn left going). Under it, how many of each; a tap lists them, each as its tool line. Nothing to say
 * shows nothing. The Running ring stands still: nothing at the foot moves on its own.
 */
@Composable
fun ActivityStrip(
    working: Boolean,
    running: List<ChatTool>,
    agent: String,
    modifier: Modifier = Modifier,
) {
    if (!working && running.isEmpty()) return
    var expanded by rememberSaveable { mutableStateOf(false) }
    val title =
        if (working) {
            stringResource(R.string.chat_activity_working, agent)
        } else {
            stringResource(R.string.chat_activity_background)
        }
    val toggle =
        running.takeIf { it.isNotEmpty() }?.let {
            Toggle(
                expanded,
                stringResource(R.string.chat_activity_show),
                stringResource(R.string.chat_activity_hide),
            ) { expanded = !expanded }
        }
    Surface(
        modifier = modifier,
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.small,
    ) {
        ChatLine(
            icon = R.drawable.ic_status_ring,
            iconLabel = null,
            title = title,
            modifier = Modifier.padding(horizontal = NgtermTheme.spacing.sm),
            titleStyle = MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.SemiBold),
            iconTint = NgtermTheme.status.connecting,
            subtitle = running.takeIf { it.isNotEmpty() }?.let { counts(it) },
            toggle = toggle,
        ) {
            running.forEach { tool ->
                ChatLine(
                    icon = tool.kind.icon,
                    iconLabel = stringResource(tool.kind.word),
                    title = tool.title.ifBlank { tool.name }.ifBlank { stringResource(R.string.chat_tool_untitled) },
                )
            }
        }
    }
}

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
