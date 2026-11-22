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
import com.nickalie.ngterm.android.data.AlertKind
import com.nickalie.ngterm.android.platform.LINK_NOTIFICATION_ID
import com.nickalie.ngterm.core.ffi.MobileEvent
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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
import java.util.Collections

/**
 * Every notification switch turned off in Settings, end to end: the session's agent asks a question (its sheet, while
 * the session is on screen), the app goes to the background, and there the agent sets its status to waiting and sends
 * a notification. The test sees the status reach the app with the session out of sight, and the notification reach
 * it, and then that none of them posts: the attached session's quiet notification, which Android needs to keep the
 * connection, is the only one of the agents' and machines' up. Then Questions is turned on again, and the question
 * that still waits posts.
 *
 * The machine is the private `ngterm-headless` behind the private `sshd` that `scripts/test-android-e2e.sh` starts;
 * without the script's arguments the test is skipped. No live ngterm.
 */
@RunWith(AndroidJUnit4::class)
@SdkSuppress(minSdkVersion = Build.VERSION_CODES.TIRAMISU)
class NotificationsOffE2eTest {
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
    fun withEverySwitchOffNothingButTheAttachedSessionsNotificationPosts() {
        signIn()
        addMachine()
        turnEverySwitchOff()
        openANewSession()
        val came = Collections.synchronizedSet(mutableSetOf<String>())
        val asks = graph().asks
        graph().connections.listen { event ->
            if (event is MobileEvent.SessionStatus && event.status?.status == "blocked") {
                // on screen, the status is taken down before any switch is read: then it tests nothing
                came += if (asks.onScreen.value == null) "status" else ON_SCREEN
            }
            if (event is MobileEvent.Notify) came += "notice"
        }

        // the agent's three signals, a moment after the app goes to the background
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        // with no hardware keyboard, only a tap gives the terminal the focus that the typed keys need
        compose.onNode(terminal()).performClick()
        compose.waitForIdle()
        awaitShell()
        SIGNALS.forEach(::type)

        // the question opens on screen, as its sheet; then the app goes to the background, where it would post
        compose.waitUntil(CONNECT_WAIT_MS) { asks.open.value.isNotEmpty() }
        shell("input keyevent KEYCODE_HOME")
        // the test rule's clock moves only when the test waits for it: then the app sees it left the screen
        compose.waitUntil(CONNECT_WAIT_MS) { asks.onScreen.value == null }
        compose.waitUntil(CONNECT_WAIT_MS) { ON_SCREEN in came || came.containsAll(listOf("status", "notice")) }
        assertFalse("the status came while its session was on screen", ON_SCREEN in came)
        // a post that a switch let through would land a moment after its event
        Thread.sleep(SETTLE_MS)
        compose.waitForIdle()
        Log.i(TAG, "notifications up in the background: ${manager().activeNotifications.map { it.tag }}")
        assertEquals("notifications up with every switch off", emptyList<String>(), alertsUp())
        assertTrue("the attached session's notification is down", LINK_NOTIFICATION_ID in manager().activeIds())

        runBlocking { graph().notificationPrefs.set(AlertKind.Questions, true) }
        compose.waitUntil(CONNECT_WAIT_MS) { alertsUp().any { it.startsWith("ask:") } }
        screenshot("question-on")
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

    private fun turnEverySwitchOff() {
        compose.onNodeWithText(text(R.string.nav_settings)).performClick()
        for (label in SWITCHES) {
            val row = hasText(text(label)) and SemanticsMatcher.keyIsDefined(TOGGLE)
            compose.onNode(row).performScrollTo().performClick()
            compose.waitUntil(CONNECT_WAIT_MS) {
                compose
                    .onNode(row)
                    .fetchSemanticsNode()
                    .config[TOGGLE] == ToggleableState.Off
            }
        }
        screenshot("all-off")
        compose.onNodeWithText(text(R.string.nav_machines)).performClick()
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

    private fun graph() = (context.applicationContext as NgtermApplication).graph

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
        shell("screencap -p $EVIDENCE_DIR/notifications-off-$name.png")
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

        const val MACHINE = "qa-quiet"
        const val SESSION = "qa-quiet-work"

        /**
         * Typed into the session's shell (fish; `%s` is a space to `input text`), a line each, since a long line loses
         * its end: `t` names this session, as the origin's active one is another; a separate process (fish runs a
         * `begin` block in the foreground) sends waiting and a notification once the app is in the background; the
         * question comes at once.
         */
        val SIGNALS =
            listOf(
                "set%s-x%st%s\$NGTERM_SESSION_ID",
                "fish%s-c%s'sleep%s10;ngtermctl%ssession%sstatus%sblocked%s--target%s\$t;" +
                    "ngtermctl%snotify%sqaoff%s--target%s\$t'%s&",
                "ngtermctl%sask%sopen%sQaOff%s--button%syes%s--button%sno%s--target%s\$t",
            )

        /** Typed until the shell prints [READY], which the typed line itself does not hold. */
        const val PING = "string%supper%sqaready"
        const val READY = "QAREADY"

        val SWITCHES =
            listOf(
                R.string.settings_alert_questions,
                R.string.settings_alert_waiting,
                R.string.settings_alert_done,
                R.string.settings_alert_notices,
                R.string.settings_alert_errors,
            )

        val TOGGLE = SemanticsProperties.ToggleableState

        const val ON_SCREEN = "status on screen"

        val ALERT_TAGS = listOf("ask:", "status:", "notice:", "lost:")

        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L

        const val SETTLE_MS = 2_000L
        const val PING_WAIT_MS = 3_000L
    }
}
