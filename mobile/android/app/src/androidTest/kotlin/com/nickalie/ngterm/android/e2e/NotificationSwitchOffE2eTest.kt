package com.nickalie.ngterm.android.e2e

import android.app.NotificationManager
import android.os.Build
import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.state.ToggleableState
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
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.NgtermApplication
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.platform.LINK_NOTIFICATION_ID
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
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
 * A notification switch turned off in Settings while its notifications are up, end to end: the session's agent asks
 * a question and sets its status to waiting once the user is in Settings, so both post; turning Questions off takes
 * the question's notification down, and turning Waiting for you off takes the status down. The agent then sets its
 * status to finished, which posts, and turning Finished off takes it down. The attached session's quiet notification
 * stays through all of it.
 *
 * The machine is the private `ngterm-headless` behind the private `sshd` that `scripts/test-android-e2e.sh` starts;
 * without the script's arguments the test is skipped. No live ngterm.
 */
@RunWith(AndroidJUnit4::class)
@SdkSuppress(minSdkVersion = Build.VERSION_CODES.TIRAMISU)
class NotificationSwitchOffE2eTest {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val arguments = InstrumentationRegistry.getArguments()
    private val compose = createAndroidComposeRule<MainActivity>()
    private val context get() = instrumentation.targetContext

    private val stack =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "no machine: run it through scripts/test-android-e2e.sh",
                    listOf(HOST, PORT, USER, KEY).all { !arguments.getString(it).isNullOrBlank() },
                )
            }
        }

    /** A fresh install (no key, no machine, no pin) that may post notifications, set before the activity starts. */
    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
                shell("pm grant ${context.packageName} android.permission.POST_NOTIFICATIONS")
                manager().cancelAll()
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
                Log.e(TAG, "notifications up: ${runCatching { alertsUp() }.getOrNull()}")
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
    fun aSwitchTurnedOffTakesItsNotificationsDown() {
        signIn()
        addMachine()
        openANewSession()
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        // with no hardware keyboard, only a tap gives the terminal the focus that the typed keys need
        compose.onNode(terminal()).performClick()
        compose.waitForIdle()
        awaitShell()
        SIGNALS.forEach(::type)

        // out of the session, to Settings, before the signals come: there they post
        compose.onNode(hasContentDescription(text(R.string.action_back))).performClick()
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasText(text(R.string.nav_settings))) > 0 }
        compose.onAllNodesWithText(text(R.string.nav_settings))[0].performClick()
        awaitText(text(R.string.settings_alert_questions))
        go(FIRST_GO)
        compose.waitUntil(CONNECT_WAIT_MS) { alertsUp().any { it.startsWith("ask:") } }
        compose.waitUntil(CONNECT_WAIT_MS) { alertsUp().any { it.startsWith("status:") } }
        screenshot("switch-off-up")

        turnOff(R.string.settings_alert_questions)
        compose.waitUntil(CONNECT_WAIT_MS) { alertsUp().none { it.startsWith("ask:") } }
        turnOff(R.string.settings_alert_waiting)
        compose.waitUntil(CONNECT_WAIT_MS) { alertsUp().none { it.startsWith("status:") } }
        assertEquals("notifications up with Questions and Waiting for you off", emptyList<String>(), alertsUp())

        // the agent finishes: Finished is still on, so it posts
        go(SECOND_GO)
        compose.waitUntil(CONNECT_WAIT_MS) { alertsUp().any { it.startsWith("status:") } }
        turnOff(R.string.settings_alert_done)
        compose.waitUntil(CONNECT_WAIT_MS) { alertsUp().none { it.startsWith("status:") } }
        screenshot("switch-off-down")
        assertEquals("notifications up with the three switches off", emptyList<String>(), alertsUp())
        assertTrue("the attached session's notification is down", LINK_NOTIFICATION_ID in manager().activeIds())
    }

    /** Lets the signals that wait on [name] go: they wait until the machine lists a session of that name. */
    private fun go(name: String) {
        runBlocking {
            val machine =
                graph()
                    .machines.machines
                    .first()
                    .single { it.label == MACHINE }
            graph().connections.createSession(machine.id, name)
        }
    }

    private fun graph() = (context.applicationContext as NgtermApplication).graph

    private fun turnOff(label: Int) {
        val row = hasText(text(label)) and SemanticsMatcher.keyIsDefined(TOGGLE)
        compose.onNode(row).performScrollTo().performClick()
        compose.waitUntil(CONNECT_WAIT_MS) {
            compose
                .onNode(row)
                .fetchSemanticsNode()
                .config[TOGGLE] == ToggleableState.Off
        }
    }

    // fish drops what is typed while it starts, so a line goes only once it answers one
    private fun awaitShell() {
        val deadline = System.currentTimeMillis() + CONNECT_WAIT_MS
        while (READY !in terminalText()) {
            check(System.currentTimeMillis() < deadline) { "the shell never answered" }
            type(PING)
            runCatching { compose.waitUntil(PING_WAIT_MS) { READY in terminalText() } }
        }
    }

    private fun type(line: String) {
        shell("input text $line")
        shell("input keyevent KEYCODE_ENTER")
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

    private fun openANewSession() {
        compose.onNodeWithText(MACHINE).performClick()
        awaitText(text(R.string.machine_sessions_new))
        compose.onAllNodesWithText(text(R.string.machine_sessions_new))[0].performTouchInput { longClick() }
        awaitText(text(R.string.new_session_create))
        compose.onNodeWithText(text(R.string.new_session_name)).performTextInput(SESSION)
        compose.onNodeWithText(text(R.string.new_session_create)).performClick()
    }

    private fun manager() = context.getSystemService(NotificationManager::class.java)

    private fun NotificationManager.activeIds() = activeNotifications.map { it.id }

    // the tags of the agents' and machines' notifications up (the GitHub build's update notice has its own switch)
    private fun alertsUp(): List<String> =
        manager().activeNotifications.mapNotNull { it.tag }.filter { tag -> ALERT_TAGS.any { tag.startsWith(it) } }

    private fun nodes(matcher: SemanticsMatcher): Int = compose.onAllNodes(matcher).fetchSemanticsNodes().size

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

    private fun text(id: Int): String = context.getString(id)

    private fun screenshot(name: String) {
        shell("screencap -p $EVIDENCE_DIR/notification-switch-off-$name.png")
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

        const val MACHINE = "qa-switch"
        const val SESSION = "qa-switch-work"

        /**
         * Typed into the session's shell (fish; `%s` is a space to `input text`), a line each, since a long line loses
         * its end: `t` names this session, as the origin's active one is another; `g` is a script that waits until
         * the machine lists a session named by its argument. Separate processes (fish runs a `begin` block in the
         * foreground) ask and set waiting once the test makes [FIRST_GO], and set finished once it makes [SECOND_GO].
         */
        val SIGNALS =
            listOf(
                "set%s-x%st%s\$NGTERM_SESSION_ID",
                "set%s-x%sg%s(mktemp)",
                "echo%s'while%snot%sngtermctl%stree|string%smatch%s-qr%s\$argv[1];sleep%s1;end'>\$g",
                "fish%s-c%s'fish%s\$g%sqago1;ngtermctl%sask%sopen%sQaUp%s" +
                    "--button%syes%s--button%sno%s--target%s\$t'%s&",
                "fish%s-c%s'fish%s\$g%sqago1;ngtermctl%ssession%sstatus%sblocked%s--target%s\$t'%s&",
                "fish%s-c%s'fish%s\$g%sqago2;ngtermctl%ssession%sstatus%scompleted%s--target%s\$t'%s&",
            )

        const val FIRST_GO = "qago1"
        const val SECOND_GO = "qago2"

        /** Typed until the shell prints [READY], which the typed line itself does not hold. */
        const val PING = "string%supper%sqaready"
        const val READY = "QAREADY"

        val TOGGLE = SemanticsProperties.ToggleableState

        val ALERT_TAGS = listOf("ask:", "status:", "notice:", "lost:")

        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L

        const val PING_WAIT_MS = 3_000L
    }
}
