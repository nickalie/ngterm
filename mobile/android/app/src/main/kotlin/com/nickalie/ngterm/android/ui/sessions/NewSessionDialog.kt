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
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * New session on [machine]: an optional name (blank lets the machine name it) and Create session. While the create
 * runs (it waits until the machine lists the session, up to 20 s) the button says Creating with a spinner and the
 * field holds still; Cancel closes the dialog and the create goes on. What went wrong shows under the field, which
 * keeps its text for another try.
 */
@Composable
fun NewSessionDialog(
    machine: String,
    state: NewSessionState,
    onCreate: (String) -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var name by rememberSaveable { mutableStateOf("") }
    val create = { if (!state.running) onCreate(name) }
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = modifier,
        title = { Text(stringResource(R.string.new_session_title, machine)) },
        text = {
            OutlinedTextField(
                value = name,
                onValueChange = { name = it },
                label = { Text(stringResource(R.string.new_session_name)) },
                supportingText = {
                    Text(state.failed ?: stringResource(R.string.new_session_name_hint, machine))
                },
                isError = state.failed != null,
                enabled = !state.running,
                singleLine = true,
                keyboardOptions =
                    KeyboardOptions(capitalization = KeyboardCapitalization.None, imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { create() }),
            )
        },
        confirmButton = {
            TextButton(onClick = create, enabled = !state.running) {
                if (state.running) {
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        CircularProgressIndicator(modifier = Modifier.size(16.dp), strokeWidth = 2.dp)
                        Text(stringResource(R.string.new_session_creating))
                    }
                } else {
                    Text(stringResource(R.string.new_session_create))
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
