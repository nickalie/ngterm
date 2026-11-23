package com.nickalie.ngterm.android.e2e

import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.click
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.printToString
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.R
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.rules.ExternalResource
import org.junit.rules.RuleChain
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import java.io.File
import java.util.UUID

/**
 * What the session pane's end-to-end tests do as a person does on a phone, against a real ngterm machine: sign in
 * from a fresh install, add the machine, open a session, type into its shell, tap its cells and flip the system's
 * appearance.
 *
 * The programs run in the session's shell and use the machine's own `ngtermctl` (the script's stage is on the
 * machine's PATH, its private state dir in the environment). The machine is the private `ngterm-headless` behind the
 * private `sshd` that `scripts/test-android-e2e.sh` starts; without the script's arguments a test is skipped. Each
 * test signs in from a fresh install, and the app keeps its stores in memory for the process's life, so each test is
 * a class of its own, which the script runs in an instrumentation run of its own.
 */
internal class SessionLinkDriver {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
    val compose = createAndroidComposeRule<MainActivity>()

    // the script's machine serves every test of a run: names of this test's own keep another test's sessions out
    private val runId = UUID.randomUUID().toString().take(RUN_ID_LENGTH)
    val session = "qa-links-work-$runId"
    val linked = "qa-links-target-$runId"

