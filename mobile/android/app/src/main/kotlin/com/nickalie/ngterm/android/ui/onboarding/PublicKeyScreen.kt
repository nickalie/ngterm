package com.nickalie.ngterm.android.ui.onboarding

import android.content.Intent
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.material3.Button
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.AppKey
import com.nickalie.ngterm.android.ui.components.CopyButton
import com.nickalie.ngterm.android.ui.components.KeyPlate
import com.nickalie.ngterm.android.ui.components.ScreenExit
import com.nickalie.ngterm.android.ui.components.ScreenFrame
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * A key's public half and how to put it on a machine: the three steps, the `authorized_keys` line with Copy and Share,
 * its fingerprint, and the one command that appends it. The first run ends here with [onDone]; from the machine form
 * it is a screen with an [exit] back.
 */
@Composable
fun PublicKeyScreen(
    key: AppKey,
    modifier: Modifier = Modifier,
    exit: ScreenExit? = null,
    onDone: (() -> Unit)? = null,
) {
    val spacing = NgtermTheme.spacing
    val share = shareText(stringResource(R.string.key_share_title), key.authorizedKey)
    val title = stringResource(R.string.key_title)
    ScreenFrame(
        modifier = modifier,
        title = if (exit != null) title else null,
        exit = exit,
        maxWidth = if (exit != null) spacing.readableWidth else OnboardingWidth,
    ) {
        if (exit == null) {
            Text(
                text = title,
                style = MaterialTheme.typography.headlineMedium,
                color = MaterialTheme.colorScheme.onSurface,
                modifier = Modifier.padding(top = spacing.xxl).semantics { heading() },
            )
        }
        Steps(
            listOf(
                stringResource(R.string.key_step_copy),
                stringResource(R.string.key_step_append),
                stringResource(R.string.key_step_add),
            ),
        )
        KeyPlate(text = key.authorizedKey, label = stringResource(R.string.key_line_label)) {
            CopyButton(key.authorizedKey)
            TextButton(onClick = share) { Text(stringResource(R.string.action_share)) }
        }
        Text(
            text = stringResource(R.string.key_fingerprint, key.fingerprint),
            style = NgtermTheme.mono.small,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        val command = appendCommand(key.authorizedKey)
        KeyPlate(text = command, label = stringResource(R.string.key_command_label), style = NgtermTheme.mono.small) {
            CopyButton(command)
        }
        if (onDone != null) {
            Button(onClick = onDone, modifier = Modifier.fillMaxWidth().padding(top = spacing.sm)) {
                Text(stringResource(R.string.key_done))
            }
        }
    }
}

/**
 * The shell command that adds [line] to the signing-in user's `authorized_keys`, making `~/.ssh` as sshd wants it.
 */
fun appendCommand(line: String): String =
    "mkdir -p ~/.ssh && chmod 700 ~/.ssh && echo '${line.replace("'", "'\\''")}' >> ~/.ssh/authorized_keys"

// a real sequence, so numbered: what to do, in order
@Composable
private fun Steps(
    steps: List<String>,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
        steps.forEachIndexed { index, step ->
            Row(horizontalArrangement = Arrangement.spacedBy(spacing.md)) {
                Text(
                    text = "${index + 1}",
                    style = MaterialTheme.typography.titleMedium,
                    color = MaterialTheme.colorScheme.primary,
                    modifier = Modifier.widthIn(min = 16.dp),
                )
                Text(
                    text = step,
                    style = MaterialTheme.typography.bodyLarge,
                    color = MaterialTheme.colorScheme.onSurface,
                )
            }
        }
    }
}

@Composable
private fun shareText(
    title: String,
    text: String,
): () -> Unit {
    val context = LocalContext.current
    return {
        val send =
            Intent(Intent.ACTION_SEND)
                .setType("text/plain")
                .putExtra(Intent.EXTRA_SUBJECT, title)
                .putExtra(Intent.EXTRA_TEXT, text)
        context.startActivity(Intent.createChooser(send, title))
    }
}
