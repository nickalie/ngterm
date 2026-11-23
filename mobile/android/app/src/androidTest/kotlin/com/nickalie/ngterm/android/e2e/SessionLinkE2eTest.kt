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
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.performTouchInput
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.Ignore
import org.junit.Rule
import org.junit.Test
import org.junit.rules.ExternalResource
import org.junit.rules.RuleChain
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import org.junit.runner.RunWith
import java.io.File

/**
 * A session's pane as a person uses it on a phone, against a real ngterm machine:
 * - a tap on an OSC 8 link `ngterm://session/<id>` opens that session, attached first since the phone has not
 *   attached it; a tap on a link to a session the machine does not list does the usual tap and the view stays;
 * - a program that asks for the color scheme (`CSI ? 996 n`) gets the app's light or dark, and with mode 2031 on, a
 *   flip of the system's appearance is reported to it;
 * - a key typed on the phone into the session's pane clears the session's unseen badge on the machine.
 *
 * The programs run in the session's shell and use the machine's own `ngtermctl` (the script's stage is on the
 * machine's PATH, its private state dir in the environment). The machine is the private `ngterm-headless` behind the
 * private `sshd` that `scripts/test-android-e2e.sh` starts; without the script's arguments the test is skipped. Each
 * test signs in from a fresh install, so the script runs each as `SessionLinkE2eTest#<test>` of its own.
 */
