package com.nickalie.ngterm.android.ui.chat

import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.Saver
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatBody
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.flow.filter

/** The key of the list's first row: earlier messages, the conversation's start, or why they couldn't be read. */
internal const val OLDER_KEY = "older"

/**
 * A chat's rows, oldest first, in one column no wider than a reading width: the [OlderHeader] on top, which asks for
 * the page before ([ChatActions.onLoadOlder]) as soon as it shows, then each row. The list opens at the newest row and
 * follows new ones while it is at the end; a page that goes in above keeps the row that was on top in place
 * ([ChatUiState.anchor]). Which rows are open (a tool's output, a subagent's steps) survives scrolling and rotation.
 */
@Composable
fun ChatList(
    state: ChatUiState,
    actions: ChatActions,
    modifier: Modifier = Modifier,
) {
    val listState = rememberLazyListState()
    val items = state.items
    var expanded by rememberSaveable(stateSaver = ExpandedSaver) { mutableStateOf(emptySet<String>()) }
    val toggle: (String) -> Unit = { key -> expanded = if (key in expanded) expanded - key else expanded + key }
    val readable = NgtermTheme.spacing.readableWidth
    FollowEnd(listState, items)
    KeepAnchor(listState, state.anchor, items)
    LazyColumn(
        state = listState,
        modifier = modifier.fillMaxSize(),
        contentPadding = PaddingValues(horizontal = NgtermTheme.spacing.lg, vertical = NgtermTheme.spacing.sm),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        item(key = OLDER_KEY, contentType = OLDER_KEY) {
            OlderHeader(
                older = state.older,
                cwd = state.conversation?.cwd,
                onRetry = actions.onLoadOlder,
                modifier = Modifier.widthIn(max = readable),
            )
            AskOlder(listState, state.older, actions.onLoadOlder)
        }
        items(items, key = { it.key }, contentType = { it.contentType }) { item ->
            ChatRow(
                item = item,
                expanded = expanded,
                onToggle = toggle,
                modifier = Modifier.widthIn(max = readable).fillMaxWidth().padding(vertical = item.gap),
            )
        }
    }
}

// asks for the page before once the header is on screen (not only composed ahead of it), whenever more may remain
@Composable
private fun AskOlder(
    listState: LazyListState,
    older: OlderPages,
    onLoadOlder: () -> Unit,
) {
    val load by rememberUpdatedState(onLoadOlder)
    LaunchedEffect(older) {
        if (older != OlderPages.More) return@LaunchedEffect
        // a page that just went in moves the list in this frame: look once it is laid out
        withFrameNanos { }
        if (listState.layoutInfo.visibleItemsInfo.any { it.key == OLDER_KEY }) load()
    }
}

// while the list is at its end (where it opens), a new or changed last row keeps it there; a scroll away lets it be
@Composable
private fun FollowEnd(
    listState: LazyListState,
    items: List<ChatItem>,
) {
    var following by remember { mutableStateOf(true) }
    LaunchedEffect(listState) {
        // each scroll that ends (not the list's start, before it is scrolled to its end) says where it stopped
        snapshotFlow { listState.isScrollInProgress }
            .drop(1)
            .filter { !it }
            .collect { following = !listState.canScrollForward }
    }
    val last = items.lastOrNull()
    LaunchedEffect(last) {
        // the header is the list's first item, so the last row's index is the rows' count
        if (last != null && following) listState.scrollToItem(items.size, END_OFFSET)
    }
}

// past any row's height: the list stops at its end
private const val END_OFFSET = 1_000_000

// a page went in above the rows while the header was on top: the row that was on top goes back there
@Composable
private fun KeepAnchor(
    listState: LazyListState,
    anchor: String?,
    items: List<ChatItem>,
) {
    val now by rememberUpdatedState(items)
    LaunchedEffect(anchor) {
        val key = anchor ?: return@LaunchedEffect
        val index = now.indexOfFirst { it.key == key }
        if (index >= 0 && listState.firstVisibleItemIndex == 0) listState.scrollToItem(index + 1)
    }
}

/** A row of the chat, by what it is, with its subagent's steps under a task; [expanded] holds the open rows' keys. */
@Composable
fun ChatRow(
    item: ChatItem,
    expanded: Set<String>,
    onToggle: (String) -> Unit,
    modifier: Modifier = Modifier,
) {
    val open = item.key in expanded
    val toggle = { onToggle(item.key) }
    val rows: @Composable () -> Unit = { item.steps.forEach { ChatRow(it, expanded, onToggle) } }
    when (item) {
        ChatItem.Divider -> ConversationDivider(modifier)
        is ChatItem.Subagent -> SubagentRow(item.children.size, open, toggle, modifier, rows)
        is ChatItem.Entry -> EntryRow(item.body, item.children.size, open, toggle, modifier, rows)
    }
}

@Composable
private fun EntryRow(
    body: ChatBody,
    steps: Int,
    open: Boolean,
    onToggle: () -> Unit,
    modifier: Modifier = Modifier,
    children: @Composable () -> Unit,
) {
    when (body) {
        is ChatBody.User -> PromptBubble(body.text, body.images.toInt(), modifier)

        is ChatBody.Assistant -> ReplyText(body.text, modifier)

        is ChatBody.Thinking -> ThinkingRow(body.text, open, onToggle, modifier)

        is ChatBody.Tool -> ToolRow(body.tool, open, onToggle, modifier, steps, children)

        is ChatBody.Question -> QuestionRow(body.question, open, onToggle, modifier)

        is ChatBody.Notice -> NoticeRow(body.kind, body.text, body.detail, open, onToggle, modifier)

        // the rest is the view model's, never a row
        else -> Unit
    }
}

// a task's subagent steps
private val ChatItem.steps: List<ChatItem>
    get() =
        when (this) {
            is ChatItem.Entry -> children
            is ChatItem.Subagent -> children
            ChatItem.Divider -> emptyList()
        }

// the room around a row: a prompt opens a turn, a reply breathes, the one-line rows sit close
private val ChatItem.gap: Dp
    get() =
        when ((this as? ChatItem.Entry)?.body) {
            is ChatBody.User -> 8.dp
            is ChatBody.Assistant -> 4.dp
            else -> 0.dp
        }

private val ChatItem.contentType: String
    get() =
        when (this) {
            ChatItem.Divider -> {
                "divider"
            }

            is ChatItem.Subagent -> {
                "line"
            }

            is ChatItem.Entry -> {
                when (body) {
                    is ChatBody.User -> "prompt"
                    is ChatBody.Assistant -> "reply"
                    else -> "line"
                }
            }
        }

// the open rows' keys, as saved state
private val ExpandedSaver =
    Saver<Set<String>, ArrayList<String>>(
        save = { ArrayList(it) },
        restore = { it.toSet() },
    )
