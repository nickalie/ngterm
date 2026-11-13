package com.nickalie.ngterm.android.ui.components

import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RichTooltip
import androidx.compose.material3.Text
import androidx.compose.material3.TooltipAnchorPosition
import androidx.compose.material3.TooltipBox
import androidx.compose.material3.TooltipDefaults
import androidx.compose.material3.minimumInteractiveComponentSize
import androidx.compose.material3.rememberTooltipState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlinx.coroutines.launch

/**
 * The app a machine runs when it isn't ngterm ([word]: "agterm"): the word in dim ink inside a hairline outline, so it
 * reads as a fact about the machine, not a status or a place. With a [note] (what that app lacks) a tap, or a long
 * press, shows the note in a tooltip, and TalkBack reads the word and the note; without one it is a plain label.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PeerBadge(
    word: String,
    modifier: Modifier = Modifier,
    note: String? = null,
) {
    if (note == null) {
        Label(word, modifier.semantics { contentDescription = word })
        return
    }
    val state = rememberTooltipState(isPersistent = true)
    val scope = rememberCoroutineScope()
    val description = stringResource(R.string.peer_badge_description, word, note)
    TooltipBox(
        positionProvider = TooltipDefaults.rememberTooltipPositionProvider(TooltipAnchorPosition.Below),
        tooltip = {
            RichTooltip(title = { Text(word) }) { Text(note) }
        },
        state = state,
        modifier = modifier,
    ) {
        Box(
            modifier =
                Modifier
                    .minimumInteractiveComponentSize()
                    .clip(MaterialTheme.shapes.small)
                    .clickable(
                        role = Role.Button,
                        onClickLabel = stringResource(R.string.peer_badge_explain),
                        onClick = { scope.launch { state.show() } },
                    ).semantics { contentDescription = description },
            contentAlignment = Alignment.Center,
        ) {
            Label(word, Modifier.clearAndSetSemantics {})
        }
    }
}

@Composable
private fun Label(
    word: String,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    Text(
        text = word,
        style = MaterialTheme.typography.labelMedium,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        maxLines = 1,
        modifier =
            modifier
                .border(1.dp, MaterialTheme.colorScheme.outline, MaterialTheme.shapes.small)
                .padding(horizontal = spacing.sm, vertical = spacing.xxs),
    )
}
