package com.nickalie.ngterm.android.ui.chat

import android.Manifest
import android.app.Notification
import android.app.NotificationManager
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasScrollToIndexAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isHeading
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeDown
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestApplication
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.Alerts
import com.nickalie.ngterm.android.data.OpenTarget
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.platform.Notifications
import com.nickalie.ngterm.android.platform.openTarget
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.terminal.SelectingPane
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.ChatBody
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.RemoteStatus
import com.nickalie.ngterm.core.ffi.StreamState
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.Shadows.shadowOf
import org.robolectric.annotation.Config

private const val WAIT_MS = 10_000L

/**
 * A "waiting for you" notification about a pane running an agent, tapped, in the app on the fake core: the session
 * opens on that pane, as its chat at its newest row with the card the agent waits on, or as its terminal while
 * Settings' Agent chat view is off; and a tap for the pane already on screen brings its chat, scrolled away, back to
 * the card. The session's second pane (P2) runs Claude; the first (P1) runs nothing.
 */
@RunWith(AndroidJUnit4::class)
@Config(application = TestApplication::class, qualifiers = "w1000dp-h800dp-mdpi")
class ChatNotificationTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private val p2 = PaneRef("m1", "s1", "P2")

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
        val app = compose.activity.application as TestApplication
        app.testGraph = test.graph
        shadowOf(app).grantPermissions(Manifest.permission.POST_NOTIFICATIONS)
        Alerts(
            scope,
            test.connections,
            test.asks,
            test.graph.machines.machines,
            test.notificationPrefs.enabled,
            Notifications(app),
            test.live.sessions,
        )
    }

    @After
    fun tearDown() {
        scope.cancel()
        test.close()
    }

    private fun waitFor(check: () -> Boolean) = compose.waitUntil(WAIT_MS, check)

    private fun described(text: String) =
        compose.onAllNodes(hasContentDescription(text)).fetchSemanticsNodes().isNotEmpty()

    private fun onScreen(node: SemanticsNodeInteraction) =
        try {
            node.assertIsDisplayed()
            true
        } catch (_: AssertionError) {
            false
        }

    // a prompt bubble is read as one line ("You: …")
    private fun row(text: String) = onScreen(compose.onNodeWithContentDescription("You: $text"))

    private fun card() = compose.onNode(hasText("Allow Bash?") and isHeading())

    // the app on Machines with docs-site (s1) on build-01 attached, its stream live with panes P1 and P2, and Claude
    // announced in P2
    private fun attach() {
        test.seedMachine(testMachine(keyId = test.seedKey().id))
        compose.setContent {
            NgtermTheme { NgtermApp(graph = test.graph, coreVersion = "1.2.3", layout = NavLayout.Bar) }
        }
        waitFor { test.connections.trees.value["m1"] != null }
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "docs-site"))
        test.core.paneFactory = { _, _, pane, _ -> SelectingPane(List(4) { "$pane line $it" }) }
        compose.onNodeWithText("build-01").performClick()
        waitFor { compose.onAllNodes(hasText("docs-site")).fetchSemanticsNodes().isNotEmpty() }
        compose.runOnIdle {
            runBlocking { test.connections.attach("m1", "s1") }
            test.core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Connected))
            test.core.emit(
                MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("P1", "P2"), "P1", "vertical", true)),
            )
            test.core.emit(MobileEvent.PaneAgent("m1", "s1", "P2", "claude", "c1"))
        }
        waitFor {
            test.live.sessions.value[SessionKey("m1", "s1")]
                ?.agents
                ?.containsKey("P2") == true
        }
    }

    // P2's agent waits for an answer: the notification it posts, and where its tap leads
    private fun blocked(): OpenTarget {
        compose.runOnIdle {
            test.core.emit(
                MobileEvent.SessionStatus("m1", "s1", RemoteStatus("blocked", false, null, null, "P2", null), true),
            )
        }
        compose.waitForIdle()
        val manager = compose.activity.getSystemService(NotificationManager::class.java)
        val notification: Notification = shadowOf(manager).getNotification("status:m1:s1", 1)
        return openTarget(shadowOf(notification.contentIntent).savedIntent)!!
    }

    // P2's conversation: thirty prompts, the last one's tool waiting on its permission prompt
    private fun converse() =
        compose.runOnIdle {
            val rows = (0 until 30).map { event("p$it", user("prompt $it")) }
            val events =
                listOf(event("c", ChatBody.Conversation("claude", "c1", null, null))) + rows +
                    listOf(event("a1", ChatBody.Approval(approval)), live)
            test.core.emit(MobileEvent.Chat(p2, events))
        }

    @Test
    fun aWaitingNotificationForAPaneInChatViewOpensItsChatAtTheCard() {
        attach()
        val target = blocked()
        assertEquals(OpenTarget("m1", "s1", "P2"), target)
        compose.runOnIdle { test.opens.request(target) }
        // the session opens on P2 alone, as its chat
        waitFor { p2 in test.core.openChats }
        assertTrue(PaneRef("m1", "s1", "P1") !in test.core.shownPanes)
        converse()
        waitFor { described("Chat with Claude Code") && onScreen(card()) }
        assertTrue("the chat did not open at its newest row", row("prompt 29"))
        assertTrue(!described("Terminal"))
    }

    @Test
    fun withTheSettingOffTheNotificationOpensThePanesTerminal() {
        runBlocking { test.settings.update { it.copy(agentChat = false) } }
        attach()
        compose.runOnIdle { test.opens.request(blocked()) }
        waitFor { p2 in test.core.shownPanes }
        waitFor { described("Terminal") }
        compose.waitForIdle()
        assertTrue(test.core.openChats.isEmpty())
        assertTrue(!described("Chat with The agent") && !described("Chat with Claude Code"))
    }

    @Test
    fun aTapForTheChatOnScreenBringsItBackFromWhereItWasScrolledToTheCard() {
        attach()
        compose.runOnIdle { test.opens.request(OpenTarget("m1", "s1", "P2")) }
        waitFor { p2 in test.core.openChats }
        converse()
        waitFor { onScreen(card()) && row("prompt 29") }
        compose
            .onNode(hasScrollToIndexAction())
            .performTouchInput { swipeDown(durationMillis = 600) }
        compose.waitForIdle()
        assertTrue("the swipe left the end in sight", !row("prompt 29"))

        compose.runOnIdle { test.opens.request(OpenTarget("m1", "s1", "P2")) }
        waitFor { test.opens.pending.value == null }
        waitFor { row("prompt 29") }
        assertTrue(onScreen(card()))
    }
}
