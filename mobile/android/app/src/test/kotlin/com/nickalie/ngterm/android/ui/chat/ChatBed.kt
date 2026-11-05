package com.nickalie.ngterm.android.ui.chat

import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.ChatApproval
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.ChatEvent
import com.nickalie.ngterm.core.ffi.ChatQuestion
import com.nickalie.ngterm.core.ffi.ChatQuestionItem
import com.nickalie.ngterm.core.ffi.ChatQuestionOption
import com.nickalie.ngterm.core.ffi.ChatTool
import com.nickalie.ngterm.core.ffi.ChatToolKind
import com.nickalie.ngterm.core.ffi.ChatToolStatus
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneSize
import com.nickalie.ngterm.core.ffi.RemoteLayout
import kotlinx.coroutines.runBlocking
import java.io.File

/**
 * A session `s1` on machine `m1` over a [TestGraph], for the chat's tests: [attach] connects and attaches it with its
 * panes, [show] shows one (a [FakePane] each, in [panes]), [agent] announces what a pane runs as its origin would,
 * and [emit] streams a pane's chat events. The test sets the main dispatcher first.
 */
class ChatBed(
    dir: File,
) {
    val test = TestGraph(dir)
    val key = SessionKey("m1", "s1")

    /** The panes shown, by id. */
    val panes: MutableMap<String, FakePane> = mutableMapOf()

    /** Connects m1 with s1 listed, attaches s1, and lays it out as [ids] (the first primary). */
    fun attach(vararg ids: String) {
        val stored = testMachine(keyId = test.seedKey().id)
        test.seedMachine(stored)
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", paneCount = ids.size.toUInt()))
        test.core.paneFactory = { _, _, id, _ ->
            // a pane with frames to read takes input; the chat's actions reach it
            FakePane(id, frames = { ByteArray(0) }).also { panes[id] = it }
        }
        test.connections.connect(stored.toCore())
        eventually { test.connections.trees.value["m1"] != null }
        runBlocking { test.connections.attach("m1", "s1") }
        test.core.emit(MobileEvent.SessionLayout("m1", "s1", RemoteLayout(ids.toList(), ids.first(), "vertical", true)))
    }

    fun ref(id: String) = PaneRef("m1", "s1", id)

    /** Shows pane [id], as its view does once it has a size. */
    fun show(id: String) {
        test.panes.show(ref(id), PaneSize(80u, 24u, 10u, 20u))
        eventually { ref(id) in test.panes.shown.value }
    }

    /** The origin announces that pane [id] runs [agent] in [conversation] (`null` agent: none). */
    fun agent(
        id: String,
        agent: String? = "claude",
        conversation: String? = "c1",
    ) = test.core.emit(MobileEvent.PaneAgent("m1", "s1", id, agent, conversation?.takeIf { agent != null }))

    /** Pane [id]'s open chat, or `null`. */
    fun chat(id: String): ChatViewModel? = test.chats.chats.value[ref(id)]

    /** Pane [id]'s view, or `null` while it shows its terminal with no switch. */
    fun view(id: String): PaneView? = test.chats.views.value[ref(id)]

    /** Streams [events] as one batch of pane [id]'s chat. */
    fun emit(
        id: String,
        vararg events: ChatEvent,
    ) = test.core.emit(MobileEvent.Chat(ref(id), events.toList()))

    fun close() = test.close()
}

fun eventually(check: () -> Boolean) {
    val deadline = System.currentTimeMillis() + 10_000
    while (!check()) {
        check(System.currentTimeMillis() < deadline) { "timed out" }
        Thread.sleep(10)
    }
}

/** A chat event: [body] as event [id], from a subagent's transcript when it has a [parent]. */
fun event(
    id: String,
    body: ChatBody,
    parent: String? = null,
) = ChatEvent(id, 0u, parent, body)

fun user(text: String): ChatBody = ChatBody.User(text, 0u)

fun reply(text: String): ChatBody = ChatBody.Assistant(text)

fun tool(
    name: String,
    title: String,
    status: ChatToolStatus = ChatToolStatus.RUNNING,
    kind: ChatToolKind = ChatToolKind.SHELL,
    output: String? = null,
): ChatBody = ChatBody.Tool(ChatTool(name, kind, title, status, output, null, false))

val live = event("live", ChatBody.Live)

val reset = event("reset", ChatBody.Reset)

val approval = ChatApproval("Bash", "rm -rf build", "Remove the build directory", "toolu_1", null)

/** A one-question `AskUserQuestion` in [status]. */
fun question(status: ChatToolStatus = ChatToolStatus.RUNNING): ChatBody =
    ChatBody.Question(
        ChatQuestion(
            listOf(
                ChatQuestionItem(
                    "Which database?",
                    "Database",
                    false,
                    listOf(ChatQuestionOption("Postgres", null), ChatQuestionOption("SQLite", "One file")),
                ),
            ),
            status,
            if (status == ChatToolStatus.RUNNING) null else "Postgres",
        ),
    )
