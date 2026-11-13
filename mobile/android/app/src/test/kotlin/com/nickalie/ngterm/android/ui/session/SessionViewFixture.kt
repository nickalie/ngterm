package com.nickalie.ngterm.android.ui.session

import androidx.activity.ComponentActivity
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.StateRestorationTester
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.terminal.SelectingPane
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakePane
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PeerKind
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.StreamState
import org.junit.After
import org.junit.Before
import org.junit.Rule
import org.junit.rules.TemporaryFolder
import java.util.concurrent.ConcurrentHashMap

private const val WAIT_MS = 10_000L

/** The app and its fake-core terminal fixture, shared by the session interaction tests. */
abstract class SessionViewFixture {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val dir = TemporaryFolder()

    protected lateinit var test: TestGraph
    protected val layout = mutableStateOf(NavLayout.Bar)
    protected val panes = ConcurrentHashMap<String, FakePane>()
    protected val p1 = PaneRef("m1", "s1", "P1")
    protected val p2 = PaneRef("m1", "s1", "P2")

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
    }

    @After
    fun tearDown() = test.close()

    protected fun waitFor(check: () -> Boolean) = compose.waitUntil(WAIT_MS, check)

    protected fun shows(text: String) = compose.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty()

    protected fun described(text: String) =
        compose.onAllNodes(hasContentDescription(text)).fetchSemanticsNodes().isNotEmpty()

    // the app with docs-site (s1) on build-01 (running [peer]; null: ngterm) opened from the machine's list, its stream
    // live with panes P1 and P2
    protected fun openSession(
        withLayout: Boolean = true,
        restoration: StateRestorationTester? = null,
        peer: PeerKind? = null,
    ) {
        val key = test.seedKey()
        test.seedMachine(testMachine(keyId = key.id))
        val content: @Composable () -> Unit = {
            NgtermTheme { NgtermApp(graph = test.graph, coreVersion = "1.2.3", layout = layout.value) }
        }
        if (restoration == null) compose.setContent(content) else restoration.setContent(content)
        waitFor { test.connections.trees.value["m1"] != null }
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "docs-site", peer = peer))
        test.core.paneFactory =
            { _, _, pane, _ -> SelectingPane(List(4) { "$pane line $it" }).also { panes[pane] = it } }
        compose.onNodeWithText("build-01").performClick()
        compose.waitUntil(WAIT_MS) { shows("docs-site") }
        compose.onNodeWithText("docs-site").performClick()
        waitFor {
            "s1" in
                test.connections.attached.value["m1"]
                    .orEmpty()
        }
        test.core.emit(MobileEvent.SessionConnection("m1", "s1", StreamState.Connected))
        if (withLayout) {
            test.core.emit(
                MobileEvent.SessionLayout("m1", "s1", RemoteLayout(listOf("P1", "P2"), "P1", "vertical", true)),
            )
            compose.waitForIdle()
            waitFor { p1 in test.core.shownPanes }
        } else {
            compose.awaitText("Opening docs-site")
        }
    }
}
