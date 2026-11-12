package com.nickalie.ngterm.android.ui.terminal

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.ClipboardRead
import com.nickalie.ngterm.android.data.ClipboardReads
import com.nickalie.ngterm.android.data.ClipboardSnapshot
import com.nickalie.ngterm.android.ui.components.StatusGlyph
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusShape
import kotlinx.coroutines.flow.catch

/** The most lines of an unsafe paste the confirmation shows. */
private const val PREVIEW_LINES = 6

/**
 * The confirmation before pasting [text] the pane refused as unsafe: it has a line break (or a control character),
 * so the program may take it as Enter and run it at once. The text shows as it is on a plate, its first lines; Paste
 * repeats the toolbar's word, and Cancel is the safe default.
 */
@Composable
fun PasteConfirmDialog(
    text: String,
    onPaste: () -> Unit,
    onDismiss: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    AlertDialog(
        onDismissRequest = onDismiss,
        modifier = modifier,
        title = { Text(stringResource(R.string.paste_unsafe_title)) },
        text = {
            Column(verticalArrangement = Arrangement.spacedBy(spacing.md)) {
                Text(stringResource(R.string.paste_unsafe_body), style = MaterialTheme.typography.bodyMedium)
                Text(
                    text = text,
                    style = NgtermTheme.mono.small,
                    color = MaterialTheme.colorScheme.onSurface,
                    maxLines = PREVIEW_LINES,
                    overflow = TextOverflow.Ellipsis,
                    modifier =
                        Modifier
                            .fillMaxWidth()
                            .background(MaterialTheme.colorScheme.surfaceContainerLow, MaterialTheme.shapes.small)
                            .padding(spacing.md),
                )
            }
        },
        confirmButton = { TextButton(onClick = onPaste) { Text(stringResource(R.string.terminal_paste)) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text(stringResource(R.string.action_cancel)) } },
        shape = MaterialTheme.shapes.large,
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
    )
}

/**
 * The first clipboard read [graph]'s panes are waiting on, as a sheet over whatever is shown: the machine and the
 * session it comes from by their names, what it would hand over ([ClipboardReads.preview], read once the sheet is up;
 * a clipboard that fails to read refuses the read, and the sheet goes),
 * and Allow, Always allow on the machine, or Deny. Swiping it away denies.
 */
@Composable
fun ClipboardReadPrompt(graph: AppGraph) {
    val pending by graph.clipboardReads.pending.collectAsStateWithLifecycle()
    val read = pending.firstOrNull() ?: return
    val machinesFlow = remember(graph) { graph.machines.machines.catch { emit(emptyList()) } }
    val machines by machinesFlow.collectAsStateWithLifecycle(emptyList())
    val trees by graph.connections.trees.collectAsStateWithLifecycle()
    val pane = read.pane
    val machine = machines.firstOrNull { it.id == pane.machineId }?.title ?: pane.machineId
    val session = trees[pane.machineId]?.firstOrNull { it.id == pane.sessionId }?.name ?: pane.sessionId
    val reads = graph.clipboardReads
    key(read) {
        val preview by produceState<ClipboardSnapshot?>(null, read) { value = reads.preview(read) }
        ClipboardReadSheet(
            read = read,
            machine = machine,
            session = session,
            preview = preview,
            onAllow = { remember -> preview?.let { reads.allow(read, it, remember) } },
            onAlways = { remember -> preview?.let { reads.allowAlways(read, it, remember) } },
            onDeny = { reads.deny(read) },
        )
    }
}

/** [ClipboardReadQuestion] in a sheet; dismissing the sheet is [onDeny]. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun ClipboardReadSheet(
    read: ClipboardRead,
    machine: String,
    session: String,
    preview: ClipboardSnapshot?,
    onAllow: (remember: Boolean) -> Unit,
    onAlways: (remember: Boolean) -> Unit,
    onDeny: () -> Unit,
    modifier: Modifier = Modifier,
) {
    ModalBottomSheet(
        onDismissRequest = onDeny,
        modifier = modifier,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.large,
    ) {
        ClipboardReadQuestion(read.canRemember, machine, session, preview, onAllow, onAlways, onDeny)
    }
}

/**
 * The question a program's clipboard read asks: the attention diamond, "Let docs-site read your clipboard?", from
 * where, what it would see ([ClipboardPreview]), the "don't ask again" switch when the read [canRemember], then Allow
 * (filled), Always allow on [machine] (outlined) and Deny. Allow and Always stay disabled until [preview] is read.
 */
@Composable
internal fun ClipboardReadQuestion(
    canRemember: Boolean,
    machine: String,
    session: String,
    preview: ClipboardSnapshot?,
    onAllow: (remember: Boolean) -> Unit,
    onAlways: (remember: Boolean) -> Unit,
    onDeny: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    var remember by rememberSaveable { mutableStateOf(false) }
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
            text = stringResource(R.string.clipboard_read_title, session),
            style = MaterialTheme.typography.headlineSmall,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.semantics { heading() },
        )
        Text(
            text = stringResource(R.string.clipboard_read_body, session, machine),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        ClipboardPreview(preview)
        if (canRemember) {
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .heightIn(min = spacing.touchTarget)
                        .toggleable(value = remember, role = Role.Switch, onValueChange = { remember = it }),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(spacing.md),
            ) {
                Text(
                    text = stringResource(R.string.clipboard_read_remember),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurface,
                    modifier = Modifier.weight(1f),
                )
                Switch(checked = remember, onCheckedChange = null)
            }
        }
        Column(verticalArrangement = Arrangement.spacedBy(spacing.sm)) {
            // nothing is handed over before the user has seen it: Allow and Always wait for the preview
            val shown = preview != null
            Button(onClick = { onAllow(remember) }, enabled = shown, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.clipboard_read_allow))
            }
            OutlinedButton(onClick = { onAlways(remember) }, enabled = shown, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.clipboard_read_always, machine))
            }
            TextButton(onClick = onDeny, modifier = Modifier.fillMaxWidth()) {
                Text(stringResource(R.string.clipboard_read_deny))
            }
        }
    }
}
