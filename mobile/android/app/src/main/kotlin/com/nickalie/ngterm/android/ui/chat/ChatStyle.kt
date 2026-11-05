package com.nickalie.ngterm.android.ui.chat

import androidx.annotation.DrawableRes
import androidx.annotation.StringRes
import androidx.compose.runtime.Composable
import androidx.compose.runtime.ReadOnlyComposable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.unit.sp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusShape
import com.nickalie.ngterm.android.ui.theme.TerminalFamily
import com.nickalie.ngterm.core.ffi.ChatToolKind
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import com.nickalie.ngterm.core.ffi.QuestionAnswer

/** The chat's own text roles: the terminal's face for what the terminal holds (DESIGN.md, The chat). */
object ChatType {
    /** A tool's one line: its command, path, URL or query. */
    val title = TextStyle(fontFamily = TerminalFamily, fontSize = 13.sp, lineHeight = 20.sp)

    /** A code block in a reply, an approval's command. */
    val code = TextStyle(fontFamily = TerminalFamily, fontSize = 13.sp, lineHeight = 18.sp)

    /** A tool's output and a diff. */
    val output = TextStyle(fontFamily = TerminalFamily, fontSize = 12.sp, lineHeight = 16.sp)
}

/** A status as a row of the chat shows it: the glyph, its color and the word that goes with them. */
data class StatusLook(
    val shape: StatusShape,
    val color: Color,
    @param:StringRes val word: Int,
)

/** A tool's status: Running on the Connecting ring, Done on the Online circle, Failed on the Error triangle. */
@Composable
@ReadOnlyComposable
fun ChatToolStatus.look(): StatusLook {
    val status = NgtermTheme.status
    return when (this) {
        ChatToolStatus.RUNNING -> StatusLook(StatusShape.Ring, status.connecting, R.string.chat_tool_running)
        ChatToolStatus.OK -> StatusLook(StatusShape.Circle, status.online, R.string.chat_tool_done)
        ChatToolStatus.ERROR -> StatusLook(StatusShape.Triangle, status.error, R.string.chat_tool_failed)
    }
}

/** A question's status: Needs you while it waits, then Answered, or Failed. */
@Composable
@ReadOnlyComposable
fun ChatToolStatus.questionLook(): StatusLook {
    val status = NgtermTheme.status
    return when (this) {
        ChatToolStatus.RUNNING -> StatusLook(StatusShape.Diamond, status.attention, R.string.status_attention)
        ChatToolStatus.OK -> StatusLook(StatusShape.Circle, status.online, R.string.chat_question_answered)
        ChatToolStatus.ERROR -> StatusLook(StatusShape.Triangle, status.error, R.string.chat_tool_failed)
    }
}

/** The glyph a tool's kind shows at the start of its row. */
@get:DrawableRes
val ChatToolKind.icon: Int
    get() =
        when (this) {
            ChatToolKind.SHELL -> R.drawable.ic_tool_shell
            ChatToolKind.EDIT -> R.drawable.ic_tool_edit
            ChatToolKind.READ -> R.drawable.ic_tool_read
            ChatToolKind.SEARCH -> R.drawable.ic_tool_search
            ChatToolKind.WEB -> R.drawable.ic_tool_web
            ChatToolKind.TASK -> R.drawable.ic_tool_task
            ChatToolKind.OTHER -> R.drawable.ic_tool_other
        }

/** The word TalkBack reads for a tool's kind, the glyph having no word beside it. */
@get:StringRes
val ChatToolKind.word: Int
    get() =
        when (this) {
            ChatToolKind.SHELL -> R.string.chat_kind_shell
            ChatToolKind.EDIT -> R.string.chat_kind_edit
            ChatToolKind.READ -> R.string.chat_kind_read
            ChatToolKind.SEARCH -> R.string.chat_kind_search
            ChatToolKind.WEB -> R.string.chat_kind_web
            ChatToolKind.TASK -> R.string.chat_kind_task
            ChatToolKind.OTHER -> R.string.chat_kind_other
        }

/** The agent's name as people know it, from its wire name (`claude`, `codex`). */
@StringRes
fun agentName(agent: String?): Int =
    when (agent) {
        "claude" -> R.string.chat_agent_claude
        "codex" -> R.string.chat_agent_codex
        else -> R.string.chat_agent_unknown
    }

/**
 * The answers an approval card offers before the pane's screen is read: every dialog of the agent has them, worded as
 * its dialog words them, so a tap finds its option once the screen is read (crates/ngterm-mobile, `fallback_labels`).
 */
fun genericApprovalLabels(agent: String?): List<String> =
    when (agent) {
        "codex" -> listOf("Yes, proceed", "No, and tell Codex what to do differently")
        else -> listOf("Yes", "No")
    }

/**
 * What a chat does: [onLoadOlder] asks for the page before what it shows, [onAnswerApproval] and [onAnswerQuestion]
 * answer its cards (the id of the card drawn, then its answer), [onProblemShown] dismisses the last action's problem,
 * [onShowTerminal] shows the pane's terminal in its place (the bar's Show terminal, and Answer in terminal), [onSend]
 * sends the composer's prompt, [onStop] interrupts the agent's turn, and [onEndShown] says the list went to its newest
 * row ([ChatUiState.toEnd]).
 */
data class ChatActions(
    val onLoadOlder: () -> Unit = {},
    val onAnswerApproval: (String, String) -> Unit = { _, _ -> },
    val onAnswerQuestion: (String, List<QuestionAnswer>) -> Unit = { _, _ -> },
    val onProblemShown: () -> Unit = {},
    val onShowTerminal: () -> Unit = {},
    val onSend: () -> Unit = {},
    val onStop: () -> Unit = {},
    val onEndShown: () -> Unit = {},
)

/** How a line that opens names its tap for TalkBack: what opening it shows, and what closing it hides. */
data class Toggle(
    val expanded: Boolean,
    val show: String,
    val hide: String,
    val onToggle: () -> Unit,
)
