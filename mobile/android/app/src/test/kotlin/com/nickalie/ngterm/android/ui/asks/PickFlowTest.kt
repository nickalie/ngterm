package com.nickalie.ngterm.android.ui.asks

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performImeAction
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestApplication
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.terminal.SelectingPane
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeCall
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.PickReply
import com.nickalie.ngterm.core.ffi.PickRow
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.RemotePick
import com.nickalie.ngterm.core.ffi.RemotePickItem
import com.nickalie.ngterm.core.ffi.StreamState
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

private const val WAIT_MS = 10_000L

/** A pick the origin handed over: three items, one blocked, one idle, one with no status. */
fun testPick(
    allowCustom: Boolean = false,
    selection: String? = null,
    query: String? = null,
) = RemotePick(
    id = "K1",
    items =
        listOf(
            RemotePickItem("api", "api", "~/src/api", "blocked", null, null),
            RemotePickItem("web", "web", null, "idle", null, null),
            RemotePickItem("docs", "docs", null, null, null, null),
        ),
    prompt = "Switch to",
    query = query,
    allowCustom = allowCustom,
    selection = selection,
)

/**
 * A pick answered in the app on the fake core: a sheet while its session is on screen (a tap, the search text, the
 * custom row, Cancel, the keyboard's Go on the origin's selection), taken down when the core dismisses it, waiting
 * while an ask of its session is open or its session is out of sight, and gone with its session's detach.
 */
