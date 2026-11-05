package com.nickalie.ngterm.android.ui.chat

import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.session.OpenSession
import com.nickalie.ngterm.android.ui.session.SessionViewModel
import com.nickalie.ngterm.core.fake.FakeCall
import com.nickalie.ngterm.core.ffi.ChatState
import com.nickalie.ngterm.core.ffi.MachineState
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.MobileException
import com.nickalie.ngterm.core.ffi.RemoteLayout
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.test.UnconfinedTestDispatcher
import kotlinx.coroutines.test.resetMain
import kotlinx.coroutines.test.setMain
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith

/** Which view a pane running an agent shows, and the chats that follow from it ([PaneChats], [SessionViewModel]). */
@OptIn(ExperimentalCoroutinesApi::class)
@RunWith(AndroidJUnit4::class)
class ChatModeTest {
    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var bed: ChatBed
    private val models = ViewModelStore()

    @Before
    fun setUp() {
        Dispatchers.setMain(UnconfinedTestDispatcher())
        bed = ChatBed(dir.root)
    }

    @After
    fun tearDown() {
        models.clear()
        bed.close()
        Dispatchers.resetMain()
    }

    private fun model(): SessionViewModel =
        ViewModelProvider(models, viewModelFactory { initializer { SessionViewModel(bed.test.graph) } })[
            SessionViewModel::class.java,
        ]

    private fun openChats() = bed.test.core.openChats

    private fun closes(id: String) =
        bed.test.core.calls
            .count { it == FakeCall("closeChat", listOf("m1", "s1", id)) }

    private fun setChatView(on: Boolean) = runBlocking { bed.test.settings.update { it.copy(agentChat = on) } }

    @Test
    fun aShownPaneWhoseOriginAnnouncesAnAgentSwitchesToItsChat() {
        bed.attach("p1", "p2")
        val model = model()
        model.open(bed.key)
        bed.show("p1")
        assertTrue(openChats().isEmpty())
        bed.agent("p1")
        eventually { (model.state.value.open as? OpenSession.Live)?.views == mapOf("p1" to PaneView.Chat) }
        assertEquals(setOf(bed.ref("p1")), openChats())
        assertEquals(bed.ref("p1"), bed.chat("p1")?.pane)
        // the chat reads; it claims nothing, and the pane's own attach stays
        assertTrue(
            bed.test.core.calls
                .none { it.method == "takeLead" || it.method == "hidePane" },
        )
        assertTrue(bed.ref("p1") in bed.test.panes.shown.value)
    }

