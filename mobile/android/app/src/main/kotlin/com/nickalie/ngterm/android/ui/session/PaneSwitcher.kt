package com.nickalie.ngterm.android.ui.session

import androidx.compose.animation.animateColorAsState
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.BusBar
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * A session's panes as numbered tabs in the origin's order, the one on screen or with the focus ([selected]) in copper
 * with the bus bar under it. TalkBack reads a tab as "Pane 1 of 2" and the pane's title when its program set one
 * ([titles], by pane id). With one pane there is nothing to switch, and it draws nothing. Where the window can show
 * the split ([alone] not `null`), a tab also toggles it ([tabTapAlone]), and TalkBack names what its tap does: "Show
 * this pane alone" in the split, "Show both panes" on the pane shown alone.
 */
@Composable
fun PaneSwitcher(
    panes: List<String>,
    selected: String?,
    titles: Map<String, String?>,
    onSelect: (String) -> Unit,
    modifier: Modifier = Modifier,
    alone: Boolean? = null,
) {
    if (panes.size < 2) return
    Row(modifier = modifier.selectableGroup()) {
        panes.forEachIndexed { index, pane ->
            val title = titles[pane]?.takeIf { it.isNotBlank() }
            val label =
                if (title != null) {
                    stringResource(R.string.session_pane_titled, index + 1, panes.size, title)
                } else {
                    stringResource(R.string.session_pane, index + 1, panes.size)
                }
            val action =
                when {
                    alone == false -> stringResource(R.string.session_pane_show_alone)
                    alone == true && pane == selected -> stringResource(R.string.session_pane_show_both)
                    else -> null
                }
            PaneTab(index + 1, pane == selected, label, { onSelect(pane) }, action = action)
        }
    }
}

@Composable
private fun PaneTab(
    number: Int,
    selected: Boolean,
    label: String,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
    action: String? = null,
) {
    val colors = MaterialTheme.colorScheme
    val tint by animateColorAsState(
        targetValue = if (selected) colors.primary else colors.onSurfaceVariant,
        animationSpec = NgtermTheme.motion.quick(),
        label = "pane tab",
    )
    Box(
        modifier =
            modifier
                .size(NgtermTheme.spacing.touchTarget)
                .clip(MaterialTheme.shapes.small)
                .selectable(selected = selected, role = Role.Tab, onClick = onClick)
                // the tab is read by its label, not its number; the click, the role and the state stay
                .clearAndSetSemantics {
                    contentDescription = label
                    // TalkBack says what the tap does where it toggles the split
                    action?.let {
                        onClick(label = it) {
                            onClick()
                            true
                        }
                    }
                },
        contentAlignment = Alignment.Center,
    ) {
        Text(text = number.toString(), style = MaterialTheme.typography.titleSmall, color = tint)
        BusBar(
            active = selected,
            vertical = false,
            length = 20.dp,
            modifier = Modifier.align(Alignment.BottomCenter),
        )
    }
}
