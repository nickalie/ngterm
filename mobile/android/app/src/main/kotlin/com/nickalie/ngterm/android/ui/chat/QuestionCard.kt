package com.nickalie.ngterm.android.ui.chat

import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.asks.ChoiceRow
import com.nickalie.ngterm.android.ui.asks.Marker
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatQuestionItem
import com.nickalie.ngterm.core.ffi.QuestionAnswer

/**
 * Claude's `AskUserQuestion`, waiting: "Claude Code asks", then each question (its header over it, radio rows for a
 * single pick, checkbox rows for several, each option's label over its description, then the "Type an answer" row),
 * and **Send answers** once every question has an answer. The draft lives with the card's question, and goes with it:
 * another question starts its own, even one in the same place after the app was put away, and a saved draft that
 * doesn't fit the question in front of it is let go.
 */
@Composable
fun QuestionCardView(
    card: OpenQuestion,
    agent: String?,
    busy: Boolean,
    onAnswer: (List<QuestionAnswer>) -> Unit,
    modifier: Modifier = Modifier,
) {
    // keyed by the question, so state saved for another one sits under another key and is never read back here
    key(card.id) {
        val saver = remember(card) { draftSaver(card) }
        var draft by rememberSaveable(card.id, saver = saver) { mutableStateOf(QuestionDraft.of(card.question)) }
        val title = stringResource(R.string.chat_asks, stringResource(agentName(agent)))
        NeedsYouCard(title, modifier) {
            card.question.questions.forEachIndexed { index, item ->
                QuestionItem(item, index, draft, onDraft = { draft = it })
            }
            val answers = draft.answers(card.question)
            CardButtons(
                labels = listOf(stringResource(R.string.chat_send_answers)),
                busy = busy,
                onPress = { answers?.let(onAnswer) },
                enabled = answers != null,
            )
        }
    }
}

@Composable
private fun QuestionItem(
    item: ChatQuestionItem,
    index: Int,
    draft: QuestionDraft,
    onDraft: (QuestionDraft) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier.fillMaxWidth()) {
        item.header?.takeIf { it.isNotBlank() }?.let {
            Text(it, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Text(item.question, style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurface)
        Column(Modifier.fillMaxWidth().then(if (item.multiSelect) Modifier else Modifier.selectableGroup())) {
            item.options.forEach { option ->
                val checked = option.label in draft.picked[index]
                ChoiceRow(item.multiSelect, checked, onToggle = { onDraft(draft.toggle(index, item, option.label)) }) {
                    Column(Modifier.padding(vertical = NgtermTheme.spacing.sm)) {
                        Text(
                            option.label,
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurface,
                        )
                        option.description?.takeIf { it.isNotBlank() }?.let {
                            Text(
                                it,
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
            TypedRow(item, index, draft, onDraft)
        }
    }
}

// the free-form row: its marker, then the field in place of a label; typing picks it, clearing it lets it go, and its
// marker takes a tap only while it holds text
@Composable
private fun TypedRow(
    item: ChatQuestionItem,
    index: Int,
    draft: QuestionDraft,
    onDraft: (QuestionDraft) -> Unit,
    modifier: Modifier = Modifier,
) {
    val label = stringResource(R.string.chat_type_answer)
    Row(
        modifier = modifier.fillMaxWidth().heightIn(min = NgtermTheme.spacing.touchTarget + NgtermTheme.spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Marker(
            multiple = item.multiSelect,
            checked = draft.otherPicked[index],
            enabled = draft.other[index].isNotBlank(),
            onClick = { onDraft(draft.toggleOther(index, item)) },
            modifier = Modifier.semantics { contentDescription = label },
        )
        OutlinedTextField(
            value = draft.other[index],
            onValueChange = { text -> onDraft(draft.withText(index, item, text.replace('\n', ' '))) },
            modifier = Modifier.weight(1f).padding(vertical = NgtermTheme.spacing.xs),
            placeholder = { Text(label) },
            singleLine = true,
            shape = MaterialTheme.shapes.medium,
        )
    }
}

// the draft as saved state: the question's id, then each question's picks, typed text and whether that is picked. A
// draft comes back only for the same question ([card]) and only when it fits it, else the card starts afresh
private fun draftSaver(card: OpenQuestion) =
    listSaver<MutableState<QuestionDraft>, Any>(
        save = { state ->
            val draft = state.value
            listOf(
                card.id,
                ArrayList(draft.picked.map { ArrayList(it) }),
                ArrayList(draft.other),
                ArrayList(draft.otherPicked),
            )
        },
        restore = { saved ->
            savedDraft(saved)?.takeIf { saved[0] == card.id && it.fits(card.question) }?.let(::mutableStateOf)
        },
    )

// the draft [draftSaver] wrote, or null when [saved] is not in its shape (a slot that isn't is left out, so the draft
// fits no question)
private fun savedDraft(saved: List<Any>): QuestionDraft? {
    val picked = saved.getOrNull(SAVED_PICKED).listOfOnly<List<*>>()?.mapNotNull { it.listOfOnly<String>() }
    val other = saved.getOrNull(SAVED_OTHER).listOfOnly<String>()
    val otherPicked = saved.getOrNull(SAVED_OTHER_PICKED).listOfOnly<Boolean>()
    return if (picked != null && other != null &&
        otherPicked != null
    ) {
        QuestionDraft(picked, other, otherPicked)
    } else {
        null
    }
}

// this as a list of [T] alone, or null
private inline fun <reified T> Any?.listOfOnly(): List<T>? =
    (this as? List<*>)?.takeIf { list -> list.all { it is T } }?.filterIsInstance<T>()

// where the saved draft's lists sit, after the question's id
private const val SAVED_PICKED = 1
private const val SAVED_OTHER = 2
private const val SAVED_OTHER_PICKED = 3
