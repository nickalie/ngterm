package com.nickalie.ngterm.android.ui.machines

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.listSaver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.KeyboardType
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.AppKey
import com.nickalie.ngterm.android.ui.components.KeyPlate
import com.nickalie.ngterm.android.ui.components.ScreenExit
import com.nickalie.ngterm.android.ui.components.ScreenFrame
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

private val DraftSaver =
    listSaver<MachineDraft, String>(
        save = { listOf(it.label, it.host, it.port, it.user, it.keyId) },
        restore = { MachineDraft(it[0], it[1], it[2], it[3], it[4]) },
    )

/**
 * Add machine (no [editing]) or Edit machine: the name, host, port and user, the key to sign in with (a choice only
 * when the app has more than one), the way to its public key, and for a machine already added its pinned host key. A
 * field says what is wrong with it once it has been left or Save was tried; Save with problems only shows them.
 */
@Composable
fun MachineFormScreen(
    initial: MachineDraft,
    editing: Boolean,
    keys: List<AppKey>,
    pinned: String?,
    onSave: (MachineDraft) -> Unit,
    onClose: () -> Unit,
    onShowKey: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    var draft by rememberSaveable(stateSaver = DraftSaver) {
        mutableStateOf(if (initial.keyId.isEmpty()) initial.copy(keyId = keys.firstOrNull()?.id.orEmpty()) else initial)
    }
    var tried by rememberSaveable { mutableStateOf(false) }
    // the fields focused once, and those left after that: a field shows its problem from when it is left
    var visited by rememberSaveable { mutableStateOf(listOf<String>()) }
    var left by rememberSaveable { mutableStateOf(listOf<String>()) }
    val problems = draft.problems()

    fun problem(field: MachineField) = problems[field]?.takeIf { tried || field.name in left }

    fun focus(
        field: MachineField,
        focused: Boolean,
    ) {
        when {
            focused && field.name !in visited -> visited = visited + field.name
            !focused && field.name in visited && field.name !in left -> left = left + field.name
        }
    }
    val save = {
        tried = true
        if (problems.isEmpty()) onSave(draft)
    }
    ScreenFrame(
        modifier = modifier,
        title = stringResource(if (editing) R.string.form_edit_title else R.string.form_add_title),
        exit = ScreenExit(R.drawable.ic_close, stringResource(R.string.form_close), onClose),
        action = { TextButton(onClick = save) { Text(stringResource(R.string.form_save)) } },
    ) {
        AddressFields(
            draft = draft,
            onChange = { draft = it },
            problem = ::problem,
            onFocus = ::focus,
        )
        KeyChoice(
            keys = keys,
            selected = draft.keyId,
            problem = problem(MachineField.Key),
            onSelect = { draft = draft.copy(keyId = it) },
            onShowKey = onShowKey,
        )
        if (editing) {
            Pinned(pinned)
        }
    }
}

// the machine's name and address: what the form's fields hold before the key
@Composable
private fun AddressFields(
    draft: MachineDraft,
    onChange: (MachineDraft) -> Unit,
    problem: (MachineField) -> FieldProblem?,
    onFocus: (MachineField, Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.lg)) {
        FormField(
            value = draft.label,
            onValueChange = { onChange(draft.copy(label = it)) },
            label = stringResource(R.string.form_label),
            hint = stringResource(R.string.form_label_hint),
            problem = null,
            capitalization = KeyboardCapitalization.Sentences,
        )
        FormField(
            value = draft.host,
            onValueChange = { onChange(draft.copy(host = it)) },
            label = stringResource(R.string.form_host),
            hint = stringResource(R.string.form_host_hint),
            problem = problem(MachineField.Host),
            keyboardType = KeyboardType.Uri,
            mono = true,
            onFocus = { onFocus(MachineField.Host, it) },
        )
        FormField(
            value = draft.port,
            onValueChange = { onChange(draft.copy(port = it.filter(Char::isDigit))) },
            label = stringResource(R.string.form_port),
            hint = null,
            problem = problem(MachineField.Port),
            keyboardType = KeyboardType.Number,
            mono = true,
            onFocus = { onFocus(MachineField.Port, it) },
        )
        FormField(
            value = draft.user,
            onValueChange = { onChange(draft.copy(user = it)) },
            label = stringResource(R.string.form_user),
            hint = stringResource(R.string.form_user_hint),
            problem = problem(MachineField.User),
            mono = true,
            imeAction = ImeAction.Done,
            onFocus = { onFocus(MachineField.User, it) },
        )
    }
}

