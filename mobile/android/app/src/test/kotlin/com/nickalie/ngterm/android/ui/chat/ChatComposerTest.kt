package com.nickalie.ngterm.android.ui.chat

import androidx.compose.runtime.saveable.SaverScope
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.TextFieldValue
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.core.fake.FakeAction
import com.nickalie.ngterm.core.ffi.AgentCommand
import com.nickalie.ngterm.core.ffi.AgentCommandKind
import com.nickalie.ngterm.core.ffi.AgentInputOutcome
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.RemoteStatus
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith

/** A chat's composer: the `/` menu over the agent's commands, sending the prompt, and Stop ([ChatComposer]). */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class ChatComposerTest {
    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var bed: ChatBed
    private lateinit var chat: ChatViewModel

    private val claude =
        listOf(
            command("clear", "Clear the conversation history and free up context"),
            command("compact", "Clear history but keep a summary in context", hint = "[instructions]"),
            command("context", "Show how the context is used"),
            command("review", "Review a pull request", AgentCommandKind.SKILL),
            command("frontend:design", "Design a screen", AgentCommandKind.PLUGIN),
        )

    @Before
    fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
        bed = ChatBed(dir.root)
        bed.attach("p1")
        bed.show("p1")
        bed.test.core.catalogs[bed.ref("p1")] = claude
        bed.agent("p1")
        eventually { bed.chat("p1") != null }
        chat = bed.chat("p1")!!
        bed.emit("p1", event("conv", ChatBody.Conversation("claude", "c1", "/home/user/project", null)), live)
    }

    @After
    fun tearDown() {
        bed.close()
        Dispatchers.resetMain()
    }

    private val composer get() = chat.composer
    private val pane get() = bed.panes.getValue("p1")

    private fun type(
        text: String,
        composition: TextRange? = null,
    ) = composer.edit(TextFieldValue(text, TextRange(text.length), composition))

    private fun names(): List<String>? = (composer.menu as? SlashMenu.Commands)?.commands?.map { it.name }

    // what saving the app's state keeps of [chat]'s draft
    private fun saver(chat: ChatViewModel) =
        keptDraftSaver(chat.pane, chat.composer) {
            chat.state.value.conversation
                ?.conversationId
        }

    private fun asked() =
        bed.test.core.calls
            .count { it.method == "agentCommands" }

    @Test
    fun aSlashAtTheStartOpensTheMenuWithTheWholeCatalog() {
        type("/")
        assertEquals(listOf("clear", "compact", "context", "frontend:design", "review"), names())
        assertEquals(0, (composer.menu as SlashMenu.Commands).highlighted)
        // a slash anywhere else is text
        for (text in listOf("a/", " /", "fix /tmp", "")) {
            type(text)
            assertNull(text, composer.menu)
        }
        type("/")
        assertEquals(1, asked())
    }

    @Test
    fun eachKeystrokeNarrowsItThroughFilterCommands() {
        // the menu is read as each keystroke is drawn
        type("/")
        assertEquals(5, names()?.size)
        type("/c")
        assertEquals(listOf("clear", "compact", "context"), names())
        type("/co")
        assertEquals(listOf("compact", "context"), names())
        // the IME's composition is typed text: Gboard holding "com" as a word still narrows
        type("/com", composition = TextRange(1, 4))
        assertEquals(listOf("compact"), names())
        assertEquals(listOf("", "c", "co", "com"), bed.test.filtered.toList())
        // a cursor moving inside the query ranks nothing again
        composer.edit(TextFieldValue("/com", TextRange(2)))
        assertEquals(4, bed.test.filtered.size)
        type("/xyz")
        assertEquals(emptyList<String>(), names())
        assertEquals(1, asked())
    }

    @Test
    fun aSpaceALineBreakOrDeletingTheSlashClosesIt() {
        type("/comp")
        assertTrue(composer.menu != null)
        type("/comp ")
        assertNull(composer.menu)
        type("/comp\n")
        assertNull(composer.menu)
        type("comp")
        assertNull(composer.menu)
        // Escape closes it until the text changes
        type("/c")
        assertTrue(composer.dismissMenu())
        assertNull(composer.menu)
        assertFalse(composer.dismissMenu())
        composer.edit(TextFieldValue("/c", TextRange(0)))
        assertNull(composer.menu)
        type("/co")
        assertEquals(listOf("compact", "context"), names())
    }

    @Test
    fun pickingACommandPutsItInTheFieldWithASpace() {
        type("/co")
        assertTrue(composer.moveHighlight(1))
        assertTrue(composer.moveHighlight(5))
        assertEquals(1, (composer.menu as SlashMenu.Commands).highlighted)
        assertTrue(composer.pickHighlighted())
        assertEquals(TextFieldValue("/context ", TextRange(9)), composer.draft)
        assertNull(composer.menu)
        assertFalse(composer.pickHighlighted())

        // a Codex skill goes in as it stands
        composer.pick(command("\$review", "Review a change", AgentCommandKind.SKILL))
        assertEquals(TextFieldValue("\$review ", TextRange(8)), composer.draft)
        assertNull(composer.menu)
        // nothing matching picks nothing
        type("/xyz")
        assertFalse(composer.pickHighlighted())
        assertFalse(composer.moveHighlight(1))
        assertEquals("/xyz", composer.draft.text)
    }

    @Test
    fun anEmptyCatalogShowsAQuietRowNeverAnError() {
        // the fetch still running
        val gate = CompletableDeferred<Unit>()
        bed.test.core.holds["agentCommands"] = gate
        type("/")
        assertEquals(SlashMenu.Waiting("", fetching = true), composer.menu)
        // an older origin answers with no catalog: a quiet row, and no problem for the chat
        bed.test.core.catalogs
            .remove(bed.ref("p1"))
        gate.complete(Unit)
        eventually { composer.catalog == CommandCatalog.Failed }
        assertEquals(SlashMenu.Waiting("", fetching = false), composer.menu)
        assertNull(chat.state.value.problem)
        // typing on doesn't ask again; opening the menu again does
        type("/c")
        assertEquals(1, asked())
        type("")
        bed.test.core.catalogs[bed.ref("p1")] = emptyList()
        type("/")
        assertEquals(2, asked())
        // an agent with no commands at all reads the same
        assertEquals(SlashMenu.Waiting("", fetching = false), composer.menu)
    }

    @Test
    fun aCatalogArrivingAfterTheMenuOpenedListsWhatIsTypedByThen() {
        val gate = CompletableDeferred<Unit>()
        bed.test.core.holds["agentCommands"] = gate
        type("/")
        type("/co")
        assertEquals(SlashMenu.Waiting("co", fetching = true), composer.menu)
        gate.complete(Unit)
        eventually { names() != null }
        assertEquals(listOf("compact", "context"), names())
        assertEquals(1, asked())
    }

    @Test
    fun aNewConversationAsksForItsCatalogAndDropsTheOldOnesAnswer() {
        type("/")
        assertEquals(5, names()?.size)
        // a reset while the menu is open asks again at once
        bed.test.core.catalogs[bed.ref("p1")] = claude.take(2)
        bed.emit("p1", reset)
        assertEquals(listOf("clear", "compact"), names())
        assertEquals(2, asked())

        // an answer asked for before the next reset is dropped: the new conversation's goes in
        val gate = CompletableDeferred<Unit>()
        bed.test.core.holds["agentCommands"] = gate
        bed.emit("p1", reset)
        assertEquals(SlashMenu.Waiting("", fetching = true), composer.menu)
        bed.test.core.catalogs[bed.ref("p1")] = claude.takeLast(1)
        bed.emit("p1", reset)
        assertEquals(listOf("frontend:design"), names())
        // the held ask would answer with another list: were it let in, the menu would show it
        bed.test.core.catalogs[bed.ref("p1")] = claude.take(1)
        gate.complete(Unit)
        assertEquals(listOf("frontend:design"), names())

        // with the menu closed a reset only forgets the catalog, and the next menu asks
        type("fix it")
        bed.emit("p1", reset)
        assertEquals(CommandCatalog.Unasked, composer.catalog)
        assertEquals(4, asked())
        type("/")
        assertEquals(5, asked())
    }

    @Test
    fun sendTakesTheDraftAndLetsItGoOnceSent() {
        type("  Fix the install guide.\n")
        chat.send()
        assertEquals(listOf(FakeAction.Prompt("Fix the install guide.")), pane.actions.toList())
        assertEquals("", composer.draft.text)
        // not sent: the draft stays
        pane.outcome = AgentInputOutcome.DIALOG_OPEN
        type("/compact keep the plan")
        chat.send()
        assertEquals("/compact keep the plan", composer.draft.text)
        assertEquals(ChatProblem.DialogOpen, chat.state.value.problem)
        // nothing to send sends nothing
        type(" \n ")
        chat.send()
        assertEquals(2, pane.actions.size)
    }

    @Test
    fun aSecondTapWhileTheFirstIsOnItsWayDoesNothing() {
        val gate = CompletableDeferred<Unit>()
        pane.gate = gate
        type("run the tests")
        chat.send()
        assertTrue(chat.state.value.busy)
        chat.send()
        chat.stop()
        assertEquals(listOf(FakeAction.Prompt("run the tests")), pane.actions.toList())
        assertEquals("run the tests", composer.draft.text)
        gate.complete(Unit)
        eventually { !chat.state.value.busy }
        assertEquals("", composer.draft.text)
        assertEquals(1, pane.actions.size)
    }

    @Test
    fun aCardHoldsSendAndStop() {
        bed.emit("p1", event("ap", ChatBody.Approval(approval)))
        assertTrue(chat.state.value.awaitsCard)
        type("and then deploy")
        chat.send()
        chat.stop()
        assertTrue(pane.actions.isEmpty())
        assertEquals("and then deploy", composer.draft.text)
        bed.emit("p1", event("ap", ChatBody.ApprovalDone(approval)))
        assertFalse(chat.state.value.awaitsCard)
        chat.send()
        assertEquals(listOf(FakeAction.Prompt("and then deploy")), pane.actions.toList())
    }

    @Test
    fun workingFollowsTheAgentsStatusForThisPane() {
        fun status(
            name: String,
            pane: String?,
        ) = bed.test.core.emit(
            MobileEvent.SessionStatus("m1", "s1", RemoteStatus(name, false, null, null, pane, null), true),
        )
        assertFalse(chat.state.value.working)
        status("active", "p1")
        eventually { chat.state.value.working }
        status("active", "p2")
        eventually { !chat.state.value.working }
        status("active", null)
        eventually { chat.state.value.working }
        // a reset keeps it: the agent's status says it
        bed.emit("p1", reset)
        assertTrue(chat.state.value.working)
        status("blocked", "p1")
        eventually { !chat.state.value.working }
    }

    @Test
    fun aKeptDraftComesBackOnlyIntoItsConversationAndAnEmptyField() {
        chat.restoreDraft("c0", "for another conversation")
        assertEquals("", composer.draft.text)
        chat.restoreDraft("c1", "Fix the guide")
        assertEquals(TextFieldValue("Fix the guide", TextRange(13)), composer.draft)
        chat.restoreDraft("c1", "an older copy")
        assertEquals("Fix the guide", composer.draft.text)
        // asked before the conversation is named, it waits for the name
        bed.emit("p1", reset)
        type("")
        chat.restoreDraft("c2", "the new one's")
        assertEquals("", composer.draft.text)
        bed.emit("p1", event("conv2", ChatBody.Conversation("claude", "c2", null, null)))
        assertEquals("the new one's", composer.draft.text)
    }

    @Test
    fun aDraftKeptWhileTheAppIsAwayGoesBackOnlyToItsPaneAndConversation() {
        val scope = SaverScope { true }
        type("Fix the install guide")
        val saved = with(saver(chat)) { scope.save(KeptDraft(null, "")) }!!
        // the app comes back without its process: the pane's chat is a new one, empty
        bed.test.chats.pick(bed.ref("p1"), PaneView.Terminal)
        eventually { bed.chat("p1") == null }
        bed.test.chats.pick(bed.ref("p1"), PaneView.Chat)
        eventually { bed.chat("p1") != null }
        val again = bed.chat("p1")!!
        assertEquals("", again.composer.draft.text)
        val kept = saver(again).restore(saved)!!
        again.restoreDraft(kept.conversation!!, kept.text)
        assertEquals("", again.composer.draft.text)
        bed.emit("p1", event("conv", ChatBody.Conversation("claude", "c1", null, null)))
        assertEquals("Fix the install guide", again.composer.draft.text)

        // saved for another pane: let go
        assertNull(saver(again).restore(arrayListOf("m1", "s1", "p2", "c1", "Fix")))
        assertNull(saver(again).restore(arrayListOf("m1", "s1")))

        // nothing to keep: a blank draft, one too long for saved state, or one before its conversation is named
        fun keeps(chat: ChatViewModel) = with(saver(chat)) { scope.save(KeptDraft(null, "")) } != null
        again.composer.edit(TextFieldValue(" "))
        assertFalse(keeps(again))
        again.composer.edit(TextFieldValue("x".repeat(KEPT_DRAFT_MAX + 1)))
        assertFalse(keeps(again))
        again.composer.edit(TextFieldValue("x"))
        assertTrue(keeps(again))
        bed.emit("p1", reset)
        assertFalse(keeps(again))
    }

    @Test
    fun theFieldTakesLineBreaksAsTypedAndRefusesOtherControls() {
        type("one\r\ntwo\rthree")
        assertEquals(TextFieldValue("one\ntwo\nthree", TextRange(13)), composer.draft)
        // a word still being composed keeps its composition, on the same characters
        type("one\r\ntwo\rthree", composition = TextRange(9, 14))
        assertEquals(TextFieldValue("one\ntwo\nthree", TextRange(13), TextRange(8, 13)), composer.draft)
        type("one\ntwo\u001b[201~")
        assertEquals("one\ntwo\nthree", composer.draft.text)
        type("tab\there")
        assertEquals("tab\there", composer.draft.text)
    }
}

private fun command(
    name: String,
    description: String,
    kind: AgentCommandKind = AgentCommandKind.BUILTIN,
    hint: String? = null,
) = AgentCommand(name, description, kind, hint)