@RunWith(AndroidJUnit4::class)
class SessionLinkE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
    private val compose = createAndroidComposeRule<MainActivity>()

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
            }
        }

    @get:Rule
    val rules: RuleChain =
        RuleChain
            .outerRule(stack)
            .around(freshInstall)
            .around(compose)
            .around(evidence)

    @Test
    fun aTapOnASessionLinkOpensThatSessionAttachedFirst() {
        openANewSession(SESSION)
        printALinkToANewSession()
        // the machine's sessions screen lists the machine again; the session then opens from the Sessions list
        compose.onNode(hasContentDescription(text(R.string.action_back))).performClick()
        compose.onAllNodesWithText(text(R.string.nav_machines))[0].performClick()
        awaitText(MACHINE)
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(LINKED)
        screenshot("link-listed")
        compose.onAllNodesWithText(text(R.string.nav_sessions))[0].performClick()
        awaitText(SESSION)
        assertEquals("the phone attached the linked session before the tap", 0, nodes(hasText(LINKED)))
        compose.onNodeWithText(SESSION).performClick()
        awaitTerminal("links-ready")
        tapTheTopLeftCell()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(more(LINKED)) > 0 }
        // the machine's own terminal of the new session leads it, so its pane shows behind Take control
        awaitText(text(R.string.session_take_control))
        screenshot("linked-opened")
        assertEquals("the linked session's bar is not the one shown", 0, nodes(more(SESSION)))
        compose.onNode(hasContentDescription(text(R.string.action_back))).performClick()
        awaitText(LINKED)
        screenshot("linked-attached")
    }

    @Ignore("Q1 of QA 328: a link to a session made after the phone's last listing does the usual tap")
    @Test
    fun aTapOnALinkToASessionMadeAfterTheListingOpensIt() {
        openANewSession(SESSION)
        printALinkToANewSession()
        tapTheTopLeftCell()
        val opened = runCatching { compose.waitUntil(CONNECT_WAIT_MS) { nodes(more(LINKED)) > 0 } }.isSuccess
        screenshot("link-unlisted-tap")
        assertTrue("a tap on a link to a session made after the listing did not open it", opened)
    }

    // makes the session [LINKED] and covers the screen's top rows with a link to it
    private fun printALinkToANewSession() {
        run(
            "id=\$(ngtermctl session new --no-select --name $LINKED)\n" +
                "printf '\\033[H\\033[2J'\n" +
                "for i in 1 2 3 4; do printf '\\033]8;;ngterm://session/%s\\007%s\\033]8;;\\007\\n' \"\$id\" " +
                "open-linked-session-link-here; done\n" +
                "printf '%s-%s\\n' links ready\n" +
                "sleep 300\n",
        )
        awaitTerminal("links-ready")
        screenshot("link-printed")
    }

    @Test
    fun aTapOnALinkToASessionTheMachineDoesNotListStaysInThePane() {
        openANewSession(SESSION)
        run(
            "printf '\\033[H\\033[2J'\n" +
                "for i in 1 2 3 4; do printf '\\033]8;;ngterm://session/%s\\007%s\\033]8;;\\007\\n' " +
                "$UNKNOWN unknown-session-link-here; done\n" +
                "printf '%s-%s\\n' links ready\n" +
                "sleep 300\n",
        )
        awaitTerminal("links-ready")
        tapTheTopLeftCell()
        val moved = runCatching { compose.waitUntil(ASK_WAIT_MS) { nodes(more(SESSION)) == 0 } }.isSuccess
        screenshot("unknown-tapped")
        assertTrue("a tap on an unknown session's link left the session", !moved)
        assertTrue(terminalText().contains("links-ready"))
    }

    @Test
    fun aProgramGetsTheAppsAppearanceAndItsFlip() {
        openANewSession(SESSION)
        run(
            "stty raw -echo; printf '\\033[?996n'; r=\$(dd bs=1 count=9 2>/dev/null); stty sane\n" +
                "printf '%s:%s\\n' asked \"\$r\" | tr -d '\\033'\n",
        )
        awaitTerminal("asked:$LIGHT")
        screenshot("scheme-light")

        run(
            "stty raw -echo; printf '\\033[?2031h%s%s\\r\\n' arm ed; r=\$(dd bs=1 count=9 2>/dev/null)\n" +
                "printf '\\033[?2031l'; stty sane; printf '%s%s:%s\\n' flip ped \"\$r\" | tr -d '\\033'\n",
        )
        awaitTerminal("armed")
        appearance(night = true)
        awaitTerminal("flipped:$DARK")
        screenshot("scheme-flipped")

        run(
            "stty raw -echo; printf '\\033[?996n'; r=\$(dd bs=1 count=9 2>/dev/null); stty sane\n" +
                "printf '%s:%s\\n' again \"\$r\" | tr -d '\\033'\n",
        )
        awaitTerminal("again:$DARK")
    }

    @Test
    fun aKeyTypedOnThePhoneClearsTheSessionsBadge() {
        openANewSession(SESSION)
        run(
            "count() { ngtermctl tree --json | python3 -c 'import json,sys\n" +
                "t=json.load(sys.stdin)[\"result\"][\"tree\"]\n" +
                "print(sum(s.get(\"unseen\",0) for w in t[\"workspaces\"] for s in w[\"sessions\"]" +
                " if s[\"name\"]==sys.argv[1]))' $SESSION; }\n" +
                "id=\$(ngtermctl tree --json | python3 -c 'import json,sys\n" +
                "t=json.load(sys.stdin)[\"result\"][\"tree\"]\n" +
                "print(next(s[\"id\"] for w in t[\"workspaces\"] for s in w[\"sessions\"]" +
                " if s[\"name\"]==sys.argv[1]))' $SESSION)\n" +
                "ngtermctl notify qa-badge --target \"\$id\" >/dev/null\n" +
                "sleep 1\n" +
                "echo \"badge-before=\$(count)\"\n" +
                "read -r _\n" +
                "sleep 2\n" +
                "echo \"badge-after=\$(count)\"\n",
        )
        awaitTerminal("badge-before=1")
        screenshot("badge-raised")
        shell("input text x")
        shell("input keyevent KEYCODE_ENTER")
        awaitTerminal("badge-after=0")
        screenshot("badge-cleared")
    }

    private fun openANewSession(name: String) {
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
    private fun run(script: String) {
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
    private fun tapTheTopLeftCell() {
        compose.onNode(terminal()).performTouchInput { click(Offset(TAP_INSET_PX, TAP_INSET_PX)) }
    }

    private fun nodes(matcher: SemanticsMatcher): Int = compose.onAllNodes(matcher).fetchSemanticsNodes().size

    private fun more(session: String): SemanticsMatcher = hasContentDescription(text(R.string.action_more, session))

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

    private fun awaitTerminal(needle: String) {
        val shown = runCatching { compose.waitUntil(CONNECT_WAIT_MS) { needle in terminalText() } }.isSuccess
        assertTrue("the terminal never showed $needle:\n${terminalText()}", shown)
    }

    private fun awaitText(text: String) = compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text)) > 0 }

    private fun text(
        id: Int,
        vararg args: Any,
    ): String = instrumentation.targetContext.getString(id, *args)

    private fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/session-link-$name.png")
    }

    // the system's appearance; `cmd uimode` may hold its output open past the change, so its output is read aside
    private fun appearance(night: Boolean) {
        val output = instrumentation.uiAutomation.executeShellCommand("cmd uimode night ${if (night) "yes" else "no"}")
        Thread { ParcelFileDescriptor.AutoCloseInputStream(output).use { it.readBytes() } }.start()
        Thread.sleep(SETTLE_MS)
    }

    private fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    private companion object {
        const val TAG = "QaE2e"

        /** The instrumentation arguments the script passes: the host, the machine's port, the user and key. */
        const val HOST = "e2eHost"
        const val PORT = "e2ePort"
        const val USER = "e2eUser"
        const val KEY = "e2eKey"

        const val MACHINE = "qa-links"
        const val SESSION = "qa-links-work"
        const val LINKED = "qa-links-target"
        const val UNKNOWN = "0E2B7C1A-5D4F-4E3B-9A61-7C2D8F0B1A23"

        /** The color scheme reports of the light and the dark appearance, as the screen shows them without ESC. */
        const val LIGHT = "[?997;2n"
        const val DARK = "[?997;1n"

        /** Inside the first cell from the terminal's top left corner, at any text size the app offers. */
        const val TAP_INSET_PX = 6f

        /** A piece of the base64 text short enough for `input text` to type whole. */
        const val TYPED_PIECE = 32

        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
        const val ASK_WAIT_MS = 5_000L

        /** How long the system takes to give the app a new appearance. */
        const val SETTLE_MS = 2_000L
    }
}
