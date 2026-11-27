package com.nickalie.ngterm.android.e2e

import android.os.ParcelFileDescriptor
import android.os.SystemClock
import android.util.Base64
import android.util.Log
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsNode
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ComposeTimeoutException
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.hasScrollToNodeAction
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performScrollToNode
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.R
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Assume.assumeTrue
import org.junit.rules.ExternalResource
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.rules.TestWatcher
import org.junit.runner.Description
import java.io.File
import java.io.OutputStream
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread

/**
 * What the connect timeout's end-to-end tests do as a person does: set Connection timeout in Settings, add machines
 * with and without a timeout of their own, and see when a connect to a machine that never answers gives up.
 *
 * A machine that never answers is a [BlackHole] on the device's loopback: it accepts the TCP connection and never
 * sends an ssh banner, as a host behind a stuck firewall or a hung sshd does. The real machine is the private
 * `ngterm-headless` behind the private `sshd` of `scripts/test-android-e2e.sh`; without the script's arguments a test
 * is skipped. [fresh] clears the app's stores before the activity starts; without it the app starts with what an
 * earlier run stored, as after a restart.
 */
internal class ConnectTimeoutDriver(
    fresh: Boolean,
    extra: TestRule? = null,
) {
    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    val arguments = InstrumentationRegistry.getArguments()
    val compose = createAndroidComposeRule<MainActivity>()
    val settingsHole = BlackHole(SETTINGS_PORT)
    val ownHole = BlackHole(OWN_PORT)

    private val stack =
        object : ExternalResource() {
            override fun before() {
                assumeTrue(
                    "no machine: run it through scripts/test-android-e2e.sh",
                    listOf(HOST, PORT, USER, KEY).all { !arguments.getString(it).isNullOrBlank() },
                )
            }
        }

    private val install =
        object : ExternalResource() {
            override fun before() {
                if (!fresh) return
                val context = instrumentation.targetContext
                File(context.filesDir, "datastore").deleteRecursively()
                File(context.noBackupFilesDir, "keys").deleteRecursively()
            }
        }

    // the machines that never answer take connections before the activity starts, as the app connects at launch
    private val holes =
        object : ExternalResource() {
            override fun before() {
                settingsHole.open()
                ownHole.open()
            }

            override fun after() {
                settingsHole.close()
                ownHole.close()
            }
        }

    private val evidence =
        object : TestWatcher() {
            override fun failed(
                e: Throwable?,
                description: Description,
            ) {
                screenshot("${description.methodName}-failed")
                Log.e(TAG, "screen at the failure:\n${screenTexts()}")
            }
        }

    val rules: RuleChain =
        RuleChain
            .outerRule(stack)
            .around(install)
            .around(extra ?: RuleChain.emptyRuleChain())
            .around(holes)
            .around(compose)
            .around(evidence)

    fun signIn() {
        val key = String(Base64.decode(arguments.getString(KEY), Base64.DEFAULT))
        awaitText(text(R.string.welcome_import))
        compose.onNodeWithText(text(R.string.welcome_import)).performClick()
        compose.onNodeWithText(text(R.string.import_key_label)).performTextInput(key)
        compose.onNodeWithText(text(R.string.import_action)).performScrollTo().performClick()
        awaitText(text(R.string.key_done))
        compose.onNodeWithText(text(R.string.key_done)).performScrollTo().performClick()
        awaitText(text(R.string.machines_add))
    }

    /** Fills Add machine as a person types it, [timeout] empty for none; Save is left to the caller. */
    fun fillNewMachine(
        label: String,
        host: String,
        port: Int,
        timeout: String,
    ) {
        compose.onAllNodesWithText(text(R.string.machines_add))[0].performClick()
        awaitText(text(R.string.form_save))
        compose.onNodeWithText(text(R.string.form_label)).performTextInput(label)
        compose.onNodeWithText(text(R.string.form_host)).performTextInput(host)
        compose.onNodeWithText(text(R.string.form_port)).performTextReplacement(port.toString())
        compose.onNodeWithText(text(R.string.form_user)).performTextInput(arguments.getString(USER).orEmpty())
        if (timeout.isNotEmpty()) {
            timeoutField().performScrollTo().performTextInput(timeout)
        }
    }

    fun save() {
        compose.onNodeWithText(text(R.string.form_save)).performClick()
    }

    /** The real machine of the script, which connects once its host key is trusted. */
    fun addRealMachine() {
        fillNewMachine(REAL, arguments.getString(HOST).orEmpty(), arguments.getString(PORT).orEmpty().toInt(), "")
        save()
        awaitText(text(R.string.hostkey_trust))
        compose.onNodeWithText(text(R.string.hostkey_trust)).performClick()
        awaitConnected(REAL)
    }

    fun awaitConnected(label: String) {
        reveal(label)
        val connected =
            runCatching {
                compose.waitUntil(CONNECT_WAIT_MS) {
                    text(R.string.status_online) in
                        card(label)?.descriptions().orEmpty()
                }
            }.isSuccess
        assertTrue("$label never connected: ${card(label)?.words()}", connected)
    }

    /**
     * Waits for the connect to [label] behind [hole] to give up, and returns the seconds from the connection [hole]
     * accepted as number [attempt] to the app's close of it. The card then says that the machine did not answer.
     */
    fun secondsToGiveUp(
        label: String,
        hole: BlackHole,
        attempt: Int,
    ): Double {
        try {
            compose.waitUntil(GIVE_UP_MAX_MS) { hole.connections.getOrNull(attempt - 1)?.closed != null }
        } catch (e: ComposeTimeoutException) {
            throw AssertionError("the app never closed its connection number $attempt to $label", e)
        }
        val connection = hole.connections[attempt - 1]
        val seconds = (requireNotNull(connection.closed) - connection.accepted) / MS_PER_SECOND
        Log.i(TAG, "$label gave up %.2f s after its connection number $attempt".format(seconds))
        reveal(label)
        try {
            compose.waitUntil(CONNECT_WAIT_MS) { gaveUp(label) }
        } catch (e: ComposeTimeoutException) {
            throw AssertionError("the card of $label never said that it gave up: ${card(label)?.words()}", e)
        }
        return seconds
    }

    fun assertGaveUpAfter(
        expected: Int,
        seconds: Double,
        what: String,
    ) {
        assertTrue(
            "$what gave up after %.2f s, not after $expected s".format(seconds),
            seconds >= expected - EARLY_S && seconds <= expected + LATE_S,
        )
    }

    private fun gaveUp(label: String): Boolean = card(label)?.words().orEmpty().any { NO_ANSWER in it }

    fun retry(label: String) {
        reveal(label)
        compose.onAllNodes(hasText(text(R.string.action_retry)) and isInCard(label))[0].performClick()
    }

    fun openSettings() {
        compose.onAllNodesWithText(text(R.string.nav_settings))[0].performClick()
        awaitText(text(R.string.settings_connections))
    }

    fun openMachines() {
        compose.onAllNodesWithText(text(R.string.nav_machines))[0].performClick()
        awaitText(text(R.string.machines_add))
    }

    /** The Connection timeout slider, its note scrolled onto the screen with it. */
    fun timeoutSlider() =
        compose.onNodeWithContentDescription(text(R.string.settings_connect_timeout)).also {
            compose.onNodeWithText(text(R.string.settings_connect_timeout_note)).performScrollTo()
        }

    /** Moves the slider as an accessibility service (TalkBack's adjust) does, to whole [seconds]. */
    fun setSettingsTimeout(seconds: Int) {
        timeoutSlider().performSemanticsAction(SemanticsActions.SetProgress) { it(seconds.toFloat()) }
        awaitText(text(R.string.settings_connect_timeout_value, seconds))
    }

    /**
     * Waits until the Add machine hint shows [seconds]. The hint comes from the stored settings, as the core's value
     * does; the slider's label shows a move before the store has it.
     */
    fun awaitStoredSettingsTimeout(seconds: Int) {
        compose.onAllNodesWithText(text(R.string.machines_add))[0].performClick()
        awaitText(text(R.string.form_save))
        timeoutField().performScrollTo()
        awaitText(text(R.string.form_timeout_hint, seconds))
        closeEditor()
    }

    fun openEditor(label: String) {
        reveal(label)
        compose.onNodeWithContentDescription(text(R.string.action_more, label)).performClick()
        awaitText(text(R.string.machine_edit))
        compose.onNodeWithText(text(R.string.machine_edit)).performClick()
        awaitText(text(R.string.form_save))
    }

    fun timeoutField() = compose.onNode(hasSetTextAction() and hasText(text(R.string.form_timeout)))

    /** What the editor's Connection timeout field holds, its label left out. */
    fun timeoutFieldValue(): String =
        timeoutField()
            .performScrollTo()
            .fetchSemanticsNode()
            .config
            .getOrNull(SemanticsProperties.EditableText)
            ?.text
            .orEmpty()

    fun assertTimeoutHint(settingsSeconds: Int) {
        val hint = text(R.string.form_timeout_hint, settingsSeconds)
        timeoutField().performScrollTo()
        assertEquals(
            "the field's hint",
            1,
            compose.onAllNodes(hasText(hint), useUnmergedTree = true).fetchSemanticsNodes().size,
        )
    }

    /** Closes the machine editor with no change. */
    fun closeEditor() {
        compose.onNodeWithContentDescription(text(R.string.form_close)).performClick()
        // the Add form's title is the same text as the list's Add button: wait for its Save to go
        awaitNoText(text(R.string.form_save))
        awaitText(text(R.string.machines_add))
    }

    // a card of the list composes only on the screen: the list scrolls to it first
    private fun reveal(label: String) {
        compose.waitUntil(CONNECT_WAIT_MS) {
            compose.onAllNodes(hasScrollToNodeAction()).fetchSemanticsNodes().isNotEmpty()
        }
        compose.onAllNodes(hasScrollToNodeAction())[0].performScrollToNode(hasText(label))
    }

    private fun isInCard(label: String): SemanticsMatcher =
        SemanticsMatcher("in the card of $label") { node ->
            generateSequence(node.parent) { it.parent }.any { label in it.words() }
        }

    // the card's node: the clickable surface, which merges its title, address, badge and summary
    private fun card(label: String): SemanticsNode? =
        compose
            .onAllNodes(hasText(label))
            .fetchSemanticsNodes()
            .firstOrNull { it.config.getOrNull(SemanticsActions.OnClick) != null }

    private fun SemanticsNode.words(): List<String> =
        config.getOrNull(SemanticsProperties.Text).orEmpty().map { it.text }

    private fun SemanticsNode.descriptions(): List<String> =
        config.getOrNull(SemanticsProperties.ContentDescription).orEmpty()

    private fun screenTexts(): String =
        runCatching {
            compose
                .onAllNodes(SemanticsMatcher.keyIsDefined(SemanticsProperties.Text), useUnmergedTree = true)
                .fetchSemanticsNodes()
                .mapNotNull { it.config.getOrNull(SemanticsProperties.Text) }
                .flatten()
                .joinToString("\n") { it.text }
        }.getOrDefault("")

    fun awaitText(text: String) =
        compose.waitUntil(CONNECT_WAIT_MS) {
            compose.onAllNodes(hasText(text), useUnmergedTree = true).fetchSemanticsNodes().isNotEmpty()
        }

    private fun awaitNoText(text: String) =
        compose.waitUntil(CONNECT_WAIT_MS) {
            compose.onAllNodes(hasText(text), useUnmergedTree = true).fetchSemanticsNodes().isEmpty()
        }

    fun text(
        id: Int,
        vararg args: Any,
    ): String = instrumentation.targetContext.getString(id, *args)

    fun screenshot(name: String) {
        compose.waitForIdle()
        ParcelFileDescriptor
            .AutoCloseInputStream(
                instrumentation.uiAutomation.executeShellCommand(
                    "screencap -p $EVIDENCE_DIR/connect-timeout-$name.png",
                ),
            ).use { it.readBytes() }
    }

    /**
     * A machine that never answers: it listens on [port] of the loopback, accepts each connection and holds it open
     * with no word until the app closes it or [close]. [connections] has the times ([SystemClock.elapsedRealtime]).
     */
    class BlackHole(
        private val port: Int,
    ) {
        class Connection(
            val accepted: Long,
        ) {
            @Volatile var closed: Long? = null
        }

        val connections = CopyOnWriteArrayList<Connection>()
        private val held = CopyOnWriteArrayList<Socket>()
        private var server: ServerSocket? = null

        fun open() {
            val listening = ServerSocket(port, BACKLOG, InetAddress.getByName(LOOPBACK)).also { server = it }
            thread(name = "black-hole-$port", isDaemon = true) {
                runCatching {
                    while (true) {
                        val socket = listening.accept()
                        val connection = Connection(SystemClock.elapsedRealtime())
                        connections += connection
                        held += socket
                        thread(name = "black-hole-$port-read", isDaemon = true) {
                            // the client sends its banner and waits for this side's, which never comes: the read ends
                            // when the client closes the connection
                            runCatching { socket.getInputStream().use { it.copyTo(OutputStream.nullOutputStream()) } }
                            connection.closed = SystemClock.elapsedRealtime()
                        }
                    }
                }
            }
        }

        fun close() {
            server?.close()
            held.forEach { runCatching { it.close() } }
        }
    }

    companion object {
        private const val TAG = "QaTimeout"

        private const val HOST = "e2eHost"
        private const val PORT = "e2ePort"
        private const val USER = "e2eUser"
        private const val KEY = "e2eKey"

        /** Set by the run that restarts the app over what [ConnectTimeoutE2eTest] stored (commands in its doc). */
        const val KEPT = "e2eConnectTimeoutKept"

        const val LOOPBACK = "127.0.0.1"

        /** The machine that never answers and takes Settings' timeout, and the one with a timeout of its own. */
        const val SETTINGS_MACHINE = "qa-timeout-settings"
        const val OWN_MACHINE = "qa-timeout-own"
        const val REAL = "qa-timeout-real"

        // fixed, so the run after a restart finds the machines the run before it added where they were; below 32768,
        // out of the ephemeral range that the script's adb reverse ports come from
        const val SETTINGS_PORT = 27231
        const val OWN_PORT = 27232

        const val SETTINGS_SECONDS = 4
        const val OWN_SECONDS = 7
        const val DEFAULT_SECONDS = 10

        private const val NO_ANSWER = "did not answer in time"
        private const val BACKLOG = 8
        private const val MS_PER_SECOND = 1000.0

        // early: the app's deadline starts before the TCP connect that the hole stamps; late: the app's work to close
        private const val EARLY_S = 0.3
        private const val LATE_S = 2.5
        private const val GIVE_UP_MAX_MS = 70_000L

        private const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
    }
}
