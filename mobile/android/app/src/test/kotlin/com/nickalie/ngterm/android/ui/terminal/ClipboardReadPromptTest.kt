package com.nickalie.ngterm.android.ui.terminal

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeCall
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.ClipboardReply
import com.nickalie.ngterm.core.ffi.MobileEvent
import com.nickalie.ngterm.core.ffi.PaneClipboard
import com.nickalie.ngterm.core.ffi.PaneRef
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/** A program's clipboard read in the app on the fake core: the sheet, each answer, and a read the core dismisses. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w411dp-h891dp")
class ClipboardReadPromptTest {
    @get:Rule
    val compose = createAndroidComposeRule<ComponentActivity>()

    @get:Rule
    val dir = TemporaryFolder()

    private lateinit var test: TestGraph
    private val pane = PaneRef("m1", "s1", "P1")

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
        test.seedKey()
        test.seedMachine(testMachine())
        test.clipboard = "copied text"
        compose.setContent {
            NgtermTheme { NgtermApp(graph = test.graph, coreVersion = "1.2.3", layout = NavLayout.Bar) }
        }
        compose.awaitText("build-01")
        test.core.emit(MobileEvent.Tree("m1", listOf(fakeSession("s1", name = "docs-site"))))
    }

    @After
    fun tearDown() = test.close()

    private fun request(id: ULong) {
        compose.runOnIdle { test.core.emit(MobileEvent.ClipboardReadRequest(pane, id, PaneClipboard.STANDARD, false)) }
    }

    private fun answers() = test.core.calls.filter { it.method == "answerClipboardRead" }

    private fun answer(
        id: ULong,
        reply: ClipboardReply,
    ) = FakeCall("answerClipboardRead", listOf("m1", "s1", "P1", id, reply))

    @Test
    fun allowAndDenyAnswerEachReadOnce() {
        request(1u)
        compose.awaitText("Let docs-site read your clipboard?")
        compose.onNodeWithText("A program in docs-site on build-01", substring = true).assertExists()
        compose.onNodeWithText("Allow").performClick()
        compose.waitForIdle()
        assertEquals(listOf(answer(1u, ClipboardReply.Allow("copied text"))), answers())

        request(2u)
        compose.awaitText("Deny")
        compose.onNodeWithText("Deny").performClick()
        compose.waitForIdle()
        assertEquals(answer(2u, ClipboardReply.Deny), answers().last())
        compose.onNodeWithText("Let docs-site read your clipboard?").assertDoesNotExist()
    }

    @Test
    fun alwaysAllowStopsAskingForTheMachine() {
        request(1u)
        compose.awaitText("Always allow on build-01")
        compose.onNodeWithText("Always allow on build-01").performClick()
        compose.waitForIdle()
        request(2u)
        compose.waitForIdle()
        compose.onNodeWithText("Let docs-site read your clipboard?").assertDoesNotExist()
        assertEquals(
            listOf(answer(1u, ClipboardReply.Allow("copied text")), answer(2u, ClipboardReply.Allow("copied text"))),
            answers(),
        )
    }

    @Test
    fun aReadTheCoreDismissesClosesItsSheet() {
        request(1u)
        compose.awaitText("Let docs-site read your clipboard?")
        compose.runOnIdle { test.core.emit(MobileEvent.ClipboardReadDismiss(pane, 1u)) }
        compose.waitForIdle()
        compose.onNodeWithText("Let docs-site read your clipboard?").assertDoesNotExist()
        assertEquals(emptyList<FakeCall>(), answers())
    }
}
