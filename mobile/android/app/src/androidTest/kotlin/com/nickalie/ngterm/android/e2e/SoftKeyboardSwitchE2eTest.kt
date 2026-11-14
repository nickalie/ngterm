package com.nickalie.ngterm.android.e2e

import android.os.Build
import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
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
import androidx.test.filters.SdkSuppress
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
 * Typing survives the input method binding again, end to end: in a live session on a real machine, the soft keyboard
 * goes to another input method and comes back, as when a person taps Gboard's mic for voice typing (or picks another
 * keyboard) and returns, and what they type after it still reaches the shell; again for a second and a third round trip
 * (one with a word half typed as the keyboard goes, one through the system's input method switch), and after leaving
 * the session and opening it again.
 *
 * The real stack: this app and its mobile core, signed in over ssh to a private `ngterm-headless` behind a private
 * `sshd` on the host, which `scripts/test-android-e2e.sh` starts and hands over as instrumentation arguments
 * ([HOST], [PORT], [USER], [KEY]); without them the test is skipped. The keyboards are the test APK's own
 * ([QaKeyboard], [QaVoiceTyping]), so the run does not hang on which keyboards the device has: they type through the
 * connection the system hands the input method, as Gboard does, and switch input methods through the system as Gboard's
 * mic key and voice typing do. Each step types `expr 7000 + <n>` and waits for the shell's answer on the terminal's
 * screen (its text as TalkBack reads it), which the typed line itself never shows.
 */
@RunWith(AndroidJUnit4::class)
@SdkSuppress(minSdkVersion = Build.VERSION_CODES.P)
class SoftKeyboardSwitchE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
    private val device = UiDevice.getInstance(instrumentation)
    private val compose = createAndroidComposeRule<MainActivity>()

    /** The stack's address, or the skip, before anything starts. */
    private val stack =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "no machine: run it through scripts/test-android-e2e.sh",
                    listOf(HOST, PORT, USER, KEY).all { !arguments.getString(it).isNullOrBlank() },
                )
            }
        }

    /** A fresh install (no key, no machine, no pin), cleared before the activity starts, as LaunchSmokeTest does. */
    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
                val context = instrumentation.targetContext
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
            }
        }

    /** The test's keyboards on for the test, the device's own back after it. */
    private val keyboards = QaKeyboards(instrumentation)

    /** A screenshot and the terminal's text as a step fails, kept where the script collects them. */
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
            .around(keyboards)
            .around(compose)
            .around(evidence)

    @Test
    fun typingReachesTheShellAfterTheKeyboardSwitchesAwayAndBack() {
        signInAndOpenANewSession()
        showKeyboard()
        typeAndExpectTheAnswer(1)

        // Gboard's mic: to voice typing and back, then type
        toVoiceTypingAndBack()
        typeAndExpectTheAnswer(2)

        // again, with a word half typed as the keyboard goes: it lands, and the rest follows it
        keyboards.tapKeys("ex")
        toVoiceTypingAndBack()
        typeAndExpectTheAnswer(3, typed = "pr")

        // the system's own switch (the keyboard picker): to the other input method and back
        shell("ime set ${keyboards.voiceTyping}")
        keyboards.awaitKey(QaVoiceTyping.BACK)
        shell("ime set ${keyboards.keyboard}")
        keyboards.awaitKey(QaKeyboard.keyDescription("e"))
        typeAndExpectTheAnswer(4)
        screenshot("after-switches")

        // leaving the session and opening it again: typing still reaches it, and switches still keep it
        leaveAndReopenTheSession()
        showKeyboard()
        typeAndExpectTheAnswer(5)
        toVoiceTypingAndBack()
        typeAndExpectTheAnswer(6)
        screenshot("passed")
    }

    private fun signInAndOpenANewSession() {
        val host = arguments.getString(HOST).orEmpty()
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
        compose.onNodeWithText(text(R.string.form_host)).performTextInput(host)
        compose.onNodeWithText(text(R.string.form_port)).performTextReplacement(arguments.getString(PORT).orEmpty())
        compose.onNodeWithText(text(R.string.form_user)).performTextInput(arguments.getString(USER).orEmpty())
        compose.onNodeWithText(text(R.string.form_save)).performClick()
        awaitText(text(R.string.hostkey_trust))
        compose.onNodeWithText(text(R.string.hostkey_trust)).performClick()
        awaitText(MACHINE)
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(text(R.string.machine_sessions_new))
        // a long press names it, so the test finds it again in the list
        compose.onAllNodesWithText(text(R.string.machine_sessions_new))[0].performTouchInput { longClick() }
        awaitText(text(R.string.new_session_create))
        compose.onNodeWithText(text(R.string.new_session_name)).performTextInput(SESSION)
        compose.onNodeWithText(text(R.string.new_session_create)).performClick()
        awaitTerminal()
        notNowToNotifications()
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

    private fun leaveAndReopenTheSession() {
        // a phone's session bar has Back, to the attached sessions' list; a tablet's list stays beside the session, so
        // it leaves for Machines and comes back through Sessions
        val back = hasContentDescription(text(R.string.action_back))
        if (compose.onAllNodes(back).fetchSemanticsNodes().isNotEmpty()) {
            compose.onNode(back).performClick()
        } else {
            compose.onNodeWithText(text(R.string.nav_machines)).performClick()
        }
        compose.waitUntil(CONNECT_WAIT_MS) { compose.onAllNodes(terminal()).fetchSemanticsNodes().isEmpty() }
        if (compose.onAllNodes(hasText(SESSION)).fetchSemanticsNodes().isEmpty()) {
            compose.onNodeWithText(text(R.string.nav_sessions)).performClick()
        }
        awaitText(SESSION)
        compose.onAllNodesWithText(SESSION)[0].performClick()
        awaitTerminal()
    }

    /**
     * A tap on the terminal, which brings up the soft keyboard: the test's. Once the session is ready for it: the
     * terminal shown on its own (no cover over it while it attaches) and the shell's prompt drawn on it.
     */
    private fun showKeyboard() {
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        compose.waitForIdle()
        val bounds = compose.onNode(terminal()).fetchSemanticsNode().boundsInWindow
        device.click(bounds.center.x.toInt(), bounds.center.y.toInt())
        keyboards.awaitKey(QaKeyboard.keyDescription("e"))
    }

    /** The mic key to the voice-typing stand-in, and its key back to the keyboard. */
    private fun toVoiceTypingAndBack() {
        keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.VOICE))
        keyboards.tapKey(QaVoiceTyping.BACK)
        keyboards.awaitKey(QaKeyboard.keyDescription("e"))
    }

    /** Types `expr 7000 + <step>` (after [typed], what is already on the line) and Enter, and waits for its answer. */
    private fun typeAndExpectTheAnswer(
        step: Int,
        typed: String = "expr",
    ) {
        keyboards.tapKeys(typed)
        keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.SPACE))
        keyboards.tapKeys("7000")
        keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.SPACE))
        keyboards.tapKeys("+")
        keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.SPACE))
        keyboards.tapKeys("$step")
        keyboards.tapKey(QaKeyboard.keyDescription(QaKeyboard.ENTER))
        val answer = "${ANSWER_BASE + step}"
        val came =
            runCatching {
                compose.waitUntil(ANSWER_WAIT_MS) { terminalText().lines().any { it.trim() == answer } }
            }.isSuccess
        Log.i(TAG, "step $step: terminal now:\n${terminalText()}")
        assertTrue("step $step: the shell never answered $answer; the terminal shows:\n${terminalText()}", came)
    }

    private fun terminal(): SemanticsMatcher = hasContentDescription(text(R.string.terminal_description))

    private fun awaitTerminal() =
        compose.waitUntil(CONNECT_WAIT_MS) { compose.onAllNodes(terminal()).fetchSemanticsNodes().isNotEmpty() }

    /** The focused terminal's screen as TalkBack reads it: its rows, one per line. */
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

    private fun text(id: Int): String = instrumentation.targetContext.getString(id)

    private fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/soft-keyboard-$name.png")
    }

    private fun shell(command: String): String =
        ParcelFileDescriptor
            .AutoCloseInputStream(instrumentation.uiAutomation.executeShellCommand(command))
            .bufferedReader()
            .use { it.readText() }

    private companion object {
        const val TAG = "QaE2e"

        /** The instrumentation arguments the script passes: the machine's host, ssh port, user and private key. */
        const val HOST = "e2eHost"
        const val PORT = "e2ePort"
        const val USER = "e2eUser"
        const val KEY = "e2eKey"

        const val MACHINE = "qa-machine"

        const val SESSION = "qa-session"
        const val ANSWER_BASE = 7000

        /** Where screenshots go on the device; scripts/test-android-e2e.sh collects them. */
        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val ANSWER_WAIT_MS = 10_000L
        const val CONNECT_WAIT_MS = 60_000L
        const val ASK_WAIT_MS = 5_000L
    }
}
