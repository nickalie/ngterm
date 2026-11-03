package com.nickalie.ngterm.android.ui.onboarding

import android.app.Activity
import android.content.Context
import android.content.ContextWrapper
import android.net.Uri
import android.view.WindowManager
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.ScreenExit
import com.nickalie.ngterm.android.ui.components.ScreenFrame
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import java.io.ByteArrayOutputStream
import java.io.IOException
import java.io.InputStream

// a private key file is a few kilobytes; anything far bigger is not one
private const val MAX_KEY_BYTES = 64 * 1024

/**
 * Importing a key the user already has: pasted, or read from a file they choose, and its passphrase. [problem] says
 * what was wrong with the last attempt, under the field it is about, and the text (or the chosen file's key) stays for
 * another try. The key's text, the chosen file's bytes and the passphrase live only in this screen's memory, never in
 * saved state, and go when the screen does. While the screen is up the window is secure, so the key stays out of
 * screenshots, recordings and the Recents thumbnail.
 */
@Composable
fun ImportKeyScreen(
    busy: Boolean,
    problem: ImportProblem?,
    onImport: (ByteArray, String?) -> Unit,
    onUnreadable: () -> Unit,
    onBack: () -> Unit,
    modifier: Modifier = Modifier,
    initialKey: String = "",
) {
    SecureWindow()
    var keyText by remember { mutableStateOf(initialKey) }
    var passphrase by remember { mutableStateOf("") }
    val chosen = remember { ChosenKey() }
    DisposableEffect(chosen) { onDispose { chosen.set(null) } }
    val chooseFile =
        rememberKeyFileChooser(onRead = { onImport(chosen.keep(it), passphrase) }, onUnreadable)
    val keyProblem = problem?.takeUnless { it.isAboutPassphrase() }
    ScreenFrame(
        modifier = modifier,
        title = stringResource(R.string.import_title),
        exit = ScreenExit(R.drawable.ic_back, stringResource(R.string.action_back), onBack),
        maxWidth = OnboardingWidth,
    ) {
        Text(
            text = stringResource(R.string.import_body),
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        OutlinedTextField(
            value = keyText,
            onValueChange = { keyText = it.also { chosen.set(null) } },
            label = { Text(stringResource(R.string.import_key_label)) },
            textStyle = NgtermTheme.mono.small,
            minLines = 6,
            maxLines = 12,
            isError = keyProblem != null,
            supportingText = keyFieldNote(keyProblem, fileChosen = chosen.bytes != null),
            keyboardOptions = KeyboardOptions(autoCorrectEnabled = false, keyboardType = KeyboardType.Ascii),
            enabled = !busy,
            modifier = Modifier.fillMaxWidth(),
        )
        TextButton(onClick = chooseFile, enabled = !busy) {
            Text(stringResource(R.string.import_choose_file))
        }
        PassphraseField(
            value = passphrase,
            onValueChange = { passphrase = it },
            problem = problem?.takeIf { it.isAboutPassphrase() },
            enabled = !busy,
        )
        BusyButton(
            label = stringResource(R.string.import_action),
            busy = busy,
            onClick = { onImport(chosen.bytes?.copyOf() ?: keyText.toByteArray(), passphrase) },
            modifier = Modifier.fillMaxWidth().padding(top = NgtermTheme.spacing.sm),
        )
    }
}

/**
 * The key read from the file the user chose, kept for a retry (with its passphrase, say) until they type a key or
 * leave. Import wipes what it is handed, so it gets a copy; a key replaced or let go is wiped here.
 */
private class ChosenKey {
    var bytes by mutableStateOf<ByteArray?>(null)
        private set

    fun set(key: ByteArray?) {
        bytes?.fill(0)
        bytes = key
    }

    /** Keeps [key] and returns a copy of it, for import to use and wipe. */
    fun keep(key: ByteArray): ByteArray = key.copyOf().also { set(key) }
}

// what shows under the key field: the last attempt's problem with the key, or that the chosen file's key is used
private fun keyFieldNote(
    problem: ImportProblem?,
    fileChosen: Boolean,
): (@Composable () -> Unit)? =
    when {
        problem != null -> {
            { Text(problem.words()) }
        }

        fileChosen -> {
            { Text(stringResource(R.string.import_file_chosen)) }
        }

        else -> {
            null
        }
    }

// keeps the window out of screenshots, screen recordings and the Recents thumbnail while it is composed
@Composable
private fun SecureWindow() {
    val activity = LocalContext.current.findActivity()
    DisposableEffect(activity) {
        val window = activity?.window
        window?.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        onDispose { window?.clearFlags(WindowManager.LayoutParams.FLAG_SECURE) }
    }
}

private tailrec fun Context.findActivity(): Activity? =
    when (this) {
        is Activity -> this
        is ContextWrapper -> baseContext.findActivity()
        else -> null
    }

// opens the system's file chooser and hands over the chosen file's bytes, or says it couldn't be read
@Composable
private fun rememberKeyFileChooser(
    onRead: (ByteArray) -> Unit,
    onUnreadable: () -> Unit,
): () -> Unit {
    val context = LocalContext.current
    val launcher =
        rememberLauncherForActivityResult(ActivityResultContracts.OpenDocument()) { uri: Uri? ->
            if (uri != null) {
                val bytes =
                    try {
                        context.contentResolver.openInputStream(uri)?.use { it.readAtMost(MAX_KEY_BYTES + 1) }
                    } catch (_: IOException) {
                        null
                    } catch (_: SecurityException) {
                        null
                    }
                if (bytes == null || bytes.size > MAX_KEY_BYTES) onUnreadable() else onRead(bytes)
            }
        }
    return { launcher.launch(arrayOf("*/*")) }
}

@Composable
private fun PassphraseField(
    value: String,
    onValueChange: (String) -> Unit,
    problem: ImportProblem?,
    enabled: Boolean,
    modifier: Modifier = Modifier,
) {
    var shown by remember { mutableStateOf(false) }
    OutlinedTextField(
        value = value,
        onValueChange = onValueChange,
        label = { Text(stringResource(R.string.import_passphrase_label)) },
        placeholder = { Text(stringResource(R.string.import_passphrase_hint)) },
        singleLine = true,
        isError = problem != null,
        supportingText = problem?.let { { Text(it.words()) } },
        visualTransformation = if (shown) VisualTransformation.None else PasswordVisualTransformation(),
        keyboardOptions =
            KeyboardOptions(
                autoCorrectEnabled = false,
                keyboardType = KeyboardType.Password,
                imeAction = ImeAction.Done,
            ),
        trailingIcon = {
            TextButton(onClick = { shown = !shown }) {
                Text(stringResource(if (shown) R.string.import_hide_passphrase else R.string.import_show_passphrase))
            }
        },
        enabled = enabled,
        modifier = modifier.fillMaxWidth(),
    )
}

private fun ImportProblem.isAboutPassphrase() =
    this == ImportProblem.NeedsPassphrase || this == ImportProblem.WrongPassphrase

@Composable
private fun ImportProblem.words(): String =
    when (this) {
        ImportProblem.NeedsPassphrase -> stringResource(R.string.import_needs_passphrase)
        ImportProblem.WrongPassphrase -> stringResource(R.string.import_wrong_passphrase)
        is ImportProblem.NotAKey -> stringResource(R.string.import_not_a_key, reason)
        is ImportProblem.NotStored -> stringResource(R.string.import_not_stored, reason)
        ImportProblem.Empty -> stringResource(R.string.import_empty)
        ImportProblem.Unreadable -> stringResource(R.string.import_unreadable)
    }

// up to [limit] bytes of the stream (InputStream.readNBytes is API 33)
private fun InputStream.readAtMost(limit: Int): ByteArray {
    val out = ByteArrayOutputStream()
    val buffer = ByteArray(DEFAULT_BUFFER_SIZE)
    while (out.size() < limit) {
        val read = read(buffer, 0, minOf(buffer.size, limit - out.size()))
        if (read < 0) break
        out.write(buffer, 0, read)
    }
    return out.toByteArray()
}
