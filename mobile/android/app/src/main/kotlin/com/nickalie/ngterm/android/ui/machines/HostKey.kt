package com.nickalie.ngterm.android.ui.machines

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
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
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.ConfirmDialog
import com.nickalie.ngterm.android.ui.components.CopyButton
import com.nickalie.ngterm.android.ui.components.KeyPlate
import com.nickalie.ngterm.android.ui.components.ScreenExit
import com.nickalie.ngterm.android.ui.components.ScreenFrame
import com.nickalie.ngterm.android.ui.components.StatusGlyph
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusShape

/** What prints a machine's host key fingerprints, whatever kinds of key it has. */
const val FINGERPRINT_COMMAND = "ssh-keyscan localhost 2>/dev/null | ssh-keygen -lf -"

/**
 * The first connection to [machine], reached at [address] (`user@host:port`, which tells apart machines that share a
 * name): it offered [fingerprint] and waits for the user. Trust pins it and connects; Don't connect, swiping the sheet
 * away or back refuses it.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HostKeySheet(
    machine: String,
    address: String,
    fingerprint: String,
    onAnswer: (trust: Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val sheet = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = { onAnswer(false) },
        modifier = modifier,
        sheetState = sheet,
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.large,
    ) {
        HostKeyQuestion(machine, address, fingerprint, onAnswer)
    }
}

/** The host-key question's content, apart from its sheet. */
@Composable
internal fun HostKeyQuestion(
    machine: String,
    address: String,
    fingerprint: String,
    onAnswer: (trust: Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
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
            text = stringResource(R.string.hostkey_title, machine),
            style = MaterialTheme.typography.headlineSmall,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.semantics { heading() },
        )
        Text(
            text = address,
            style = NgtermTheme.mono.small,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            text = stringResource(R.string.hostkey_body, machine),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        KeyPlate(text = fingerprint, label = stringResource(R.string.hostkey_fingerprint))
        KeyPlate(
            text = FINGERPRINT_COMMAND,
            label = stringResource(R.string.hostkey_check),
            style = NgtermTheme.mono.small,
        ) { CopyButton(FINGERPRINT_COMMAND) }
        Column(verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
            Button(onClick = { onAnswer(true) }, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.hostkey_trust))
            }
            TextButton(onClick = { onAnswer(false) }, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.hostkey_decline))
            }
        }
    }
}

/**
 * The hard stop when [machine] offered [offered] instead of its [pinned] key: what that can mean, both
 * fingerprints, and two ways on. Back to machines is the safe one and the main action; Remove pinned key is the
 * deliberate one, behind a confirmation.
 */
@Composable
fun HostKeyChangedScreen(
    machine: String,
    pinned: String,
    offered: String,
    onBack: () -> Unit,
    onRemovePin: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    var confirming by rememberSaveable { mutableStateOf(false) }
    ScreenFrame(
        modifier = modifier,
        exit = ScreenExit(R.drawable.ic_back, stringResource(R.string.action_back), onBack),
    ) {
        StatusGlyph(StatusShape.Triangle, NgtermTheme.status.error, size = 28.dp)
        Text(
            text = stringResource(R.string.hostkey_changed_title, machine),
            style = MaterialTheme.typography.headlineMedium,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.semantics { heading() },
        )
        Text(
            text = stringResource(R.string.hostkey_changed_body, machine),
            style = MaterialTheme.typography.bodyLarge,
            color = MaterialTheme.colorScheme.onSurface,
        )
        KeyPlate(text = pinned, label = stringResource(R.string.hostkey_changed_pinned))
        KeyPlate(text = offered, label = stringResource(R.string.hostkey_changed_offered))
        Text(
            text = stringResource(R.string.hostkey_changed_advice),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Column(
            modifier = Modifier.padding(top = spacing.sm),
            verticalArrangement = Arrangement.spacedBy(spacing.sm),
        ) {
            Button(onClick = onBack, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.hostkey_keep_blocked))
            }
            OutlinedButton(
                onClick = { confirming = true },
                modifier = Modifier.fillMaxWidth(),
                colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.error),
            ) {
                Text(stringResource(R.string.hostkey_remove_pin))
            }
        }
    }
    if (confirming) {
        ConfirmDialog(
            title = stringResource(R.string.hostkey_remove_title, machine),
            body = stringResource(R.string.hostkey_remove_body, machine),
            confirmLabel = stringResource(R.string.hostkey_remove_pin),
            onConfirm = {
                confirming = false
                onRemovePin()
            },
            onDismiss = { confirming = false },
        )
    }
}
