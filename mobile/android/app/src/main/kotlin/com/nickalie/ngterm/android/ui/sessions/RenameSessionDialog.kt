package com.nickalie.ngterm.android.ui.sessions

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * Rename session: one Name field that starts with the session's name, selected, then Rename and Cancel. Rename takes
 * no blank name. While the machine renames it the button says Renaming with a spinner and the field holds still; what
 * went wrong shows under the field, which keeps its text for another try.
 */
@Composable
fun RenameSessionDialog(
    state: RenameState,
    onRename: (String) -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var field by rememberSaveable(state.sessionId, stateSaver = TextFieldValue.Saver) {
        mutableStateOf(TextFieldValue(state.name, TextRange(0, state.name.length)))
    }
    val ready = !state.running && field.text.isNotBlank()
    val rename = { if (ready) onRename(field.text) }
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = modifier,
        title = { Text(stringResource(R.string.rename_session_title, state.name)) },
        text = {
            OutlinedTextField(
                value = field,
                onValueChange = { field = it },
                label = { Text(stringResource(R.string.rename_session_name)) },
                supportingText = state.failed?.let { failed -> { Text(failed) } },
                isError = state.failed != null,
                enabled = !state.running,
                singleLine = true,
                keyboardOptions =
                    KeyboardOptions(capitalization = KeyboardCapitalization.None, imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { rename() }),
            )
        },
        confirmButton = {
            TextButton(onClick = rename, enabled = ready) {
                if (state.running) {
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                        Text(stringResource(R.string.rename_session_renaming))
                    }
                } else {
                    Text(stringResource(R.string.rename_session_save))
                }
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) }
        },
        shape = MaterialTheme.shapes.large,
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
    )
}