@RunWith(AndroidJUnit4::class)
@Config(application = TestApplication::class)
class PickFlowTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph
    private val key = SessionKey("m1", "s1")

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
        (compose.activity.application as TestApplication).testGraph = test.graph
    }

    @After
    fun tearDown() = test.close()

    private fun waitFor(check: () -> Boolean) = compose.waitUntil(WAIT_MS, check)

    private fun shows(text: String) = compose.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty()

    private fun answers() = test.core.calls.filter { it.method == "answerPick" }

    private fun answer(reply: PickReply) = FakeCall("answerPick", listOf("m1", "s1", "K1", reply))

    // the app with docs-site (s1) on build-01 attached; opened in its view when [open]
    private fun attach(open: Boolean = true) {
        test.seedMachine(testMachine(keyId = test.seedKey().id))
        compose.setContent {
            NgtermTheme { NgtermApp(graph = test.graph, coreVersion = "1.2.3", layout = NavLayout.Bar) }
        }
        waitFor { test.connections.trees.value["m1"] != null }
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "docs-site"))
        test.core.paneFactory = { _, _, pane, _ -> SelectingPane(List(4) { "$pane line $it" }) }
        compose.onNodeWithText("build-01").performClick()
        waitFor { shows("docs-site") }
        if (!open) {
            compose.runOnIdle { runBlocking { test.connections.attach("m1", "s1") } }
            return
        }
        compose.onNodeWithText("docs-site").performClick()
        waitFor {
            "s1" in
                test.connections.attached.value["m1"]
                    .orEmpty()
        }
        compose.runOnIdle {
            test.core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Connected))
            test.core.emit(MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("P1"), "P1", null, false)))
        }
        waitFor { test.asks.onScreen.value == key }
    }

    private fun pick(pick: RemotePick) = compose.runOnIdle { test.core.emit(MobileEvent.Pick("m1", "s1", pick)) }

    @Test
    fun aTappedItemAnswersWithItsIdOnce() {
        attach()
        pick(testPick())
        waitFor { shows("Switch to") }
        compose.onNodeWithText("~/src/api").assertExists()
        // a blocked item draws its glyph, read as its word; an idle one draws none
        compose.onNodeWithContentDescription("Waiting").assertExists()
        compose.onNodeWithText("web").performClick()
        waitFor { answers().isNotEmpty() }
        assertEquals(listOf(answer(PickReply.Picked("web"))), answers())
        waitFor { !shows("Switch to") }
    }

    @Test
    fun theSearchTextFiltersTheItemsAndAnUnmatchedOneIsTheCustomAnswer() {
        attach()
        pick(testPick(allowCustom = true))
        waitFor { shows("Switch to") }
        compose.onNode(hasSetTextAction()).performTextReplacement("do")
        waitFor { !shows("api") }
        compose.onNodeWithText("docs").assertExists()
        compose.onNode(hasSetTextAction()).performTextReplacement("zzz top")
        waitFor { shows("Use \"zzz top\"") }
        compose.onNodeWithText("Use \"zzz top\"").performClick()
        waitFor { answers().isNotEmpty() }
        assertEquals(listOf(answer(PickReply.Custom("zzz top"))), answers())
    }

    @Test
    fun anUnmatchedSearchWithoutCustomAnswersSaysSoAndCancelAnswersCancelled() {
        attach()
        pick(testPick())
        waitFor { shows("Switch to") }
        compose.onNode(hasSetTextAction()).performTextReplacement("zzz")
        waitFor { shows("No matches") }
        compose.onNodeWithText("Cancel").performClick()
        waitFor { answers().isNotEmpty() }
        assertEquals(listOf(answer(PickReply.Cancelled)), answers())
    }

    @Test
    fun theKeyboardsGoAnswersWithTheOriginsSelectionThenTheFirstRowAfterAnEdit() {
        attach()
        pick(testPick(selection = "docs"))
        waitFor { shows("Switch to") }
        compose.onNode(hasSetTextAction()).performImeAction()
        waitFor { answers().isNotEmpty() }
        assertEquals(answer(PickReply.Picked("docs")), answers().single())

        pick(testPick(selection = "docs").copy(id = "K2"))
        waitFor { shows("Switch to") }
        compose.onNode(hasSetTextAction()).performTextReplacement("a")
        compose.onNode(hasSetTextAction()).performImeAction()
        waitFor { answers().size == 2 }
        assertEquals(FakeCall("answerPick", listOf("m1", "s1", "K2", PickReply.Picked("api"))), answers().last())
    }

    @Test
    fun aPickTheCoreDismissesClosesItsSheet() {
        attach()
        pick(testPick(query = "a"))
        waitFor { shows("Switch to") }
        compose.runOnIdle { test.core.emit(MobileEvent.PickDismiss("m1", "s1", "K1")) }
        waitFor { !shows("Switch to") }
        assertEquals(emptyList<FakeCall>(), answers())
    }

    @Test
    fun aPickWaitsForAnOpenAskOfItsSessionAndForItsSessionOnScreen() {
        attach(open = false)
        pick(testPick())
        compose.waitForIdle()
        assertFalse(shows("Switch to"))
        assertEquals(1, test.picks.open.value.size)

        compose.runOnIdle {
            test.core.emit(MobileEvent.Ask("m1", "s1", testAsk()))
            test.asks.showing(key)
        }
        waitFor { shows("Deploy to production?") }
        assertFalse(shows("Switch to"))
        compose.runOnIdle { test.core.emit(MobileEvent.AskDismiss("m1", "s1", "A1")) }
        waitFor { shows("Switch to") }
    }

    @Test
    fun aDetachedSessionsPickIsGone() {
        attach(open = false)
        pick(testPick())
        waitFor {
            test.picks.open.value
                .isNotEmpty()
        }
        compose.runOnIdle { test.connections.detach("m1", "s1") }
        waitFor {
            test.picks.open.value
                .isEmpty()
        }
        compose.runOnIdle { assertFalse(test.picks.answer(key, "K1", PickReply.Cancelled)) }
        assertEquals(emptyList<FakeCall>(), answers())
    }

    @Test
    fun theOpeningRowIsTheOriginsSelectionWhenShownAndAReplyNamesItsItem() {
        val items = testPick().items
        val rows = listOf(PickRow.Item(2u), PickRow.Item(0u))
        assertEquals(1, initialRow(rows, items, "api"))
        assertEquals(0, initialRow(rows, items, "web"))
        assertEquals(0, initialRow(rows, items, null))
        assertEquals(PickReply.Picked("docs"), replyFor(PickRow.Item(2u), items))
        assertEquals(PickReply.Custom("x"), replyFor(PickRow.Custom("Use \"x\"", "x"), items))
        assertEquals(PickReply.Cancelled, replyFor(PickRow.Item(9u), items))
    }
}