    @Test
    fun aPaneOffScreenGetsItsViewButNoChatUntilItIsShown() {
        bed.attach("p1", "p2")
        bed.show("p1")
        bed.agent("p2", conversation = "c9")
        eventually { bed.view("p2") == PaneView.Chat }
        assertTrue(openChats().isEmpty())
        bed.show("p2")
        eventually { openChats() == setOf(bed.ref("p2")) }
        // hidden (its session's layout dropped it), it closes
        bed.test.core.emit(MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("p1"), "p1", "vertical", true)))
        eventually { openChats().isEmpty() && bed.chat("p2") == null }
    }

    @Test
    fun withTheSettingOffEveryPaneShowsItsTerminal() {
        setChatView(false)
        bed.attach("p1")
        bed.show("p1")
        bed.agent("p1")
        eventually {
            bed.test.live.sessions.value[bed.key]
                ?.agents
                ?.isNotEmpty() == true
        }
        assertNull(bed.view("p1"))
        assertTrue(openChats().isEmpty())

        setChatView(true)
        eventually { bed.view("p1") == PaneView.Chat && openChats() == setOf(bed.ref("p1")) }
        val closed = closes("p1")

        setChatView(false)
        eventually { bed.view("p1") == null && openChats().isEmpty() }
        assertEquals(closed + 1, closes("p1"))
    }

    @Test
    fun aSessionsOwnChatViewOverridesTheSettingUntilSetBackToIt() {
        bed.attach("p1")
        val model = model()
        bed.show("p1")
        bed.agent("p1")
        eventually { bed.view("p1") == PaneView.Chat && openChats() == setOf(bed.ref("p1")) }
        assertEquals(
            true,
            model.state.value.sessions
                .single()
                .chatView,
        )

        // the session's menu turns it off: its panes show their terminal, the setting still on
        model.setChatView(bed.key, false)
        eventually { bed.view("p1") == null && openChats().isEmpty() }
        assertEquals(
            false,
            model.state.value.sessions
                .single()
                .chatView,
        )

        // the setting off and back on leaves the session's own choice alone
        setChatView(false)
        setChatView(true)
        eventually { bed.test.chats.chatViews.value.default }
        assertNull(bed.view("p1"))

        // on again equals the default: the choice is dropped, and the session follows the setting from then on
        model.setChatView(bed.key, true)
        eventually { bed.view("p1") == PaneView.Chat }
        eventually {
            runBlocking {
                bed.test.sessionViews.sessions
                    .first()
            }.isEmpty()
        }
        setChatView(false)
        eventually { bed.view("p1") == null }

        // with the setting off, a session's menu turns the chat on for it alone
        model.setChatView(bed.key, true)
        eventually { bed.view("p1") == PaneView.Chat && openChats() == setOf(bed.ref("p1")) }
        // the views follow the tap at once; the store's write lands after
        eventually {
            runBlocking {
                bed.test.sessionViews.sessions
                    .first()
            } == mapOf(bed.key to true)
        }
    }

    @Test
    fun aChoiceTheStoreCannotKeepHoldsThroughSettingsChanges() {
        bed.attach("p1")
        val model = model()
        bed.show("p1")
        bed.agent("p1")
        eventually { bed.view("p1") == PaneView.Chat }
        // the store can't be written: the choice shows at once and is kept in memory
        val views = bed.test.sessionViewsDir
        views.setWritable(false)
        try {
            model.setChatView(bed.key, false)
            eventually { bed.view("p1") == null }
            // the write failed by now; Settings' default moving re-reads the store, and the choice still holds
            Thread.sleep(300)
            setChatView(false)
            setChatView(true)
            eventually { bed.test.chats.chatViews.value.default }
            assertNull(bed.view("p1"))
            assertTrue(
                runBlocking {
                    bed.test.sessionViews.sessions
                        .first()
                }.isEmpty(),
            )
        } finally {
            views.setWritable(true)
        }
        // once the store takes writes again, setting it back lands there and follows the default
        model.setChatView(bed.key, true)
        eventually { bed.view("p1") == PaneView.Chat }
    }

    @Test
    fun pickingTheTerminalHoldsUntilThePanesNextConversation() {
        bed.attach("p1")
        val model = model()
        model.open(bed.key)
        bed.show("p1")
        bed.agent("p1", conversation = "c1")
        eventually { bed.view("p1") == PaneView.Chat }

        model.pickView(bed.ref("p1"), PaneView.Terminal)
        eventually { (model.state.value.open as? OpenSession.Live)?.views == mapOf("p1" to PaneView.Terminal) }
        assertTrue(openChats().isEmpty())
        assertEquals(1, closes("p1"))
        // the origin asserts the same conversation again (a prompt, a restart): the terminal holds
        bed.agent("p1", conversation = "c1")
        assertEquals(PaneView.Terminal, bed.view("p1"))
        // the chat picked by hand comes back, and the terminal again
        model.pickView(bed.ref("p1"), PaneView.Chat)
        eventually { bed.view("p1") == PaneView.Chat && openChats().isNotEmpty() }
        model.pickView(bed.ref("p1"), PaneView.Terminal)
        eventually { bed.view("p1") == PaneView.Terminal && openChats().isEmpty() }

        // the next conversation starts in its chat
        bed.agent("p1", conversation = "c2")
        eventually { bed.view("p1") == PaneView.Chat && openChats() == setOf(bed.ref("p1")) }
    }

    @Test
    fun theAgentEndingReturnsThePaneToItsTerminal() {
        bed.attach("p1")
        bed.show("p1")
        bed.agent("p1", conversation = "c1")
        eventually { bed.chat("p1") != null }

        bed.agent("p1", agent = null)
        eventually { bed.view("p1") == null && openChats().isEmpty() && bed.chat("p1") == null }

        // the next agent opens in its chat
        bed.agent("p1", conversation = "c2")
        eventually { bed.view("p1") == PaneView.Chat && openChats().isNotEmpty() }

        // a chat the origin ends while it still announces the agent leaves the terminal for that conversation
        bed.test.core.emit(MobileEvent.ChatState(bed.ref("p1"), ChatState.ENDED))
        eventually { bed.view("p1") == PaneView.Terminal && bed.chat("p1") == null }
        bed.agent("p1", conversation = "c2")
        assertEquals(PaneView.Terminal, bed.view("p1"))
        bed.agent("p1", conversation = "c3")
        eventually { bed.view("p1") == PaneView.Chat && bed.chat("p1") != null }
    }

    @Test
    fun aChatEndingAfterThePaneMovedOnOpensTheNextConversationsChat() {
        bed.attach("p1")
        bed.show("p1")
        bed.agent("p1", conversation = "c1")
        eventually { bed.chat("p1") != null }
        val first = bed.chat("p1")
        val closed = closes("p1")

        // /clear: the next conversation is announced before the old one's chat reports its end
        bed.agent("p1", conversation = "c2")
        bed.test.core.emit(MobileEvent.ChatState(bed.ref("p1"), ChatState.ENDED))
        eventually { bed.chat("p1") != null && bed.chat("p1") !== first }
        // no terminal held for the new conversation: the pane shows its chat, opened afresh
        assertEquals(PaneView.Chat, bed.view("p1"))
        assertEquals(closed + 1, closes("p1"))
        assertEquals(setOf(bed.ref("p1")), openChats())

        // the new chat ending in the conversation it followed holds the terminal for it, as before
        bed.test.core.emit(MobileEvent.ChatState(bed.ref("p1"), ChatState.ENDED))
        eventually { bed.view("p1") == PaneView.Terminal && bed.chat("p1") == null }
    }

    @Test
    fun aChatTheOriginCantOpenLeavesTheTerminalWithNoError() {
        bed.attach("p1")
        bed.show("p1")
        bed.test.core.failNext("openChat", MobileException.Remote("unknown command agent"))
        bed.agent("p1")
        eventually { bed.view("p1") == PaneView.Terminal && bed.chat("p1") == null }
        assertTrue(openChats().isEmpty())
    }

    @Test
    fun aChatAskedForWhileTheLinkIsDownOpensOnceItIsBack() {
        bed.attach("p1")
        bed.show("p1")
        bed.test.core.failNext("openChat", MobileException.NotConnected("m1"))
        bed.agent("p1")
        eventually {
            bed
                .chat("p1")
                ?.state
                ?.value
                ?.phase == ChatPhase.Reconnecting
        }
        // no hold: the pane stays in chat view, its chat waiting for the link
        assertEquals(PaneView.Chat, bed.view("p1"))
        assertTrue(openChats().isEmpty())

        bed.test.core.emit(MobileEvent.MachineState("m1", MachineState.Connected))
        eventually { openChats() == setOf(bed.ref("p1")) }
        assertEquals(PaneView.Chat, bed.view("p1"))
        assertEquals(
            ChatPhase.Loading,
            bed
                .chat("p1")
                ?.state
                ?.value
                ?.phase,
        )
    }

    @Test
    fun aNotificationsTapBringsThePanesOpenChatToItsEndAndNoOtherPanes() {
        bed.attach("p1", "p2")
        val model = model()
        model.open(bed.key)
        bed.show("p1")
        bed.agent("p1")
        eventually { bed.chat("p1") != null }
        val chat = bed.chat("p1")!!
        // a pane with no chat open (no agent, or not shown) has nothing to move
        model.reveal(bed.ref("p2"))
        model.reveal(bed.ref("gone"))
        assertEquals(false, chat.state.value.toEnd)

        model.reveal(bed.ref("p1"))
        assertTrue(chat.state.value.toEnd)
        // a new conversation keeps the ask: the list hasn't gone there yet
        bed.emit("p1", reset)
        eventually { chat.state.value.log.switched }
        assertTrue(chat.state.value.toEnd)
        chat.endShown()
        assertEquals(false, chat.state.value.toEnd)
    }
}