    private val stack =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "no machine: run it through scripts/test-android-e2e.sh",
                    listOf(HOST, PORT, USER, KEY).all { !arguments.getString(it).isNullOrBlank() },
                )
            }
        }

    /** A fresh install (no key, no machine, no pin), cleared before the activity starts, in the light appearance. */
    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
                val context = instrumentation.targetContext
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
                appearance(night = false)
            }

            override fun after() {
                appearance(night = false)
            }
        }

    private val evidence =
        object : TestWatcher() {
            override fun failed(
                e: Throwable?,
                description: Description,
            ) {
                screenshot("${description.methodName}-failed")
                Log.e(TAG, "terminal at the failure:\n${terminalText()}")
                // the key screens are secure, so their screenshots are black: their texts say where the test stopped
                Log.e(
                    TAG,
                    "screen at the failure:\n${runCatching { compose.onRoot().printToString() }.getOrDefault("")}",
                )
            }
        }

    val rules: RuleChain =
        RuleChain
            .outerRule(stack)
            .around(freshInstall)
            .around(compose)
            .around(evidence)

    // makes the session [linked] and covers the screen's top rows with a link to it
    fun printALinkToANewSession() {
        run(
            "id=\$(ngtermctl session new --no-select --name $linked)\n" +
                "printf '\\033[H\\033[2J'\n" +
                "for i in 1 2 3 4; do printf '\\033]8;;ngterm://session/%s\\007%s\\033]8;;\\007\\n' \"\$id\" " +
                "open-linked-session-link-here; done\n" +
                "printf '%s-%s\\n' links ready\n" +
                "sleep 300\n",
        )
        awaitTerminal("links-ready")
        screenshot("link-printed")
    }

    fun openANewSession(name: String) {
        signIn()
        addMachine()
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(text(R.string.machine_sessions_new))
        compose.onAllNodesWithText(text(R.string.machine_sessions_new))[0].performTouchInput { longClick() }
        awaitText(text(R.string.new_session_create))
        compose.onNodeWithText(text(R.string.new_session_name)).performTextInput(name)
        compose.onNodeWithText(text(R.string.new_session_create)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) > 0 }
        // the first attach asks for notifications (Android 13 and later): Not now, as one may answer
        val title = hasText(text(R.string.notifications_ask_title))
        val asked = runCatching { compose.waitUntil(ASK_WAIT_MS) { nodes(title) > 0 } }.isSuccess
        if (asked) compose.onNodeWithText(text(R.string.notifications_ask_later)).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        // with no hardware keyboard, only a tap gives the terminal the focus that the typed keys need
        compose.onNode(terminal()).performClick()
        compose.waitForIdle()
    }

    private fun signIn() {
        val key = String(Base64.decode(arguments.getString(KEY), Base64.DEFAULT))
        awaitText(text(R.string.welcome_import))
        compose.onNodeWithText(text(R.string.welcome_import)).performClick()
        compose.onNodeWithText(text(R.string.import_key_label)).performTextInput(key)
        compose.onNodeWithText(text(R.string.import_action)).performScrollTo().performClick()
        awaitText(text(R.string.key_done))
        compose.onNodeWithText(text(R.string.key_done)).performScrollTo().performClick()
        awaitText(text(R.string.machines_add))
    }

    private fun addMachine() {
        compose.onAllNodesWithText(text(R.string.machines_add))[0].performClick()
        compose.onNodeWithText(text(R.string.form_label)).performTextInput(MACHINE)
        compose.onNodeWithText(text(R.string.form_host)).performTextInput(arguments.getString(HOST).orEmpty())
        compose.onNodeWithText(text(R.string.form_port)).performTextReplacement(arguments.getString(PORT).orEmpty())
        compose.onNodeWithText(text(R.string.form_user)).performTextInput(arguments.getString(USER).orEmpty())
        compose.onNodeWithText(text(R.string.form_save)).performClick()
        awaitText(text(R.string.hostkey_trust))
        compose.onNodeWithText(text(R.string.hostkey_trust)).performClick()
        awaitText(MACHINE)
    }

    // runs [script] in the session's shell, typed as one short line any login shell runs: its text goes base64, and
    // it runs from a file, so it reads the terminal and not its own text
    fun run(script: String) {
        val encoded = Base64.encodeToString(script.toByteArray(), Base64.NO_WRAP)
        // `input text` loses keys of a long text, or of one sent before the last one's keys landed: the line goes in
        // short pieces, each once the shell echoed the one before
        val pieces = listOf("echo%s") + encoded.chunked(TYPED_PIECE) + "|base64%s-d>qa-run.sh;sh%sqa-run.sh"
        var typed = ""
        pieces.forEach { piece ->
            shell("input text $piece")
            typed += piece.replace("%s", " ")
            val echoed =
                runCatching {
                    compose.waitUntil(CONNECT_WAIT_MS) { typed.trimEnd() in terminalText().replace("\n", "") }
                }.isSuccess
            assertTrue("the shell never echoed $typed:\n${terminalText()}", echoed)
        }
        shell("input keyevent KEYCODE_ENTER")
    }

    // a finger's tap on the screen's first cell, which the programs above cover with their link
    fun tapTheTopLeftCell() {
        compose.onNode(terminal()).performTouchInput { click(Offset(TAP_INSET_PX, TAP_INSET_PX)) }
    }

    fun nodes(matcher: SemanticsMatcher): Int = compose.onAllNodes(matcher).fetchSemanticsNodes().size

    fun more(session: String): SemanticsMatcher = hasContentDescription(text(R.string.action_more, session))

    private fun terminal(): SemanticsMatcher = hasContentDescription(text(R.string.terminal_description))

    fun terminalText(): String =
        runCatching {
            compose
                .onAllNodes(terminal())
                .fetchSemanticsNodes()
                .mapNotNull { it.config.getOrNull(SemanticsProperties.Text) }
                .flatten()
                .joinToString("\n") { it.text }
        }.getOrDefault("")

    fun awaitTerminal(needle: String) {
        val shown = runCatching { compose.waitUntil(CONNECT_WAIT_MS) { needle in terminalText() } }.isSuccess
        assertTrue("the terminal never showed $needle:\n${terminalText()}", shown)
    }

    fun awaitText(text: String) = compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text)) > 0 }

    fun text(
        id: Int,
        vararg args: Any,
    ): String = instrumentation.targetContext.getString(id, *args)

    fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/session-link-$name.png")
    }

    // the system's appearance; `cmd uimode` may hold its output open past the change, so its output is read aside
    fun appearance(night: Boolean) {
        val output = instrumentation.uiAutomation.executeShellCommand("cmd uimode night ${if (night) "yes" else "no"}")
        Thread { ParcelFileDescriptor.AutoCloseInputStream(output).use { it.readBytes() } }.start()
        Thread.sleep(SETTLE_MS)
    }

    fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    companion object {
        private const val TAG = "QaE2e"
        private const val RUN_ID_LENGTH = 8

        /** The instrumentation arguments the script passes: the host, the machine's port, the user and key. */
        private const val HOST = "e2eHost"
        private const val PORT = "e2ePort"
        private const val USER = "e2eUser"
        private const val KEY = "e2eKey"

        const val MACHINE = "qa-links"
        const val UNKNOWN = "0E2B7C1A-5D4F-4E3B-9A61-7C2D8F0B1A23"

        /** The color scheme reports of the light and the dark appearance, as the screen shows them without ESC. */
        const val LIGHT = "[?997;2n"
        const val DARK = "[?997;1n"

        /** Inside the first cell from the terminal's top left corner, at any text size the app offers. */
        private const val TAP_INSET_PX = 6f

        /** A piece of the base64 text short enough for `input text` to type whole. */
        private const val TYPED_PIECE = 32

        private const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
        const val ASK_WAIT_MS = 5_000L

        /** How long the system takes to give the app a new appearance. */
        const val SETTLE_MS = 2_000L
    }
}