@Composable
private fun FormField(
    value: String,
    onValueChange: (String) -> Unit,
    label: String,
    hint: String?,
    problem: FieldProblem?,
    modifier: Modifier = Modifier,
    keyboardType: KeyboardType = KeyboardType.Text,
    capitalization: KeyboardCapitalization = KeyboardCapitalization.None,
    imeAction: ImeAction = ImeAction.Next,
    mono: Boolean = false,
    onFocus: (Boolean) -> Unit = {},
) {
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        label = { Text(label) },
        supportingText =
            when {
                problem != null -> {
                    { Text(stringResource(problem.message)) }
                }

                hint != null -> {
                    { Text(hint) }
                }

                else -> {
                    null
                }
            },
        isError = problem != null,
        singleLine = true,
        textStyle = if (mono) NgtermTheme.mono.medium else MaterialTheme.typography.bodyLarge,
        keyboardOptions =
            KeyboardOptions(
                capitalization = capitalization,
                autoCorrectEnabled = false,
                keyboardType = keyboardType,
                imeAction = imeAction,
            ),
        modifier = modifier.fillMaxWidth().onFocusChanged { onFocus(it.isFocused) },
    )
}

@Composable
private fun KeyChoice(
    keys: List<AppKey>,
    selected: String,
    problem: FieldProblem?,
    onSelect: (String) -> Unit,
    onShowKey: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(spacing.xs)) {
        Text(
            text = stringResource(R.string.form_key),
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.semantics { heading() },
        )
        Column(Modifier.selectableGroup()) {
            keys.forEach { key ->
                KeyRow(key, chosen = key.id == selected, choice = keys.size > 1, onSelect = { onSelect(key.id) })
            }
        }
        if (problem != null) {
            Text(
                text = stringResource(problem.message),
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.error,
            )
        }
        if (selected.isNotEmpty()) {
            TextButton(onClick = { onShowKey(selected) }) { Text(stringResource(R.string.form_show_key)) }
        }
    }
}

@Composable
private fun KeyRow(
    key: AppKey,
    chosen: Boolean,
    choice: Boolean,
    onSelect: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    val rowModifier =
        if (choice) {
            modifier.selectable(selected = chosen, role = Role.RadioButton, onClick = onSelect)
        } else {
            modifier
        }
    Row(
        modifier = rowModifier.fillMaxWidth().heightIn(min = spacing.touchTarget + spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        if (choice) RadioButton(selected = chosen, onClick = null, modifier = Modifier.padding(end = spacing.md))
        Column(verticalArrangement = Arrangement.spacedBy(spacing.xxs)) {
            Text(
                text = stringResource(if (key.imported) R.string.form_key_imported else R.string.form_key_created),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurface,
            )
            Text(
                text = key.fingerprint,
                style = NgtermTheme.mono.small,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}

@Composable
private fun Pinned(
    pinned: String?,
    modifier: Modifier = Modifier,
) {
    if (pinned != null) {
        KeyPlate(text = pinned, label = stringResource(R.string.form_pinned), modifier = modifier)
    } else {
        Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.xs)) {
            Text(
                text = stringResource(R.string.form_pinned),
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            Text(
                text = stringResource(R.string.form_pinned_none),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
    }
}
