package com.nickalie.ngterm.android.ui.settings

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.selectableGroup
import androidx.compose.foundation.selection.toggleable
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme

/**
 * A group of settings under its [title] (a heading), its rows flush with the heading; the [first] has no gap above
 * it, the screen's title being there.
 */
@Composable
internal fun SettingsSection(
    title: String,
    modifier: Modifier = Modifier,
    first: Boolean = false,
    content: @Composable ColumnScope.() -> Unit,
) {
    Column(modifier.fillMaxWidth()) {
        Text(
            text = title,
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier =
                Modifier
                    .padding(top = if (first) 0.dp else NgtermTheme.spacing.xl, bottom = NgtermTheme.spacing.xs)
                    .semantics { heading() },
        )
        content()
    }
}

/** A setting's words: its [label] over its [note]. */
@Composable
private fun Words(
    label: String,
    note: String?,
    modifier: Modifier = Modifier,
) {
    Column(modifier, verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.xxs)) {
        Text(label, style = MaterialTheme.typography.bodyLarge, color = MaterialTheme.colorScheme.onSurface)
        if (note != null) {
            Text(note, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

/** An on-or-off setting: the whole row toggles it, the switch at its end shows which. */
@Composable
internal fun SwitchRow(
    label: String,
    checked: Boolean,
    onCheckedChange: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
    note: String? = null,
) {
    val spacing = NgtermTheme.spacing
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .heightIn(min = RowHeight)
                .toggleable(value = checked, role = Role.Switch, onValueChange = onCheckedChange)
                .padding(vertical = spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(spacing.lg),
    ) {
        Words(label, note, Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = null)
    }
}

/** One choice of a setting's [options]: a radio row each, the whole row picks it. */
internal class Choice<T>(
    val value: T,
    val label: String,
    val note: String? = null,
)

/**
 * A setting with one of [options] picked ([selected]): a radio row per option in a group TalkBack reads as one, with
 * [trailing] drawn at an option's end (a palette's colors).
 */
@Composable
internal fun <T> ChoiceGroup(
    options: List<Choice<T>>,
    selected: T,
    onSelect: (T) -> Unit,
    modifier: Modifier = Modifier,
    trailing: (@Composable (T) -> Unit)? = null,
) {
    val spacing = NgtermTheme.spacing
    Column(modifier.fillMaxWidth().selectableGroup()) {
        for (option in options) {
            val picked = option.value == selected
            Row(
                modifier =
                    Modifier
                        .fillMaxWidth()
                        .heightIn(min = RowHeight)
                        .selectable(selected = picked, role = Role.RadioButton, onClick = { onSelect(option.value) })
                        .padding(vertical = spacing.sm),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(spacing.md),
            ) {
                RadioButton(selected = picked, onClick = null)
                Words(option.label, option.note, Modifier.weight(1f))
                trailing?.invoke(option.value)
            }
        }
    }
}

/** A line about a group as a whole, under its rows. */
@Composable
internal fun SettingsNote(
    text: String,
    modifier: Modifier = Modifier,
) {
    Text(
        text = text,
        style = MaterialTheme.typography.bodySmall,
        color = MaterialTheme.colorScheme.onSurfaceVariant,
        modifier = modifier.padding(vertical = NgtermTheme.spacing.sm),
    )
}

/** A settings row's least height (DESIGN.md: rows are 56dp). */
private val RowHeight = 56.dp
