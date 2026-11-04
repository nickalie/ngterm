package com.nickalie.ngterm.android.ui.chat

import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatToolKind
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** How [ChatLog] merges live events, older pages, placeholders and repeated ids, and the rows' keys through it all. */
class ChatLogTest {
    // the rows as (id or placeholder, children), with each row's key checked against its event's id
    private fun ChatLog.rows(items: List<ChatItem> = this.items): List<String> =
        items.map { item ->
            when (item) {
                is ChatItem.Entry -> {
                    assertEquals("e:${item.id}", item.key)
                    if (item.children.isEmpty()) item.id else "${item.id}${rows(item.children)}"
                }

                is ChatItem.Subagent -> {
                    assertEquals("e:${item.parent}", item.key)
                    "?${item.parent}${rows(item.children)}"
                }

                ChatItem.Divider -> {
                    "--"
                }
            }
        }

    private fun ChatLog.body(id: String): ChatBody =
        flat(items).filterIsInstance<ChatItem.Entry>().single { it.id == id }.body

    private fun flat(items: List<ChatItem>): List<ChatItem> =
        items.flatMap { item ->
            when (item) {
                is ChatItem.Entry -> listOf(item) + flat(item.children)
                is ChatItem.Subagent -> listOf(item) + flat(item.children)
                ChatItem.Divider -> listOf(item)
            }
        }

    private fun ChatBody.tool() = (this as ChatBody.Tool).tool

    private val task = tool("Agent", "Find the bug", kind = ChatToolKind.TASK)

    @Test
    fun liveEventsUpdateTheirRowsNewestLast() {
        val log =
            ChatLog()
                .append(listOf(event("a1", reply("Let")), event("a1", reply("Let me")), event("u1", user("hi"))))
                .append(listOf(event("a1", reply("Let me look."))))
        assertEquals(listOf("a1", "u1"), log.rows())
        assertEquals(reply("Let me look."), log.body("a1"))
    }

    @Test
    fun aTaskStreamedAfterItsSubagentsEventsTakesItsPlaceholderInPlace() {
        val before =
            ChatLog().append(
                listOf(
                    event("c1", reply("Found it"), parent = "task"),
                    event("a1", reply("Waiting")),
                    // a subagent whose task is itself one of the first subagent's events, not read yet
                    event("n1", reply("Nested"), parent = "inner"),
                ),
            )
        assertEquals(listOf("?task[c1]", "a1", "?inner[n1]"), before.rows())
        val keys = before.items.map { it.key }

        val after = before.append(listOf(event("task", task), event("inner", task, parent = "task")))
        assertEquals(listOf("task[c1, inner[n1]]", "a1"), after.rows())
        // the task's row kept its key and its place; the nested one went under its parent with its key
        assertEquals(keys.take(2), after.items.map { it.key })
        assertEquals("e:inner", ((after.items.first() as ChatItem.Entry).children[1]).key)
    }

    @Test
    fun anOlderPageIsFoldedOldestToNewestBeforeItGoesIn() {
        val log =
            ChatLog()
                .append(listOf(event("u5", user("five"))))
                .prepend(
                    listOf(
                        event("t1", tool("Bash", "make")),
                        event("q1", question()),
                        event("t1", tool("", "", ChatToolStatus.OK, ChatToolKind.OTHER, output = "built")),
                        event("q1", question(ChatToolStatus.OK)),
                    ),
                )
        assertEquals(listOf("t1", "q1", "u5"), log.rows())
        val result = log.body("t1").tool()
        assertEquals(ChatToolStatus.OK, result.status)
        assertEquals("built", result.output)
        assertEquals("Bash", result.name)
        assertEquals(ChatToolKind.SHELL, result.kind)
        assertEquals("make", result.title)
        // the question the page saw answered brings no card back
        assertNull(log.openQuestion(emptySet()))
    }

    @Test
    fun anIdOnBothSidesOfThePageKeepsItsPlaceAndTheLogsNewerWord() {
        val log =
            ChatLog().append(
                listOf(
                    event("t1", tool("", "", ChatToolStatus.OK, ChatToolKind.OTHER, output = "built")),
                    event("task", task),
                    event("c1", tool("Read", "a.rs", ChatToolStatus.OK, ChatToolKind.READ), parent = "task"),
                    event("u2", user("next")),
                ),
            )
        val page =
            log.prepend(
                listOf(
                    event("u0", user("start")),
                    event("t1", tool("Bash", "make")),
                    event("c0", reply("Reading"), parent = "task"),
                    event("c1", tool("Read", "a.rs", kind = ChatToolKind.READ), parent = "task"),
                    event("u2", user("next")),
                ),
            )
        assertEquals(listOf("u0", "t1", "task[c0, c1]", "u2"), page.rows())
        assertEquals(ChatToolStatus.OK, page.body("c1").tool().status)
        val result = page.body("t1").tool()
        assertEquals(ChatToolStatus.OK, result.status)
        assertEquals("Bash", result.name)
        assertEquals("make", result.title)
    }

    @Test
    fun aPlaceholderAnOlderPageBringsTheTaskOfMovesToTheTaskWithItsKey() {
        val log =
            ChatLog().append(
                listOf(event("c2", reply("Found it"), parent = "task"), event("a1", reply("Waiting"))),
            )
        val page =
            log.prepend(
                listOf(
                    event("u0", user("start")),
                    event("task", task),
                    event("c1", reply("Looking"), parent = "task"),
                    event("u1", user("meanwhile")),
                ),
            )
        assertEquals(listOf("u0", "task[c1, c2]", "u1", "a1"), page.rows())
    }

    @Test
    fun aPagesSubagentEventsReadBeforeTheirTaskGoUnderItAtItsPlace() {
        val log =
            ChatLog()
                .append(listOf(event("a1", reply("Done"))))
                .prepend(
                    listOf(
                        event("c1", reply("Looking"), parent = "task"),
                        event("u1", user("go")),
                        event("task", task),
                    ),
                )
        assertEquals(listOf("u1", "task[c1]", "a1"), log.rows())
    }

    @Test
    fun aResetLeavesTheDividerAndThePagesOfTheNewConversationUnderIt() {
        val log =
            ChatLog()
                .append(listOf(event("u1", user("old"))))
                .reset()
                .append(listOf(event("v1", user("new"))))
                .prepend(listOf(event("v0", user("before"))))
        assertEquals(listOf("--", "v0", "v1"), log.rows())
        assertEquals("divider", log.items.first().key)
    }
}
