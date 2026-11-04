package com.nickalie.ngterm.android.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatNoticeKind
import com.nickalie.ngterm.core.ffi.ChatQuestion

/**
 * A prompt the user sent: a bubble at the end of the line on the container tone, at most most of the width, with how
 * many images went with it. TalkBack reads it as "You: …", since its side of the screen is what says whose it is.
 */
@Composable
fun PromptBubble(
    text: String,
    images: Int,
    modifier: Modifier = Modifier,
) {
    val spoken = stringResource(R.string.chat_you_said, text)
    Box(modifier.fillMaxWidth(), contentAlignment = Alignment.CenterEnd) {
        Column(
            modifier =
                Modifier
                    .padding(start = NgtermTheme.spacing.xxxl)
                    .widthIn(max = BubbleWidth)
                    .background(MaterialTheme.colorScheme.surfaceContainerHigh, MaterialTheme.shapes.medium)
                    .padding(horizontal = NgtermTheme.spacing.md, vertical = NgtermTheme.spacing.sm)
                    .clearAndSetSemantics { contentDescription = spoken },
        ) {
            Text(text.trim(), style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurface)
            if (images > 0) {
                Text(
                    pluralStringResource(R.plurals.chat_images, images, images),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            }
        }
    }
}

// the widest a prompt's bubble grows: a reading line's measure, as the text blocks of DESIGN.md
private val BubbleWidth = 480.dp

/**
 * What the agent thought, collapsed to one quiet line: Claude writes it empty ("Thinking" then, which opens nothing),
 * Codex a summary title, which a tap shows whole when it doesn't fit.
 */
@Composable
fun ThinkingRow(
    text: String,
    expanded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val words = text.replace("**", "").trim()
    val toggle =
        if (words.isEmpty()) {
            null
        } else {
            Toggle(expanded, stringResource(R.string.chat_show_more), stringResource(R.string.chat_show_less), onToggle)
        }
    ChatLine(
        icon = R.drawable.ic_thinking,
        iconLabel = null,
        title = words.ifEmpty { stringResource(R.string.chat_thinking) },
        modifier = modifier,
        titleStyle = MaterialTheme.typography.bodyMedium,
        titleColor = MaterialTheme.colorScheme.onSurfaceVariant,
        toggle = toggle,
    )
}

/**
 * Something that happened to the conversation, as a quiet line with its glyph: an interrupt, a compaction (a tap shows
 * its summary when the transcript has it), an API error in the error color, a summary.
 */
@Composable
fun NoticeRow(
    kind: ChatNoticeKind,
    text: String,
    detail: String?,
    expanded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val error = kind == ChatNoticeKind.API_ERROR
    val color = if (error) NgtermTheme.status.error else MaterialTheme.colorScheme.onSurfaceVariant
    val toggle =
        detail?.takeIf { it.isNotBlank() }?.let {
            Toggle(
                expanded,
                stringResource(R.string.chat_show_summary),
                stringResource(R.string.chat_hide_summary),
                onToggle,
            )
        }
    ChatLine(
        icon = kind.icon,
        iconLabel = stringResource(kind.word),
        title = text.ifBlank { stringResource(kind.word) },
        modifier = modifier,
        titleStyle = MaterialTheme.typography.bodyMedium,
        titleColor = color,
        iconTint = color,
        toggle = toggle,
    ) {
        detail?.let {
            Text(it, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

private val ChatNoticeKind.icon: Int
    get() =
        when (this) {
            ChatNoticeKind.COMPACTION -> R.drawable.ic_tool_compact
            ChatNoticeKind.INTERRUPT -> R.drawable.ic_tool_stop
            ChatNoticeKind.API_ERROR -> R.drawable.ic_alert
            ChatNoticeKind.SUMMARY -> R.drawable.ic_tool_read
        }

private val ChatNoticeKind.word: Int
    get() =
        when (this) {
            ChatNoticeKind.COMPACTION -> R.string.chat_notice_compaction
            ChatNoticeKind.INTERRUPT -> R.string.chat_notice_interrupt
            ChatNoticeKind.API_ERROR -> R.string.chat_notice_api_error
            ChatNoticeKind.SUMMARY -> R.string.chat_notice_summary
        }

/**
 * An `AskUserQuestion` in the conversation: its first question, and its status (Needs you while it waits, its card
 * pinned at the chat's foot); a tap shows every question and the answer the agent got.
 */
@Composable
fun QuestionRow(
    question: ChatQuestion,
    expanded: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val first =
        question.questions
            .firstOrNull()
            ?.question
            .orEmpty()
    ChatLine(
        icon = R.drawable.ic_chat,
        iconLabel = stringResource(R.string.chat_kind_question),
        title = first.ifBlank { stringResource(R.string.chat_kind_question) },
        modifier = modifier,
        titleStyle = MaterialTheme.typography.bodyMedium,
        status = question.status.questionLook(),
        toggle =
            Toggle(
                expanded,
                stringResource(R.string.chat_show_answer),
                stringResource(R.string.chat_hide_answer),
                onToggle,
            ),
    ) {
        question.questions.forEach { item ->
            Text(
                item.question,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurface,
            )
        }
        question.output?.takeIf { it.isNotBlank() }?.let { OutputPlate(it) }
    }
}

/** The pane's conversation switched here: "New conversation" and a rule, what came before it gone. */
@Composable
fun ConversationDivider(modifier: Modifier = Modifier) {
    Row(
        modifier = modifier.fillMaxWidth().heightIn(min = NgtermTheme.spacing.touchTarget),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.md),
    ) {
        Text(
            stringResource(R.string.chat_new_conversation),
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.semantics { heading() },
        )
        Box(Modifier.weight(1f).height(1.dp).background(MaterialTheme.colorScheme.outlineVariant))
    }
}
