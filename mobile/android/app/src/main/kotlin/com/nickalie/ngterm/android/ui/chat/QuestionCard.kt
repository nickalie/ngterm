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
import androidx.compose.runtime.mutableStateOf
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
 * and **Send answers** once every question has an answer. The draft lives with the card's question, and goes with it.
 */
@Composable
fun QuestionCardView(
    card: OpenQuestion,
    agent: String?,
    busy: Boolean,
    onAnswer: (List<QuestionAnswer>) -> Unit,
    modifier: Modifier = Modifier,
) {
    var draft by rememberSaveable(card.id, saver = draftSaver) { mutableStateOf(QuestionDraft.of(card.question)) }
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

// the draft as saved state: each question's picks, typed text and whether that is picked
private val draftSaver =
    listSaver<MutableState<QuestionDraft>, Any>(
        save = { state ->
            val draft = state.value
            listOf(
                ArrayList(draft.picked.map { ArrayList(it) }),
                ArrayList(draft.other),
                ArrayList(draft.otherPicked),
            )
        },
        restore = { saved ->
            @Suppress("UNCHECKED_CAST") // the lists save wrote, in its order
            mutableStateOf(
                QuestionDraft(
                    picked = (saved[0] as List<List<String>>),
                    other = saved[1] as List<String>,
                    otherPicked = saved[2] as List<Boolean>,
                ),
            )
        },
    )
