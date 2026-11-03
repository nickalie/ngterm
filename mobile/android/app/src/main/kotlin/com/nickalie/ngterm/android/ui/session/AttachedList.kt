package com.nickalie.ngterm.android.ui.session

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.ui.components.AgentStatusBadge
import com.nickalie.ngterm.android.ui.components.BusBar
import com.nickalie.ngterm.android.ui.components.SectionHeader
import com.nickalie.ngterm.android.ui.components.SessionRow
import com.nickalie.ngterm.android.ui.components.StatusGlyph
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.android.ui.theme.StatusShape
import com.nickalie.ngterm.android.ui.theme.parseHexColor
import com.nickalie.ngterm.android.ui.theme.readableOn
import com.nickalie.ngterm.android.ui.theme.textGrounds

/**
 * The attached sessions as the Sessions destination lists them when none is open: a heading per machine, then a row
 * per session with its status and a menu (Detach, End session: [onEnd] confirms first). Tapping a row opens it
 * ([onOpen]). [contentPadding] keeps the list clear of the bars.
 */
@Composable
fun AttachedList(
    groups: List<AttachedGroup>,
    onOpen: (SessionKey) -> Unit,
    onDetach: (SessionKey) -> Unit,
    onEnd: (SessionKey) -> Unit,
    modifier: Modifier = Modifier,
    contentPadding: PaddingValues = PaddingValues(),
) {
    LazyColumn(
        modifier = modifier.widthIn(max = NgtermTheme.spacing.readableWidth),
        contentPadding = contentPadding,
    ) {
        groups.forEach { group ->
            item(key = "machine:${group.machineId}") {
                SectionHeader(group.machine, count = group.sessions.size)
            }
            items(group.sessions, key = { "${it.key.machineId}/${it.key.sessionId}" }) { session ->
                SessionRow(
                    name = session.name,
                    onClick = { onOpen(session.key) },
                    context = session.context,
                    menu = { ListMenu(session.name, { onDetach(session.key) }, { onEnd(session.key) }) },
                ) { Status(session) }
            }
        }
    }
}

/**
 * The attached sessions in the sidebar (and the open rail): a heading per machine, a row per session with its status,
 * the bus bar on the [open] one. Tapping a row opens it ([onOpen]).
 */
@Composable
fun ColumnScope.SidebarSessions(
    groups: List<AttachedGroup>,
    open: SessionKey?,
    onOpen: (SessionKey) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState())) {
        groups.forEach { group ->
            SectionHeader(group.machine, count = group.sessions.size)
            group.sessions.forEach { session ->
                SessionRow(
                    name = session.name,
                    onClick = { onOpen(session.key) },
                    context = session.context,
                    selected = session.key == open,
                ) { Status(session) }
            }
        }
    }
}

/**
 * The attached sessions in the rail: the sidebar's rows while it is [expanded], its cells otherwise. Tapping one opens
 * it ([onOpen]).
 */
@Composable
fun ColumnScope.RailSessions(
    groups: List<AttachedGroup>,
    open: SessionKey?,
    expanded: Boolean,
    onOpen: (SessionKey) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (expanded) SidebarSessions(groups, open, onOpen, modifier) else RailCells(groups, open, onOpen, modifier)
}

/**
 * The attached sessions in the collapsed rail: a cell each, its agent's status glyph over its name cut to fit, the
 * bus bar on the [open] one. TalkBack reads a cell as its name and status.
 */
@Composable
private fun ColumnScope.RailCells(
    groups: List<AttachedGroup>,
    open: SessionKey?,
    onOpen: (SessionKey) -> Unit,
    modifier: Modifier = Modifier,
) {
    Column(
        modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()),
        verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.xs),
    ) {
        groups.flatMap { it.sessions }.forEach { session -> RailSession(session, session.key == open, onOpen) }
    }
}

@Composable
private fun RailSession(
    session: AttachedSession,
    selected: Boolean,
    onOpen: (SessionKey) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = MaterialTheme.colorScheme
    val status = session.status
    val description = listOfNotNull(session.name, status?.text).joinToString(", ")
    Row(
        modifier =
            modifier
                .fillMaxWidth()
                .heightIn(min = RailCell)
                .selectable(selected = selected, role = Role.Button, onClick = { onOpen(session.key) })
                .clearAndSetSemantics { contentDescription = description },
        verticalAlignment = Alignment.CenterVertically,
    ) {
        BusBar(active = selected)
        Column(
            modifier = Modifier.weight(1f).padding(end = NgtermTheme.spacing.busBar),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(NgtermTheme.spacing.xs),
        ) {
            val tint =
                parseHexColor(status?.color)?.let { readableOn(it, colors.textGrounds()) } ?: NgtermTheme.status.idle
            StatusGlyph(StatusShape.fromWire(status?.shape), tint)
            Text(
                text = session.name,
                style = MaterialTheme.typography.labelSmall,
                color = if (selected) colors.primary else colors.onSurfaceVariant,
                textAlign = TextAlign.Center,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.padding(horizontal = NgtermTheme.spacing.xs),
            )
        }
    }
}

@Composable
private fun Status(
    session: AttachedSession,
    modifier: Modifier = Modifier,
) {
    session.status?.let { AgentStatusBadge(it.text, it.color, it.shape, modifier) }
}

@Composable
private fun ListMenu(
    name: String,
    onDetach: () -> Unit,
    onEnd: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var open by remember { mutableStateOf(false) }
    Box(modifier = modifier) {
        IconButton(onClick = { open = true }) {
            Icon(
                painterResource(R.drawable.ic_more),
                contentDescription = stringResource(R.string.action_more, name),
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(
                text = { Text(stringResource(R.string.machine_sessions_detach)) },
                onClick = {
                    open = false
                    onDetach()
                },
            )
            DropdownMenuItem(
                text = { Text(stringResource(R.string.machine_sessions_end), color = MaterialTheme.colorScheme.error) },
                onClick = {
                    open = false
                    onEnd()
                },
            )
        }
    }
}

private val RailCell = 64.dp
