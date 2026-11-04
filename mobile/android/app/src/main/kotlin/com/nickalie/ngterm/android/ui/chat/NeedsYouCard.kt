package com.nickalie.ngterm.android.ui.chat

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.StatusGlyph
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusShape

/**
 * A card pinned at the chat's foot while the agent waits on the user, in the ask sheet's shape (`ui/asks/`): the
 * sheet's tone and top corners, the attention diamond over a [title] that is a heading, then [content]. It scrolls
 * inside when it is taller than it may grow.
 */
@Composable
fun NeedsYouCard(
    title: String,
    modifier: Modifier = Modifier,
    content: @Composable ColumnScope.() -> Unit,
) {
    val spacing = NgtermTheme.spacing
    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .background(MaterialTheme.colorScheme.surfaceContainerHigh, CardShape)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = spacing.xl, vertical = spacing.lg),
        verticalArrangement = Arrangement.spacedBy(spacing.md),
    ) {
        StatusGlyph(StatusShape.Diamond, NgtermTheme.status.attention, size = 20.dp)
        Text(
            text = title,
            style = MaterialTheme.typography.titleMedium,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.semantics { heading() },
        )
        content()
    }
}

// the ask sheet's top corners: the card rises from the chat's foot as the sheet does from the screen's
private val CardShape = RoundedCornerShape(topStart = 20.dp, topEnd = 20.dp)

/**
 * The permission prompt the agent waits on: "Allow Bash?", the call's title in the terminal's face, its detail (a
 * command, or a diff) on a plate that scrolls past [DetailHeight], then the dialog's options in its order, one per
 * line: the first, the dialog's own default, filled; the rest outlined. Until the pane's screen is read the agent's
 * own answers stand in ([genericApprovalLabels] for [agent]). While an answer is on its way ([busy]) they hold still.
 */
@Composable
fun ApprovalCardView(
    card: ApprovalCard,
    agent: String?,
    busy: Boolean,
    onAnswer: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val approval = card.approval
    val labels = card.labels.ifEmpty { genericApprovalLabels(agent) }
    // keyed by the prompt: another prompt's scroll positions, live or saved, stay its own
    key(card.id) {
        NeedsYouCard(stringResource(R.string.chat_approval_title, approval.tool), modifier) {
            approval.title.takeIf { it.isNotBlank() }?.let {
                Text(it, style = ChatType.code, color = MaterialTheme.colorScheme.onSurface)
            }
            approval.detail?.takeIf { it.isNotBlank() && it != approval.title }?.let { detail ->
                val plate = Modifier.heightIn(max = DetailHeight).verticalScroll(rememberScrollState())
                if (detail.looksLikeDiff()) DiffView(detail, plate) else OutputPlate(detail, plate)
            }
            CardButtons(labels, busy, onAnswer)
        }
    }
}

// the tallest an approval's detail grows in its card before it scrolls
private val DetailHeight = 160.dp

private fun String.looksLikeDiff(): Boolean = startsWith("diff ") || startsWith("--- ") || startsWith("@@")

// the options, one per line in their order: the first filled, the rest outlined, none while one is on its way
// ([busy], which says Sending under them) or until the card can be answered ([enabled])
@Composable
internal fun CardButtons(
    labels: List<String>,
    busy: Boolean,
    onPress: (String) -> Unit,
    modifier: Modifier = Modifier,
    enabled: Boolean = true,
) {
    val active = enabled && !busy
    Column(modifier.fillMaxWidth(), verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm)) {
        labels.forEachIndexed { index, label ->
            if (index == 0) {
                Button(
                    onClick = { onPress(label) },
                    enabled = active,
                    modifier = Modifier.fillMaxWidth(),
                ) { Text(label) }
            } else {
                OutlinedButton(onClick = { onPress(label) }, enabled = active, modifier = Modifier.fillMaxWidth()) {
                    Text(label)
                }
            }
        }
        if (busy) Sending()
    }
}

@Composable
private fun Sending(modifier: Modifier = Modifier) {
    Row(
        modifier = modifier.semantics(mergeDescendants = true) { liveRegion = LiveRegionMode.Polite },
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.sm),
    ) {
        CircularProgressIndicator(Modifier.size(16.dp), strokeWidth = 2.dp)
        Text(
            stringResource(R.string.chat_sending),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
