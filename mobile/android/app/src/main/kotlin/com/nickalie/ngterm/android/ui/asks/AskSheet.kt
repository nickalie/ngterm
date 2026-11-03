package com.nickalie.ngterm.android.ui.asks

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.MutableState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.ui.components.StatusGlyph
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusShape
import com.nickalie.ngterm.core.ffi.AskReply
import com.nickalie.ngterm.core.ffi.RemoteAsk
import com.nickalie.ngterm.core.ffi.RemoteAskButton

/**
 * The first open ask of the session on screen, as a sheet over it (the ask rule: an ask out of sight is a
 * notification instead). Its answer, or a dismissal (Back or a swipe, which the origin gets as Escape), goes to the
 * core once.
 */
@Composable
fun AskPrompt(graph: AppGraph) {
    val open by graph.asks.open.collectAsStateWithLifecycle()
    val shown by graph.asks.onScreen.collectAsStateWithLifecycle()
    val ask = open.firstOrNull { it.key == shown } ?: return
    key(ask.key, ask.ask.id) {
        AskSheet(ask = ask.ask, onAnswer = { reply -> graph.asks.answer(ask.key, ask.ask.id, reply) })
    }
}

/** [AskQuestion] in a sheet, its draft kept through a configuration change; dismissing the sheet escapes. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AskSheet(
    ask: RemoteAsk,
    onAnswer: (AskReply) -> Unit,
    modifier: Modifier = Modifier,
) {
    var draft by rememberSaveable(ask.id, saver = draftSaver(ask)) { mutableStateOf(AskDraft.of(ask)) }
    ModalBottomSheet(
        onDismissRequest = { onAnswer(EscapedReply) },
        modifier = modifier,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.large,
    ) {
        AskQuestion(ask = ask, draft = draft, onDraft = { draft = it }, onAnswer = onAnswer)
    }
}

/**
 * An ask's question: the attention diamond, its title and message, its form (a text field, or a choice list with its
 * free-form row), then its buttons in the origin's order, one per line: the default filled, the destructive in the
 * error color, the cancel button quiet, the rest outlined. A button answers with [draft] ([askReply]); a line field's
 * Enter presses the default button (or the one answering button).
 */
@Composable
fun AskQuestion(
    ask: RemoteAsk,
    draft: AskDraft,
    onDraft: (AskDraft) -> Unit,
    onAnswer: (AskReply) -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    val press = { button: String -> onAnswer(askReply(ask, button, draft)) }
    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .verticalScroll(rememberScrollState())
                .navigationBarsPadding()
                .padding(horizontal = spacing.xl)
                .padding(bottom = spacing.lg),
        verticalArrangement = Arrangement.spacedBy(spacing.lg),
    ) {
        StatusGlyph(StatusShape.Diamond, NgtermTheme.status.attention, size = 20.dp)
        Text(
            text = ask.title,
            style = MaterialTheme.typography.headlineSmall,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.semantics { heading() },
        )
        ask.message?.takeIf { it.isNotBlank() }?.let {
            Text(
                text = it,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        ask.input?.let { input ->
            AnswerField(input, draft, onDraft, onDone = enterButton(ask)?.let { button -> { press(button) } })
        }
        ask.choices?.let { list -> ChoiceList(list, draft, onDraft) }
        Column(verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
            ask.buttons.forEach { button -> AskButton(ask, button, onClick = { press(button.id) }) }
        }
    }
}

// the button a line field's Enter presses: the default, or the only one that answers
private fun enterButton(ask: RemoteAsk): String? =
    ask.defaultId ?: ask.buttons
        .filter { it.id != ask.cancelId }
        .singleOrNull()
        ?.id

@Composable
private fun AskButton(
    ask: RemoteAsk,
    button: RemoteAskButton,
    onClick: () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val wide = Modifier.fillMaxWidth()
    val destructive = button.id == ask.destructiveId
    when {
        button.id == ask.defaultId -> {
            val filled =
                if (destructive) {
                    ButtonDefaults.buttonColors(containerColor = colors.error, contentColor = colors.onError)
                } else {
                    ButtonDefaults.buttonColors()
                }
            Button(onClick = onClick, modifier = wide, colors = filled) { Text(button.label) }
        }

        button.id == ask.cancelId -> {
            TextButton(onClick = onClick, modifier = wide) { Text(button.label) }
        }

        else -> {
            val outlined =
                if (destructive) {
                    ButtonDefaults.outlinedButtonColors(contentColor = colors.error)
                } else {
                    ButtonDefaults.outlinedButtonColors()
                }
            OutlinedButton(onClick = onClick, modifier = wide, colors = outlined) { Text(button.label) }
        }
    }
}

// the draft as saved state: the text, the checked ids, whether the free-form row is picked and the pick it took over
private fun draftSaver(ask: RemoteAsk) =
    listSaver<MutableState<AskDraft>, Any?>(
        save = { listOf(it.value.text, ArrayList(it.value.checked), it.value.otherSelected, it.value.previousPick) },
        restore = { saved ->
            @Suppress("UNCHECKED_CAST") // the list save wrote, in its order
            val checked = (saved[1] as List<String>).toSet()
            mutableStateOf(
                AskDraft.of(ask).copy(
                    text = saved[0] as String,
                    checked = checked,
                    otherSelected = saved[2] as Boolean,
                    previousPick = saved[3] as String?,
                ),
            )
        },
    )
