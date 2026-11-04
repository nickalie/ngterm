package com.nickalie.ngterm.android.ui.chat

import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatEvent
import com.nickalie.ngterm.core.ffi.ChatQuestion
import com.nickalie.ngterm.core.ffi.ChatToolStatus

/** One row of a pane's chat, as the list shows it. Its [key] stays the same through updates and older pages. */
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
        override val key: String get() = "e:$id"
    }

    /**
     * A subagent's events whose task is not in the chat (its call is older than what was read), under the task's
     * id ([parent]) until an older page brings the task, which then takes them.
     */
    data class Subagent(
        val parent: String,
        val children: List<ChatItem>,
    ) : ChatItem {
        override val key: String get() = "s:$parent"
    }

    /** The pane's conversation switched here: what was shown before it is gone, and what follows is the new one. */
    data object Divider : ChatItem {
        override val key: String get() = "divider"
    }
}

/** An `AskUserQuestion` still waiting for its answer: the chat event's [id] and the [question]. */
data class OpenQuestion(
    val id: String,
    val question: ChatQuestion,
)

/**
 * A pane's chat folded into its rows ([items]), immutable: [append] takes the events the core streams, [prepend] an
 * older page, and [reset] starts over after the conversation switched ([switched] puts a [ChatItem.Divider] on top).
 *
 * - Events upsert by id: a later event with an id already shown updates its row in place. A tool result whose call
 *   was outside the origin's window comes with no name or title, and keeps the ones its call gave; an older page
 *   never undoes what a newer event said.
 * - An event with a `parent` (a subagent's) folds under the row with that id (its task tool), in the order read;
 *   while that row isn't here, under a [ChatItem.Subagent] in its place.
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

    /** The log with [events] (newest last) taken in after what it shows. */
    fun append(events: List<ChatEvent>): ChatLog {
        val edit = Edit(this)
        for (event in events.filter { it.body.listed() && it.parent != it.id }) {
            val known = edit.bodies[event.id]
            when {
                known != null -> edit.bodies[event.id] = merge(older = known, newer = event.body)
                event.id in edit.order -> edit.bodies[event.id] = event.body
                else -> edit.add(event)
            }
        }
        return edit.done()
    }

    /** The log with an older page ([events], oldest first) put before what it shows. */
    fun prepend(events: List<ChatEvent>): ChatLog {
        val page = Page(Edit(this))
        for (event in events.filter { it.body.listed() && it.parent != it.id }) page.take(event)
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

        fun add(event: ChatEvent) {
            val id = event.id
            bodies[id] = event.body
            val parent = event.parent
            if (parent == null) {
                order += id
                return
            }
            parents[id] = parent
            children[parent] = children[parent].orEmpty() + id
            // a parent neither shown nor a subagent's own event stands in for its task
            if (parent !in bodies && parent !in order && parent !in parents) order += parent
        }

        fun done() = ChatLog(order.toList(), bodies.toMap(), children.toMap(), parents.toMap(), switched)
    }
}

// an older page going in: the rows it puts on top ([front]) and under each parent ([kids]), oldest first
private class Page(
    private val edit: ChatLog.Edit,
) {
    private val front = mutableListOf<String>()
    private val kids = mutableMapOf<String, MutableList<String>>()

    fun take(event: ChatEvent) {
        val id = event.id
        val known = edit.bodies[id]
        when {
            known != null -> {
                edit.bodies[id] = merge(older = event.body, newer = known)
            }

            // a task its subagent's events stood in for: it takes them, at its own place
            id in edit.order -> {
                edit.bodies[id] = event.body
                edit.order.remove(id)
                front += id
            }

            else -> {
                edit.bodies[id] = event.body
                val parent = event.parent
                if (parent == null) top(id) else under(id, parent)
            }
        }
    }

    // its subagent's events, read first, may have put it on top already
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

// what the list shows: the rest is the view model's (the conversation, approvals, live, reset) or nobody's
private fun ChatBody.listed(): Boolean =
    when (this) {
        is ChatBody.User, is ChatBody.Assistant, is ChatBody.Thinking, is ChatBody.Tool, is ChatBody.Question,
        is ChatBody.Notice,
        -> true

        else -> false
    }

// the newer word on an event wins; a tool result read without its call keeps the call's name, kind and title
private fun merge(
    older: ChatBody,
    newer: ChatBody,
): ChatBody {
    if (older !is ChatBody.Tool || newer !is ChatBody.Tool || newer.tool.name.isNotEmpty()) return newer
    val call = older.tool
    return ChatBody.Tool(
        newer.tool.copy(
            name = call.name,
            kind = call.kind,
            title = newer.tool.title.ifEmpty { call.title },
        ),
    )
}
