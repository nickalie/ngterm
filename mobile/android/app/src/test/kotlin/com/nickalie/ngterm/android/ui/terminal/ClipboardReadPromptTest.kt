package com.nickalie.ngterm.android.ui.terminal

import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.awaitText
import com.nickalie.ngterm.android.data.RED_PNG
import com.nickalie.ngterm.android.data.allowed
import com.nickalie.ngterm.android.data.key
import com.nickalie.ngterm.android.data.part
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.NgtermApp
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import com.nickalie.ngterm.core.ffi.ClipboardPart
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
import org.robolectric.annotation.GraphicsMode

/**
 * A program's clipboard read in the app on the fake core: the sheet, its preview of each type, each answer, the grant
 * switch, and a read the core dismisses.
 */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w411dp-h891dp")
@GraphicsMode(GraphicsMode.Mode.NATIVE)
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

    private fun request(
        id: ULong,
        mimes: List<String> = listOf("text/plain"),
        canRemember: Boolean = false,
    ) {
        compose.runOnIdle {
            test.core.emit(
                MobileEvent.ClipboardReadRequest(
                    pane,
                    id,
                    PaneClipboard.STANDARD,
                    false,
                    mimes,
                    false,
                    false,
                    canRemember,
                ),
            )
        }
    }

    /** The answers given, as (read id, reply) pairs that compare by their bytes. */
    private fun answers() =
        test.core.calls
            .filter { it.method == "answerClipboardRead" }
            .map { (it.args[3] as ULong) to (it.args[4] as ClipboardReply).key() }

    private val copied = part("text/plain", "copied text")

    @Test
    fun allowAndDenyAnswerEachReadOnce() {
        request(1u)
        compose.awaitText("Let docs-site read your clipboard?")
        compose.onNodeWithText("A program in docs-site on build-01", substring = true).assertExists()
        compose.awaitText("copied text")
        compose.onNodeWithText("text/plain, 11 bytes").assertExists()
        compose.onNodeWithText("Don't ask again for this program in this terminal").assertDoesNotExist()
        compose.onNodeWithText("Allow").performClick()
        compose.waitForIdle()
        assertEquals(listOf(1uL to allowed(copied)), answers())

        request(2u)
        compose.awaitText("Deny")
        compose.onNodeWithText("Deny").performClick()
        compose.waitForIdle()
        assertEquals(2uL to ClipboardReply.Deny.key(), answers().last())
        compose.onNodeWithText("Let docs-site read your clipboard?").assertDoesNotExist()
    }

    @Test
    fun everyRequestedTypeHasAPageAndTheGrantSwitchRemembers() {
        val arbitrary = ByteArray(2048) { (it % 256).toByte() }
        test.deviceClipboard.contents =
            listOf(
                copied,
                ClipboardPart("image/png", RED_PNG),
                ClipboardPart("application/octet-stream", arbitrary),
                ClipboardPart("text/html", ByteArray(0)),
            )
        request(1u, listOf("text/plain", "image/png", "application/octet-stream", "text/html"), canRemember = true)
        compose.awaitText("copied text")
        compose.onNodeWithText("image/png").performScrollTo().performClick()
        compose.onNodeWithContentDescription("Copied image, image/png").assertExists()
        compose.onNodeWithText("73 bytes").assertExists()
        compose.onNodeWithText("application/octet-stream").performScrollTo().performClick()
        compose.awaitText("No preview for this kind of content")
        compose.onNodeWithText("2.0 kB").assertExists()
        compose.onNodeWithText("text/html").performScrollTo().performClick()
        compose.awaitText("Empty")
        compose.onNodeWithText("Don't ask again for this program in this terminal").performScrollTo().performClick()
        compose.onNodeWithText("Allow").performClick()
        compose.waitForIdle()
        assertEquals(
            listOf(
                1uL to
                    allowed(
                        copied,
                        ClipboardPart("image/png", RED_PNG),
                        ClipboardPart("application/octet-stream", arbitrary),
                        ClipboardPart("text/html", ByteArray(0)),
                        remember = true,
                    ),
            ),
            answers(),
        )
    }

    @Test
    fun aClipboardHoldingNoneOfTheTypesSaysSo() {
        test.clipboard = null
        request(1u, listOf("image/png"))
        compose.awaitText("Your clipboard holds none of the kinds of content the program asks for.")
        compose.onNodeWithText("Allow").performClick()
        compose.waitForIdle()
        assertEquals(listOf(1uL to allowed()), answers())
    }

    @Test
    fun alwaysAllowStopsAskingForTheMachine() {
        request(1u)
        compose.awaitText("copied text")
        compose.onNodeWithText("Always allow on build-01").performClick()
        compose.waitForIdle()
        request(2u)
        compose.waitForIdle()
        compose.onNodeWithText("Let docs-site read your clipboard?").assertDoesNotExist()
        assertEquals(listOf(1uL to allowed(copied), 2uL to allowed(copied)), answers())
    }

    @Test
    fun aReadTheCoreDismissesClosesItsSheet() {
        request(1u)
        compose.awaitText("Let docs-site read your clipboard?")
        compose.runOnIdle { test.core.emit(MobileEvent.ClipboardReadDismiss(pane, 1u)) }
        compose.waitForIdle()
        compose.onNodeWithText("Let docs-site read your clipboard?").assertDoesNotExist()
        assertEquals(emptyList<Any>(), answers())
    }
}
