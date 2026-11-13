package com.nickalie.ngterm.android.e2e

import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.uiautomator.UiDevice
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.R
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.ExternalResource
import org.junit.rules.RuleChain
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import org.junit.runner.RunWith
import java.io.File

/**
 * A Mac running upstream agterm, end to end: the app signs in to it over ssh as to any machine, lists its live session
 * from agterm's own CLI (the machine has no `ngtermctl`), marks the machine and the session "agterm" with what agterm
 * lacks behind the badge, offers no chat view there, attaches the session through agterm's bundled zmx (its shell's
 * prompt reaches the terminal), and ends it through agterm's CLI.
 *
 * The machine is the conformance suite's stand-in for agterm v0.41.1 (`conformance/agterm_conformance/fake_agterm.py`:
 * its `agtermctl` and its `zmx`, answering as upstream's sources and fixtures say) behind the private `sshd` that
 * `scripts/test-android-e2e.sh` starts, on a port of its own whose logins see only agterm ([AGTERM_PORT]); the
 * script seeds it with the session [SESSION]. Without the script's arguments the test is skipped. No live agterm.
 */
@RunWith(AndroidJUnit4::class)
class AgtermPeerE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
    private val device = UiDevice.getInstance(instrumentation)
    private val compose = createAndroidComposeRule<MainActivity>()

    private val stack =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "no agterm machine: run it through scripts/test-android-e2e.sh",
                    listOf(HOST, AGTERM_PORT, USER, KEY).all { !arguments.getString(it).isNullOrBlank() },
                )
            }
        }

    /** A fresh install (no key, no machine, no pin), cleared before the activity starts. */
    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
                val context = instrumentation.targetContext
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
            }
        }

    private val evidence =
        object : TestWatcher() {
            override fun failed(
                e: Throwable?,
                description: Description,
            ) {
                shell("screencap -p $EVIDENCE_DIR/agterm-failed.png")
                Log.e(TAG, "terminal at the failure:\n${terminalText()}")
            }
        }

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(stack).around(freshInstall).around(compose).around(evidence)

    @Test
    fun anAgtermMacsSessionIsListedAttachedAndEndedWithoutAChatView() {
        signInAndAddTheMachine()

        // the card says which app the machine runs once its listing is in
        awaitText(AGTERM)
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(SESSION)
        compose.onNode(hasContentDescription("$AGTERM. ", substring = true)).assertExists()

        // the session's menu offers no chat view, and says why
        compose.onNodeWithContentDescription(text(R.string.action_more, SESSION)).performClick()
        awaitText(text(R.string.session_chat_view_unavailable, AGTERM))
        compose.onNodeWithText(text(R.string.session_chat_view)).assertIsNotEnabled()
        device.pressBack()

        // attached through agterm's zmx: its shell's prompt reaches the terminal
        compose.onNodeWithText(SESSION).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { compose.onAllNodes(terminal()).fetchSemanticsNodes().isNotEmpty() }
        notNowToNotifications()
        val prompted =
            runCatching {
                compose.waitUntil(CONNECT_WAIT_MS) { "attached agterm-" in terminalText() && PROMPT in terminalText() }
            }.isSuccess
        Log.i(TAG, "terminal:\n${terminalText()}")
        assertTrue("agterm's shell never showed in the pane; it shows:\n${terminalText()}", prompted)
        // the session bar carries the badge too
        compose.onNode(hasContentDescription("$AGTERM. ", substring = true)).assertExists()
        shell("screencap -p $EVIDENCE_DIR/agterm-attached.png")

        // End session goes to agterm's CLI, and the session leaves the machine's list
        compose.onAllNodes(hasContentDescription(text(R.string.action_more, SESSION)))[0].performClick()
        compose.onNodeWithText(text(R.string.machine_sessions_end)).performClick()
        compose.onNode(hasText(text(R.string.machine_sessions_end)) and hasAnyAncestor(isDialog())).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) {
            compose.onAllNodes(hasText(SESSION)).fetchSemanticsNodes().isEmpty()
        }
    }

    private fun signInAndAddTheMachine() {
        val key = String(Base64.decode(arguments.getString(KEY), Base64.DEFAULT))
        awaitText(text(R.string.welcome_import))
        compose.onNodeWithText(text(R.string.welcome_import)).performClick()
        compose.onNodeWithText(text(R.string.import_key_label)).performTextInput(key)
        compose.onNodeWithText(text(R.string.import_action)).performScrollTo().performClick()
        awaitText(text(R.string.key_done))
        compose.onNodeWithText(text(R.string.key_done)).performScrollTo().performClick()
        awaitText(text(R.string.machines_add))
        compose.onAllNodesWithText(text(R.string.machines_add))[0].performClick()
        compose.onNodeWithText(text(R.string.form_label)).performTextInput(MACHINE)
        compose.onNodeWithText(text(R.string.form_host)).performTextInput(arguments.getString(HOST).orEmpty())
        compose
            .onNodeWithText(text(R.string.form_port))
            .performTextReplacement(arguments.getString(AGTERM_PORT).orEmpty())
        compose.onNodeWithText(text(R.string.form_user)).performTextInput(arguments.getString(USER).orEmpty())
        compose.onNodeWithText(text(R.string.form_save)).performClick()
        awaitText(text(R.string.hostkey_trust))
        compose.onNodeWithText(text(R.string.hostkey_trust)).performClick()
        awaitText(MACHINE)
    }

    /** The first attach asks for notifications (Android 13 and later): Not now, as one may answer. */
    private fun notNowToNotifications() {
        val title = hasText(text(R.string.notifications_ask_title))
        val asked =
            runCatching {
                compose.waitUntil(ASK_WAIT_MS) { compose.onAllNodes(title).fetchSemanticsNodes().isNotEmpty() }
            }.isSuccess
        if (asked) compose.onNodeWithText(text(R.string.notifications_ask_later)).performClick()
    }

    private fun terminal(): SemanticsMatcher = hasContentDescription(text(R.string.terminal_description))

    private fun terminalText(): String =
        runCatching {
            compose
                .onAllNodes(terminal())
                .fetchSemanticsNodes()
                .mapNotNull { it.config.getOrNull(SemanticsProperties.Text) }
                .flatten()
                .joinToString("\n") { it.text }
        }.getOrDefault("")

    private fun awaitText(text: String) =
        compose.waitUntil(CONNECT_WAIT_MS) { compose.onAllNodes(hasText(text)).fetchSemanticsNodes().isNotEmpty() }

    private fun text(
        id: Int,
        vararg args: Any,
    ): String = instrumentation.targetContext.getString(id, *args)

    private fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    private companion object {
        const val TAG = "QaE2e"

        /** The instrumentation arguments the script passes: the host, the agterm machine's port, user and key. */
        const val HOST = "e2eHost"
        const val AGTERM_PORT = "e2eAgtermPort"
        const val USER = "e2eUser"
        const val KEY = "e2eKey"

        const val MACHINE = "qa-agterm"

        /** The session the script seeds on the stand-in agterm, and the prompt its zmx's shell shows. */
        const val SESSION = "qa-agterm-work"
        const val PROMPT = "agterm$"
        const val AGTERM = "agterm"

        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
        const val ASK_WAIT_MS = 5_000L
    }
}
