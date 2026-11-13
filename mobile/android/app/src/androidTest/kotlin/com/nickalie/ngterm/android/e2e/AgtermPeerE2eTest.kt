package com.nickalie.ngterm.android.e2e

import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.hasAnyAncestor
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDialog
import androidx.compose.ui.test.isHeading
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
import androidx.test.uiautomator.UiDevice
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.core.fake.AGTERM_NOTE
import org.junit.Assert.assertEquals
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
 * A Mac running upstream agterm beside an ngterm machine, end to end: the app signs in to both over ssh as to any
 * machine; the agterm one lists its live session from agterm's own CLI (the machine has no `ngtermctl`), and its card,
 * its sessions screen and the attached session's bar say "agterm", the last two showing what agterm lacks on a tap or a
 * long press. Every menu of the agterm session (the machine's list, the attached list, the bar) has Chat view off with
 * "agterm has no chat view"; the session attaches through agterm's bundled zmx (its shell's prompt reaches the
 * terminal) and ends through agterm's CLI. The ngterm machine beside it shows no badge and keeps Chat view in its
 * session's menus. The stand-in's pane runs no agent, so the missing Show chat switch of an agterm agent pane is
 * `ChatModeTest`'s (its agterm case), not this test's.
 *
 * The agterm machine is the conformance suite's stand-in for agterm v0.41.1
 * (`conformance/agterm_conformance/fake_agterm.py`: its `agtermctl` and its `zmx`, answering as upstream's sources and
 * fixtures say) behind the private `sshd` that `scripts/test-android-e2e.sh` starts, on a port of its own whose logins
 * see only agterm ([AGTERM_PORT]); the script seeds it with the session [SESSION]. The ngterm machine is the script's
 * private `ngterm-headless` on the same sshd's other port ([PORT]). Without the script's arguments the test is skipped.
 * No live agterm or ngterm.
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
                    listOf(HOST, PORT, AGTERM_PORT, USER, KEY).all { !arguments.getString(it).isNullOrBlank() },
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
                screenshot("failed")
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
    fun anAgtermMacsSessionIsListedAttachedAndEndedWithoutAChatViewBesideAnNgtermMachine() {
        signIn()

        // an ngterm machine first, with a session of its own: its listing is in, and says ngterm
        addMachine(NGTERM_MACHINE, arguments.getString(PORT).orEmpty())
        compose.onNodeWithText(NGTERM_MACHINE).performClick()
        awaitText(text(R.string.machine_sessions_new))
        assertNoNotedBadge("ngterm machine's sessions screen")
        compose.onAllNodesWithText(text(R.string.machine_sessions_new))[0].performTouchInput { longClick() }
        awaitText(text(R.string.new_session_create))
        compose.onNodeWithText(text(R.string.new_session_name)).performTextInput(NGTERM_SESSION)
        compose.onNodeWithText(text(R.string.new_session_create)).performClick()
        awaitTerminal()
        notNowToNotifications()
        // the ngterm session's bar: no badge, and Chat view offered in its menu
        assertNoNotedBadge("ngterm session's bar")
        assertEveryMenuOffersChatView(NGTERM_SESSION)
        toMachines()

        // the agterm Mac: its card says which app it runs once its listing is in; the ngterm card says nothing
        addMachine(MACHINE, arguments.getString(AGTERM_PORT).orEmpty())
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(card(MACHINE) and hasText(AGTERM)) == 1 }
        assertEquals(
            "the ngterm machine's card has an agterm badge",
            0,
            nodes(card(NGTERM_MACHINE) and hasText(AGTERM)),
        )
        screenshot("cards")

        // its sessions screen: the badge, whose tap and long press show what agterm lacks
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(SESSION)
        showsTheNoteOnTapAndLongPress("agterm machine's sessions screen", heading = MACHINE)

        // the machine list's menu of its session offers no chat view, and says why
        assertEveryMenuHasNoChatView(SESSION)

        // attached through agterm's zmx: its shell's prompt reaches the terminal
        compose.onNodeWithText(SESSION).performClick()
        awaitTerminal()
        notNowToNotifications()
        val prompted =
            runCatching {
                compose.waitUntil(CONNECT_WAIT_MS) { "attached agterm-" in terminalText() && PROMPT in terminalText() }
            }.isSuccess
        Log.i(TAG, "terminal:\n${terminalText()}")
        assertTrue("agterm's shell never showed in the pane; it shows:\n${terminalText()}", prompted)

        // the bar carries the badge with its note, and its menu has no chat view
        showsTheNoteOnTapAndLongPress("agterm session's bar", heading = SESSION)
        assertEveryMenuHasNoChatView(SESSION)
        screenshot("attached")

        // the attached list holds both: the agterm session's menu has no chat view, the ngterm one's keeps it
        toAttachedList()
        screenshot("attached-list")
        assertEveryMenuOffersChatView(NGTERM_SESSION)
        assertEveryMenuHasNoChatView(SESSION)

        // End session goes to agterm's CLI, and the session leaves every list
        compose.onAllNodes(more(SESSION))[0].performClick()
        compose.onNodeWithText(text(R.string.machine_sessions_end)).performClick()
        compose.onNode(hasText(text(R.string.machine_sessions_end)) and hasAnyAncestor(isDialog())).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(SESSION)) == 0 }
        awaitText(NGTERM_SESSION)
        screenshot("ended")
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

    private fun addMachine(
        label: String,
        port: String,
    ) {
        compose.onAllNodesWithText(text(R.string.machines_add))[0].performClick()
        compose.onNodeWithText(text(R.string.form_label)).performTextInput(label)
        compose.onNodeWithText(text(R.string.form_host)).performTextInput(arguments.getString(HOST).orEmpty())
        compose.onNodeWithText(text(R.string.form_port)).performTextReplacement(port)
        compose.onNodeWithText(text(R.string.form_user)).performTextInput(arguments.getString(USER).orEmpty())
        compose.onNodeWithText(text(R.string.form_save)).performClick()
        awaitText(text(R.string.hostkey_trust))
        compose.onNodeWithText(text(R.string.hostkey_trust)).performClick()
        awaitText(label)
    }

    /**
     * The badge with a note on screen (the first, where a wide screen shows two): a tap shows the core's note, and so
     * does a long press, each dismissed by a tap away from it, on the screen's [heading].
     */
    private fun showsTheNoteOnTapAndLongPress(
        where: String,
        heading: String,
    ) {
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(notedBadge()) > 0 }
        assertEquals("$where: the note is up before a tap", 0, nodes(hasText(AGTERM_NOTE)))
        compose.onAllNodes(notedBadge())[0].performClick()
        awaitNote(where, "tap")
        screenshot("note-tap-${where.replace(' ', '-').replace("'", "")}")
        dismissTheNote(where, heading)
        compose.onAllNodes(notedBadge())[0].performTouchInput { longClick() }
        awaitNote(where, "long press")
        dismissTheNote(where, heading)
    }

    private fun awaitNote(
        where: String,
        how: String,
    ) {
        val shown =
            runCatching {
                compose.waitUntil(NOTE_WAIT_MS) { nodes(hasText(AGTERM_NOTE)) > 0 }
            }.isSuccess
        assertTrue("$where: a $how on the agterm badge shows no note", shown)
    }

    // a tap away from the badge, as a person dismisses a tooltip: on the screen's heading, which takes no tap itself
    private fun dismissTheNote(
        where: String,
        heading: String,
    ) {
        val bounds = compose.onAllNodes(isHeading() and hasText(heading))[0].fetchSemanticsNode().boundsInWindow
        device.click(bounds.center.x.toInt(), bounds.center.y.toInt())
        val gone = runCatching { compose.waitUntil(NOTE_WAIT_MS) { nodes(hasText(AGTERM_NOTE)) == 0 } }.isSuccess
        assertTrue("$where: the agterm note stays after a tap away from it", gone)
    }

    private fun assertNoNotedBadge(where: String) =
        assertEquals("$where shows an agterm badge", 0, nodes(notedBadge()) + nodes(hasText(AGTERM)))

    /** Every More menu of [session] on screen: Chat view disabled, with "agterm has no chat view" under it. */
    private fun assertEveryMenuHasNoChatView(session: String) {
        val menus = nodes(more(session))
        assertTrue("no More menu of $session on screen", menus > 0)
        repeat(menus) { index ->
            compose.onAllNodes(more(session))[index].performClick()
            awaitText(text(R.string.session_chat_view_unavailable, AGTERM))
            compose.onNodeWithText(text(R.string.session_chat_view)).assertIsNotEnabled()
            device.pressBack()
            compose.waitUntil(NOTE_WAIT_MS) { nodes(hasText(text(R.string.session_chat_view))) == 0 }
        }
    }

    /** Every More menu of [session] on screen: Chat view takes a tap, with no word of agterm under it. */
    private fun assertEveryMenuOffersChatView(session: String) {
        val menus = nodes(more(session))
        assertTrue("no More menu of $session on screen", menus > 0)
        repeat(menus) { index ->
            compose.onAllNodes(more(session))[index].performClick()
            awaitText(text(R.string.session_chat_view))
            compose.onNodeWithText(text(R.string.session_chat_view)).assertIsEnabled()
            assertEquals(
                "$session's menu says agterm has no chat view",
                0,
                nodes(hasText(text(R.string.session_chat_view_unavailable, AGTERM))),
            )
            device.pressBack()
            compose.waitUntil(NOTE_WAIT_MS) { nodes(hasText(text(R.string.session_chat_view))) == 0 }
        }
    }

    /**
     * Closes the open session to the attached list, in every layout: a phone's session bar has Back; a wider screen's
     * has none, and its rail's or sidebar's sessions are no list with menus, so system Back closes it there (a first
     * Back may only put the keyboard away).
     */
    private fun toAttachedList() {
        val back = hasContentDescription(text(R.string.action_back))
        if (nodes(back) > 0) compose.onAllNodes(back)[0].performClick()
        repeat(SYSTEM_BACKS) {
            if (nodes(terminal()) > 0) {
                device.pressBack()
                runCatching { compose.waitUntil(NOTE_WAIT_MS) { nodes(terminal()) == 0 } }
            }
        }
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) == 0 && nodes(more(NGTERM_SESSION)) > 0 }
    }

    private fun toMachines() {
        val back = hasContentDescription(text(R.string.action_back))
        if (nodes(back) > 0) compose.onAllNodes(back)[0].performClick()
        compose.onNodeWithText(text(R.string.nav_machines)).performClick()
        awaitText(text(R.string.machines_add))
    }

    /** The first attach asks for notifications (Android 13 and later): Not now, as one may answer. */
    private fun notNowToNotifications() {
        val title = hasText(text(R.string.notifications_ask_title))
        val asked = runCatching { compose.waitUntil(ASK_WAIT_MS) { nodes(title) > 0 } }.isSuccess
        if (asked) compose.onNodeWithText(text(R.string.notifications_ask_later)).performClick()
    }

    private fun awaitTerminal() = compose.waitUntil(CONNECT_WAIT_MS) { nodes(terminal()) > 0 }

    private fun nodes(matcher: SemanticsMatcher): Int = compose.onAllNodes(matcher).fetchSemanticsNodes().size

    // the badge that explains itself: its spoken label is "agterm. <the note>"
    private fun notedBadge(): SemanticsMatcher = hasContentDescription("$AGTERM. ", substring = true)

    private fun card(label: String): SemanticsMatcher = hasText(label)

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

    private fun awaitText(text: String) = compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text)) > 0 }

    private fun text(
        id: Int,
        vararg args: Any,
    ): String = instrumentation.targetContext.getString(id, *args)

    private fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/agterm-$name.png")
    }

    private fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    private companion object {
        const val TAG = "QaE2e"

        /** The instrumentation arguments the script passes: the host, both machines' ports, the user and key. */
        const val HOST = "e2eHost"
        const val PORT = "e2ePort"
        const val AGTERM_PORT = "e2eAgtermPort"
        const val USER = "e2eUser"
        const val KEY = "e2eKey"

        const val MACHINE = "qa-agterm"
        const val NGTERM_MACHINE = "qa-ngterm"
        const val NGTERM_SESSION = "qa-ngterm-work"

        /** The session the script seeds on the stand-in agterm, and the prompt its zmx's shell shows. */
        const val SESSION = "qa-agterm-work"
        const val PROMPT = "agterm$"
        const val AGTERM = "agterm"

        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
        const val NOTE_WAIT_MS = 5_000L
        const val ASK_WAIT_MS = 5_000L
        const val SYSTEM_BACKS = 2
    }
}
