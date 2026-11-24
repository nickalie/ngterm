package com.nickalie.ngterm.android.ui.asks

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.nickalie.ngterm.android.AppGraph
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.ui.components.StatusGlyph
import com.nickalie.ngterm.android.ui.components.agentStatusLook
import com.nickalie.ngterm.android.ui.components.agentStatusWord
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.PickReply
import com.nickalie.ngterm.core.ffi.PickRow
import com.nickalie.ngterm.core.ffi.RemotePick
import com.nickalie.ngterm.core.ffi.RemotePickItem

// the rows a phone's sheet holds before it scrolls them, so the field and Cancel stay on screen
private val ROWS_MAX_HEIGHT = 360.dp

/**
 * The first open pick of the session on screen, as a sheet over it, once no ask of that session is open (the ask's
 * sheet comes first). Its answer, or a dismissal (Cancel, Back or a swipe: a cancel on the origin), goes to the core
 * once. A pick out of sight waits until its session is opened.
 */
@Composable
fun PickPrompt(graph: AppGraph) {
    val open by graph.picks.open.collectAsStateWithLifecycle()
    val asks by graph.asks.open.collectAsStateWithLifecycle()
    val shown by graph.asks.onScreen.collectAsStateWithLifecycle()
    if (asks.any { it.key == shown }) return
    val pick = open.firstOrNull { it.key == shown } ?: return
    key(pick.key, pick.pick.id) {
        PickSheet(
            pick = pick.pick,
            rows = { query -> graph.picks.rows(pick.pick, query) },
            onAnswer = { reply -> graph.picks.answer(pick.key, pick.pick.id, reply) },
        )
    }
}

/** [PickContent] in a sheet, its search text kept through a configuration change; dismissing the sheet cancels. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PickSheet(
    pick: RemotePick,
    rows: (String) -> List<PickRow>,
    onAnswer: (PickReply) -> Unit,
    modifier: Modifier = Modifier,
) {
    ModalBottomSheet(
        onDismissRequest = { onAnswer(PickReply.Cancelled) },
        modifier = modifier,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = MaterialTheme.shapes.large,
    ) {
        PickContent(pick = pick, rows = rows, onAnswer = onAnswer)
    }
}

/**
 * A pick: its prompt as the heading, the search field (the origin's query in it), the rows the text shows (an item's
 * label over its subtitle, its status glyph at the end; or `Use "…"` for a custom answer; "No matches" for none), and
 * Cancel. A row's tap answers with it; the keyboard's Go answers with the marked row: the origin's selection while the
 * text is as it opened, else the first.
 */
@Composable
fun PickContent(
    pick: RemotePick,
    rows: (String) -> List<PickRow>,
    onAnswer: (PickReply) -> Unit,
    modifier: Modifier = Modifier,
) {
    val spacing = NgtermTheme.spacing
    var query by rememberSaveable(pick.id) { mutableStateOf(pick.query.orEmpty()) }
    val shown = remember(query) { rows(query) }
    var marked by rememberSaveable(pick.id) { mutableIntStateOf(initialRow(shown, pick.items, pick.selection)) }
    Column(
        modifier =
            modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .padding(horizontal = spacing.xl)
                .padding(bottom = spacing.lg),
        verticalArrangement = Arrangement.spacedBy(spacing.md),
    ) {
        Text(
            text = pick.prompt?.takeIf { it.isNotBlank() } ?: stringResource(R.string.pick_title),
            style = MaterialTheme.typography.headlineSmall,
            color = MaterialTheme.colorScheme.onSurface,
            modifier = Modifier.semantics { heading() },
        )
        OutlinedTextField(
            value = query,
            onValueChange = {
                if (!hasForbiddenControls(it, multiline = false)) {
                    query = it
                    marked = 0
                }
            },
            modifier = Modifier.fillMaxWidth(),
            placeholder = { Text(stringResource(R.string.pick_search_hint)) },
            leadingIcon = { Icon(painterResource(R.drawable.ic_tool_search), contentDescription = null) },
            singleLine = true,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Go),
            keyboardActions =
                KeyboardActions(
                    onGo = { shown.getOrNull(marked)?.let { onAnswer(replyFor(it, pick.items)) } },
                ),
            shape = MaterialTheme.shapes.medium,
        )
        if (shown.isEmpty()) {
            Text(
                text = stringResource(R.string.pick_no_matches),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(vertical = spacing.md),
            )
        } else {
            LazyColumn(Modifier.fillMaxWidth().heightIn(max = ROWS_MAX_HEIGHT)) {
                itemsIndexed(shown) { index, row ->
                    PickRowView(row, pick.items, marked = index == marked) { onAnswer(replyFor(row, pick.items)) }
                }
            }
        }
        TextButton(onClick = { onAnswer(PickReply.Cancelled) }, modifier = Modifier.fillMaxWidth()) {
            Text(stringResource(R.string.pick_cancel))
        }
    }
}

@Composable
private fun PickRowView(
    row: PickRow,
    items: List<RemotePickItem>,
    marked: Boolean,
    onClick: () -> Unit,
) {
    val spacing = NgtermTheme.spacing
    val colors = MaterialTheme.colorScheme
    Row(
        modifier =
            Modifier
                .fillMaxWidth()
                .heightIn(min = spacing.touchTarget + spacing.sm)
                .background(
                    if (marked) colors.surfaceContainerHighest else colors.surfaceContainerHigh,
                    MaterialTheme.shapes.small,
                ).clickable(role = Role.Button, onClick = onClick)
                .padding(horizontal = spacing.md, vertical = spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(spacing.md),
    ) {
        when (row) {
            is PickRow.Custom -> {
                Text(row.label, style = MaterialTheme.typography.bodyLarge, color = colors.primary)
            }

            is PickRow.Item -> {
                val item = items.getOrNull(row.index.toInt()) ?: return@Row
                Column(Modifier.weight(1f)) {
                    Text(
                        item.label,
                        style = MaterialTheme.typography.bodyLarge,
                        color = colors.onSurface,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    item.subtitle?.takeIf { it.isNotBlank() }?.let {
                        Text(
                            it,
                            style = MaterialTheme.typography.bodySmall,
                            color = colors.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                ItemStatus(item)
            }
        }
    }
}

// the item's status glyph as the sidebar draws it: none for an absent or idle status; TalkBack reads its word
@Composable
private fun ItemStatus(item: RemotePickItem) {
    val status = item.status?.takeIf { it.isNotBlank() && it != IDLE } ?: return
    val look = agentStatusLook(status, item.statusColor, item.statusShape)
    val word = agentStatusWord(status)
    StatusGlyph(look.shape, look.color, modifier = Modifier.semantics { contentDescription = word }, size = 16.dp)
}

private const val IDLE = "idle"

/** The row the keyboard's Go answers with at open: the item [selection] names when [rows] shows it, else the first. */
internal fun initialRow(
    rows: List<PickRow>,
    items: List<RemotePickItem>,
    selection: String?,
): Int {
    if (selection == null) return 0
    val at = rows.indexOfFirst { it is PickRow.Item && items.getOrNull(it.index.toInt())?.id == selection }
    return if (at < 0) 0 else at
}

/** What answering with [row] sends: the item's id, or the custom text. */
internal fun replyFor(
    row: PickRow,
    items: List<RemotePickItem>,
): PickReply =
    when (row) {
        is PickRow.Custom -> PickReply.Custom(row.query)
        is PickRow.Item -> items.getOrNull(row.index.toInt())?.let { PickReply.Picked(it.id) } ?: PickReply.Cancelled
    }
