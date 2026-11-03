package com.nickalie.ngterm.android.ui.components

import android.content.ClipData
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.LocalClipboard
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * Text that is compared or pasted character by character (a public key line, a fingerprint, a command), on a plate
 * of its own: mono, selectable, wrapping anywhere, with an optional [label] above it and [actions] under it (a
 * [CopyButton], Share). Flat like a card, one tone up from the ground.
 */
@Composable
fun KeyPlate(
    text: String,
    modifier: Modifier = Modifier,
    label: String? = null,
    style: TextStyle = NgtermTheme.mono.medium,
    actions: @Composable () -> Unit = {},
) {
    val spacing = NgtermTheme.spacing
    Column(modifier = modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(spacing.xs)) {
        if (label != null) {
            Text(
                text = label,
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Surface(
            shape = MaterialTheme.shapes.medium,
            color = MaterialTheme.colorScheme.surfaceContainerLow,
            modifier = Modifier.fillMaxWidth(),
        ) {
            Column {
                SelectionContainer {
                    Text(
                        text = text,
                        style = style,
                        color = MaterialTheme.colorScheme.onSurface,
                        modifier = Modifier.padding(horizontal = spacing.lg, vertical = spacing.md),
                    )
                }
                Row(modifier = Modifier.padding(start = spacing.xs, bottom = spacing.xxs)) { actions() }
            }
        }
    }
}

/** Copies [text] to the clipboard, and says "Copied" for a moment after. */
@Composable
fun CopyButton(
    text: String,
    modifier: Modifier = Modifier,
    label: String = stringResource(R.string.action_copy),
) {
    val clipboard = LocalClipboard.current
    val scope = rememberCoroutineScope()
    var copied by remember { mutableStateOf(false) }
    LaunchedEffect(copied) {
        if (copied) {
            delay(COPIED_FOR_MS)
            copied = false
        }
    }
    TextButton(
        onClick = {
            scope.launch { clipboard.setClipEntry(ClipEntry(ClipData.newPlainText(label, text))) }
            copied = true
        },
        modifier = modifier,
    ) {
        Text(if (copied) stringResource(R.string.action_copied) else label)
    }
}

private const val COPIED_FOR_MS = 2000L
