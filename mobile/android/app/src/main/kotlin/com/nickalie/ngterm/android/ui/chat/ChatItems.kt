package com.nickalie.ngterm.android.ui.chat

import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatEvent
import com.nickalie.ngterm.core.ffi.ChatQuestion
import com.nickalie.ngterm.core.ffi.ChatTool
import com.nickalie.ngterm.core.ffi.ChatToolStatus

/**
 * One row of a pane's chat, as the list shows it. Its [key] names its event, so it stays the same through updates,
 * older pages and a [ChatItem.Subagent] becoming its task's [ChatItem.Entry]; the [ChatItem.Divider] has its own.
 */
sealed interface ChatItem {
    val key: String

    /**
     * An event of the conversation, its [body] as the latest event with its [id] left it (a tool's result completes
     * its call), with the events of the subagent it started ([children], for a task tool) under it.
     */
    data class Entry(
        val id: String,
        val body: ChatBody,
        val children: List<ChatItem> = emptyList(),
    ) : ChatItem {
        override val key: String get() = rowKey(id)
    }

    /**
     * A subagent's events whose task is not in the chat (its call is older than what was read), under the task's
     * id ([parent]) until the task comes, which then takes them and this row's [key].
     */
    data class Subagent(
        val parent: String,
        val children: List<ChatItem>,
    ) : ChatItem {
        override val key: String get() = rowKey(parent)
    }

    /** The pane's conversation switched here: what was shown before it is gone, and what follows is the new one. */
    data object Divider : ChatItem {
        override val key: String get() = "divider"
    }
}

// an event's row, the same whether its body is read (an Entry) or only its subagent's events are (a Subagent)
private fun rowKey(id: String) = "e:$id"

/** An `AskUserQuestion` still waiting for its answer: the chat event's [id] and the [question]. */
data class OpenQuestion(
    val id: String,
    val question: ChatQuestion,
)

/**
 * A pane's chat folded into its rows ([items]), immutable: [append] takes the events the core streams, [prepend] an
 * older page, and [reset] starts over after the conversation switched ([switched] puts a [ChatItem.Divider] on top).
 *
 * - Events upsert by id: a later event with an id already shown updates its row in place, the newest word winning.
 *   A tool result whose call was outside the origin's window comes with no name or title, and keeps the ones its
 *   call gave.
 * - An older page is folded on its own first, oldest to newest (the origin repeats an id there as it changed: a
 *   tool's call, then its result), then goes in beneath the log: an id the log already holds keeps its place and
 *   what the log's newer events said, the page filling in only what they left out.
 * - An event with a `parent` (a subagent's) folds under the row with that id (its task tool), in the order read;
 *   while that row isn't here, under a [ChatItem.Subagent] in its place. The task, once it comes, takes the
 *   placeholder's events and key: live in the placeholder's place, from an older page at its own place in the page.
 * - Only what the list shows is kept: prompts, replies, thinking, tools, questions and notices. The conversation's
 *   own record, approvals, `live`, `reset` and entries the origin couldn't read are the view model's or nobody's.
 */
class ChatLog private constructor(
    // the top-level ids, oldest first, a missing task's id among them
    private val order: List<String>,
    // every kept event's latest body, by id
    private val bodies: Map<String, ChatBody>,
    // each parent's events, oldest first
    private val children: Map<String, List<String>>,
    // each subagent event's parent
    private val parents: Map<String, String>,
    val switched: Boolean,
) {
    constructor() : this(emptyList(), emptyMap(), emptyMap(), emptyMap(), false)

    /** The rows, oldest first. */
    val items: List<ChatItem> by lazy {
        val rows = order.map(::item)
        if (switched) listOf(ChatItem.Divider) + rows else rows
    }

    /** Whether nothing is shown. */
    val isEmpty: Boolean get() = order.isEmpty()

    /** The latest `AskUserQuestion` still waiting for its answer, leaving out [dismissed] ones. */
    fun openQuestion(dismissed: Set<String>): OpenQuestion? =
        (order + parents.keys)
            .asReversed()
            .firstNotNullOfOrNull { id ->
                val body = bodies[id] as? ChatBody.Question
                body
                    ?.takeIf { it.question.status == ChatToolStatus.RUNNING && id !in dismissed }
                    ?.let { OpenQuestion(id, it.question) }
            }

    /**
     * The conversation's own tool calls with no result yet, oldest first: a command or a subagent still on its way,
     * including one the agent left running in the background when its turn ended. A subagent's own steps count as
     * its task's, so only rows on the conversation's level are here.
     */
    val running: List<ChatTool> by lazy {
        order.mapNotNull { id -> (bodies[id] as? ChatBody.Tool)?.tool?.takeIf { it.status == ChatToolStatus.RUNNING } }
    }

    /** The log with [events] (newest last) taken in after what it shows. */
    fun append(events: List<ChatEvent>): ChatLog {
        val edit = Edit(this)
        for (event in events.filter(::kept)) edit.newer(event)
        return edit.done()
    }

    /** The log with an older page ([events], oldest first) put before what it shows. */
    fun prepend(events: List<ChatEvent>): ChatLog {
        val page = Page(Edit(this))
        for (event in folded(events.filter(::kept))) page.take(event)
        return page.done()
    }

    /** An empty log after the conversation switched: the divider, and nothing before it. */
    fun reset(): ChatLog = ChatLog(emptyList(), emptyMap(), emptyMap(), emptyMap(), switched = true)

    private fun item(id: String): ChatItem {
        val kids = children[id].orEmpty().map(::item)
        val body = bodies[id]
        return if (body != null) ChatItem.Entry(id, body, kids) else ChatItem.Subagent(id, kids)
    }

    // a log's parts, copied to change
    internal class Edit(
        log: ChatLog,
    ) {
        val order = log.order.toMutableList()
        val bodies = log.bodies.toMutableMap()
        val children = log.children.toMutableMap()
        val parents = log.parents.toMutableMap()
        private val switched = log.switched

        // a live event: it updates its row, fills its placeholder in place, or goes in last
        fun newer(event: ChatEvent) {
            val id = event.id
            val known = bodies[id]
            if (known != null) {
                bodies[id] = merge(older = known, newer = event.body)
                return
            }
            bodies[id] = event.body
            val parent = event.parent
            if (parent == null) {
                if (id !in order) order += id
            } else {
                // a placeholder whose task is itself a subagent's goes under that subagent's task
                order.remove(id)
                hang(id, parent)
            }
        }

        private fun hang(
            id: String,
            parent: String,
        ) {
            parents[id] = parent
            children[parent] = children[parent].orEmpty() + id
            // a parent with no body (every subagent event has one) stands in for its task
            if (parent !in bodies && parent !in order) order += parent
        }

        fun done() = ChatLog(order.toList(), bodies.toMap(), children.toMap(), parents.toMap(), switched)
    }
}

