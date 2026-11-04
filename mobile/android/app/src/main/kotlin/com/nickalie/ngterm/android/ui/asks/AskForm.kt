package com.nickalie.ngterm.android.ui.asks

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Checkbox
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.RemoteAskChoices
import com.nickalie.ngterm.core.ffi.RemoteAskInput

/** The most lines a multiline answer's field grows to before it scrolls. */
private const val MULTILINE_MAX_LINES = 8

/** The lines a multiline answer's field starts with. */
private const val MULTILINE_MIN_LINES = 3

// an input ask's field: one line or an editing area, starting from the origin's text; an edit that would add a
// control character the field can't hold is refused whole, as desktop refuses it
@Composable
internal fun AnswerField(
    input: RemoteAskInput,
    draft: AskDraft,
    onDraft: (AskDraft) -> Unit,
    onDone: (() -> Unit)?,
) {
    val hint = input.placeholder ?: stringResource(R.string.ask_answer_hint)
    OutlinedTextField(
        value = draft.text,
        onValueChange = { if (!hasForbiddenControls(it, input.multiline)) onDraft(draft.withText(it)) },
        modifier = Modifier.fillMaxWidth(),
        placeholder = { Text(hint) },
        singleLine = !input.multiline,
        minLines = if (input.multiline) MULTILINE_MIN_LINES else 1,
        maxLines = if (input.multiline) MULTILINE_MAX_LINES else 1,
        keyboardOptions = KeyboardOptions(imeAction = if (input.multiline) ImeAction.Default else ImeAction.Done),
        keyboardActions = KeyboardActions(onDone = { onDone?.invoke() }),
        shape = MaterialTheme.shapes.medium,
    )
}

// a choice ask's list: a row per choice (its label, its subtitle under it), radio buttons or checkboxes, then the
// free-form row when the ask has one
@Composable
internal fun ChoiceList(
    list: RemoteAskChoices,
    draft: AskDraft,
    onDraft: (AskDraft) -> Unit,
) {
    Column(Modifier.fillMaxWidth().then(if (list.multiple) Modifier else Modifier.selectableGroup())) {
        list.choices.forEachIndexed { index, choice ->
            val checked = choice.id in draft.checked
            ChoiceRow(multiple = list.multiple, checked = checked, onToggle = { onDraft(draft.toggle(index)) }) {
                Column(Modifier.padding(vertical = NgtermTheme.spacing.sm)) {
                    Text(
                        choice.label,
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurface,
                    )
                    choice.subtitle?.let {
                        Text(
                            it,
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
        list.other?.let { placeholder ->
            OtherRow(list.multiple, placeholder, draft, onDraft, index = list.choices.size)
        }
    }
}

// the free-form row: its marker, then the field in place of a label; typing selects it, clearing it gives a radio
// list's pick back, and its marker takes a tap only while it holds text
@Composable
private fun OtherRow(
    multiple: Boolean,
    placeholder: String,
    draft: AskDraft,
    onDraft: (AskDraft) -> Unit,
    index: Int,
) {
    val label = placeholder.ifBlank { stringResource(R.string.ask_other_choice) }
    Row(
        modifier = Modifier.fillMaxWidth().heightIn(min = NgtermTheme.spacing.touchTarget + NgtermTheme.spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Marker(
            multiple = multiple,
            checked = draft.otherSelected,
            enabled = draft.otherEnabled,
            onClick = { onDraft(draft.toggle(index)) },
            modifier = Modifier.semantics { contentDescription = label },
        )
        OutlinedTextField(
            value = draft.text,
            onValueChange = { if (!hasForbiddenControls(it, multiline = false)) onDraft(draft.withText(it)) },
            modifier = Modifier.weight(1f).padding(vertical = NgtermTheme.spacing.xs),
            placeholder = { Text(label) },
            singleLine = true,
            shape = MaterialTheme.shapes.medium,
        )
    }
}

// a choice's row: its marker in a touch target's room, then its label; the whole row takes the tap (the chat's
// question card shares it)
@Composable
internal fun ChoiceRow(
    multiple: Boolean,
    checked: Boolean,
    onToggle: () -> Unit,
    label: @Composable () -> Unit,
) {
    val pick =
        if (multiple) {
            Modifier.toggleable(value = checked, role = Role.Checkbox, onValueChange = { onToggle() })
        } else {
            Modifier.selectable(selected = checked, role = Role.RadioButton, onClick = onToggle)
        }
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .heightIn(min = NgtermTheme.spacing.touchTarget + NgtermTheme.spacing.sm)
                .then(pick),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        // the marker in a touch target's room, so it lines up with the free-form row's, which takes taps itself
        Box(Modifier.size(NgtermTheme.spacing.touchTarget), contentAlignment = Alignment.Center) {
            Marker(multiple = multiple, checked = checked, enabled = true, onClick = null)
        }
        label()
    }
}

// a radio button or a checkbox; with no onClick its row takes the tap
@Composable
internal fun Marker(
    multiple: Boolean,
    checked: Boolean,
    enabled: Boolean,
    onClick: (() -> Unit)?,
    modifier: Modifier = Modifier,
) {
    if (multiple) {
        Checkbox(
            checked = checked,
            onCheckedChange =
                onClick?.let {
                    { _: Boolean ->
                        it()
                    }
                },
            modifier = modifier,
            enabled = enabled,
        )
    } else {
        RadioButton(selected = checked, onClick = onClick, modifier = modifier, enabled = enabled)
    }
}