// an older page going in, folded by id: the rows it puts on top ([front]) and under each parent ([kids]), oldest first
private class Page(
    private val edit: ChatLog.Edit,
) {
    private val front = mutableListOf<String>()
    private val kids = mutableMapOf<String, MutableList<String>>()

    fun take(event: ChatEvent) {
        val id = event.id
        val known = edit.bodies[id]
        if (known != null) {
            // the log's word is newer: the page fills in only what it left out, and the row keeps its place
            edit.bodies[id] = merge(older = event.body, newer = known)
            return
        }
        edit.bodies[id] = event.body
        // a task its subagent's events stood in for (in the log, or earlier in the page) takes them, at its own place
        edit.order.remove(id)
        front.remove(id)
        val parent = event.parent
        if (parent == null) top(id) else under(id, parent)
    }

    private fun top(id: String) {
        if (id !in front) front += id
    }

    private fun under(
        id: String,
        parent: String,
    ) {
        edit.parents[id] = parent
        kids.getOrPut(parent) { mutableListOf() } += id
        if (parent !in edit.bodies && parent !in edit.order) top(parent)
    }

    fun done(): ChatLog {
        for ((parent, ids) in kids) edit.children[parent] = ids + edit.children[parent].orEmpty()
        edit.order.addAll(0, front)
        return edit.done()
    }
}

// what the list keeps of the events: no event its own subagent's
private fun kept(event: ChatEvent): Boolean = event.body.listed() && event.parent != event.id

// a page's events by id, in the order each id first came, each one's latest word folded over its earlier ones
private fun folded(events: List<ChatEvent>): Collection<ChatEvent> {
    val byId = LinkedHashMap<String, ChatEvent>()
    for (event in events) {
        val before = byId[event.id]
        byId[event.id] =
            if (before == null) {
                event
            } else {
                event.copy(
                    body = merge(older = before.body, newer = event.body),
                    parent = before.parent ?: event.parent,
                )
            }
    }
    return byId.values
}

// what the list shows: the rest is the view model's (the conversation, approvals, live, reset) or nobody's
private fun ChatBody.listed(): Boolean =
    when (this) {
        is ChatBody.User, is ChatBody.Assistant, is ChatBody.Thinking, is ChatBody.Tool, is ChatBody.Question,
        is ChatBody.Notice,
        -> true

        else -> false
    }

// the newer word on an event wins; a tool result read without its call keeps the call's name, kind and title, or,
// for a question's, stays the question with the result's status and output
private fun merge(
    older: ChatBody,
    newer: ChatBody,
): ChatBody {
    val result = (newer as? ChatBody.Tool)?.tool?.takeIf { it.name.isEmpty() } ?: return newer
    return when (older) {
        is ChatBody.Question -> {
            ChatBody.Question(older.question.copy(status = result.status, output = result.output))
        }

        is ChatBody.Tool -> {
            ChatBody.Tool(
                result.copy(
                    name = older.tool.name,
                    kind = older.tool.kind,
                    title = result.title.ifEmpty { older.tool.title },
                ),
            )
        }

        else -> {
            newer
        }
    }
}
