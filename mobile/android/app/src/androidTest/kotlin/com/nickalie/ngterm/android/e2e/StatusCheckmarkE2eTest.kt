package com.nickalie.ngterm.android.e2e

import android.graphics.Bitmap
import android.graphics.Canvas
import android.graphics.Color
import android.os.ParcelFileDescriptor
import android.util.Base64
import android.util.Log
import androidx.compose.ui.graphics.asAndroidBitmap
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.SemanticsNodeInteraction
import androidx.compose.ui.test.captureToImage
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isDisplayed
import androidx.compose.ui.test.junit4.v2.createAndroidComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextReplacement
import androidx.compose.ui.test.performTouchInput
import androidx.core.content.ContextCompat
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import com.nickalie.ngterm.android.MainActivity
import com.nickalie.ngterm.android.NgtermApplication
import com.nickalie.ngterm.android.R
import com.nickalie.ngterm.android.data.ThemeChoice
import com.nickalie.ngterm.android.ui.theme.StatusShape
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
import kotlin.math.abs

/**
 * The checkmark status shape, end to end: the session's agent sets its status with `--shape checkmark` on the origin,
 * and the app draws a check in the session bar (which the chat view shows too), in the Sessions list (a tablet's
 * sidebar) and in the machine's session list, in the light and the dark theme. Then the agent sets `--shape bell`,
 * and the bar draws the bell. The test reads each glyph from the pixels on screen and matches it against every status
 * shape's drawable, so an app that draws another shape for `checkmark` (the status's own default, when the origin
 * leaves the shape out) fails.
 *
 * The machine is the private `ngterm-headless` behind the private `sshd` that `scripts/test-android-e2e.sh` starts;
 * without the script's arguments the test is skipped. No live ngterm.
 */
@RunWith(AndroidJUnit4::class)
class StatusCheckmarkE2eTest {
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

    /** A fresh install (no key, no machine, no pin, the default settings), set before the activity starts. */
    private val freshInstall =
        object : ExternalResource() {
            override fun before() {
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
    fun aCheckmarkStatusShowsACheckAndABellStatusABell() {
        signIn()
        addMachine()
        openANewSession()
        focusTheTerminal()
        awaitShell()
        type(TARGET)
        type(status(DONE_WIRE, "checkmark"))
        awaitBadge(done())
        inBothThemes("session-bar", done()) { assertGlyph("the session bar, $it", done(), StatusShape.Checkmark) }

        // back from the session view: the Sessions list of attached sessions (a tablet shows it in the sidebar)
        val back = compose.onAllNodesWithContentDescription(text(R.string.action_back))
        if (back.fetchSemanticsNodes().isNotEmpty()) back[0].performClick()
        awaitText(SESSION)
        inBothThemes("sessions-list", done()) { assertGlyph("the Sessions list, $it", done(), StatusShape.Checkmark) }

        // the machine's session list, as the origin lists it
        tap(text(R.string.nav_machines))
        awaitText(MACHINE)
        // a tablet's sidebar keeps its own badge on screen: wait for the machine's list to add one more
        val before = displayed(done())
        tap(MACHINE)
        awaitText(SESSION)
        compose.waitUntil(CONNECT_WAIT_MS) { displayed(done()) > before }
        inBothThemes("machine-sessions", done()) {
            assertGlyph("the machine's session list, $it", done(), StatusShape.Checkmark)
        }

        // another shape is drawn as it was: the bell
        tap(text(R.string.nav_sessions))
        awaitText(SESSION)
        tap(SESSION)
        focusTheTerminal()
        type(status(DONE_WIRE, "bell"))
        compose.waitUntil(CONNECT_WAIT_MS) {
            runCatching { bestShapes(done()).let { it.isNotEmpty() && it.all(StatusShape.Bell::equals) } }
                .getOrDefault(false)
        }
        inBothThemes(
            "session-bar-bell",
            done(),
        ) { assertGlyph("the session bar after bell, $it", done(), StatusShape.Bell) }
    }

    private fun done() = text(R.string.agent_completed)

    private fun focusTheTerminal() {
        compose.waitUntil(CONNECT_WAIT_MS) { terminalText().isNotBlank() }
        // with no hardware keyboard, only a tap gives the terminal the focus that the typed keys need
        compose.onNode(terminal()).performClick()
        compose.waitForIdle()
    }

    // a click through semantics: the soft keyboard may cover a tablet's rail, where an injected touch fails
    private fun tap(label: String) {
        compose.onAllNodes(hasText(label) and hasClickAction())[0].performSemanticsAction(SemanticsActions.OnClick)
        compose.waitForIdle()
    }

    // runs [check] with the theme's name and saves a screenshot named after [shot], light and then dark
    private fun inBothThemes(
        shot: String,
        label: String,
        check: (String) -> Unit,
    ) {
        for (theme in listOf(ThemeChoice.Light, ThemeChoice.Dark)) {
            useTheme(theme, label)
            val name = theme.name.lowercase()
            check(name)
            screenshot("$name-$shot")
        }
    }

    // every badge on screen read as [label] draws [expected]: none may draw another shape
    private fun assertGlyph(
        where: String,
        label: String,
        expected: StatusShape,
    ) {
        // a screen that comes in fades in: wait until each badge's glyph has ink
        runCatching {
            compose.waitUntil(CONNECT_WAIT_MS) {
                scores(label).let { all -> all.isNotEmpty() && all.all { it.values.max() > MIN_MATCH } }
            }
        }
        val badges = scores(label)
        assertTrue("no badge read as $label in $where", badges.isNotEmpty())
        badges.forEachIndexed { index, scores ->
            val top = scores.entries.sortedByDescending { it.value }.take(TOP)
            Log.i(TAG, "$where, badge $index: $top")
            assertTrue("no glyph drawn in $where (best matches $top)", top.first().value > MIN_MATCH)
            assertEquals("the glyph in $where (best matches $top)", expected, scores.maxBy { it.value }.key)
        }
    }

    private fun bestShapes(label: String): List<StatusShape> = scores(label).map { it.maxBy { e -> e.value }.key }

    // the badges on screen read as [label], not those of a screen on its way out
    private fun shownBadges(label: String): List<SemanticsNodeInteraction> {
        val matches = compose.onAllNodes(hasContentDescription(label), useUnmergedTree = true)
        return matches
            .fetchSemanticsNodes()
            .indices
            .map { matches[it] }
            .filter { it.isDisplayed() }
    }

    // a node may leave between the lookup and the read: then the screen is still changing
    private fun displayed(label: String): Int = runCatching { shownBadges(label).size }.getOrDefault(0)

    /**
     * For each badge on screen read as [label], how well each shape's drawable matches its glyph: the intersection
     * over the union of the glyph's pixels and the drawable's, at the best place in the badge's leading square.
     */
    private fun scores(label: String): List<Map<StatusShape, Double>> {
        compose.waitForIdle()
        val side = (GLYPH_DP * context.resources.displayMetrics.density).toInt()
        // the badge itself, not a row that merges its description
        return shownBadges(label).map { badge ->
            val ink = inkMask(badge.captureToImage().asAndroidBitmap())
            StatusShape.entries.associateWith { shape -> bestOverlap(ink, shapeMask(shape, side)) }
        }
    }

    private fun pixels(bitmap: Bitmap): IntArray {
        val pixels = IntArray(bitmap.width * bitmap.height)
        bitmap.getPixels(pixels, 0, bitmap.width, 0, 0, bitmap.width, bitmap.height)
        return pixels
    }

    // a badge's ground is its most common color
    private fun ground(pixels: IntArray): Int =
        pixels
            .toList()
            .groupingBy { it }
            .eachCount()
            .maxBy { it.value }
            .key

    // the badge's pixels that are not its ground
    private fun inkMask(bitmap: Bitmap): Mask {
        val pixels = pixels(bitmap)
        val ground = ground(pixels)
        return Mask(
            BooleanArray(pixels.size) { distance(pixels[it], ground) > INK_DISTANCE },
            bitmap.width,
            bitmap.height,
        )
    }

    private fun distance(
        a: Int,
        b: Int,
    ): Int =
        abs(Color.red(a) - Color.red(b)) + abs(Color.green(a) - Color.green(b)) + abs(Color.blue(a) - Color.blue(b))

    private fun shapeMask(
        shape: StatusShape,
        side: Int,
    ): Mask {
        val bitmap = Bitmap.createBitmap(side, side, Bitmap.Config.ARGB_8888)
        val drawable = checkNotNull(ContextCompat.getDrawable(context, shape.drawable)).mutate()
        drawable.setBounds(0, 0, side, side)
        drawable.draw(Canvas(bitmap))
        val pixels = IntArray(side * side)
        bitmap.getPixels(pixels, 0, side, 0, 0, side, side)
        return Mask(BooleanArray(pixels.size) { Color.alpha(pixels[it]) > HALF_ALPHA }, side, side)
    }

    private fun bestOverlap(
        ink: Mask,
        shape: Mask,
    ): Double {
        if (ink.width < shape.width || ink.height < shape.height) return 0.0
        val rows = 0..ink.height - shape.height
        val columns = 0..minOf(MAX_X_OFFSET, ink.width - shape.width)
        return rows.maxOf { dy -> columns.maxOf { dx -> overlap(ink, shape, dx, dy) } }
    }

    // the intersection over the union of [shape] placed at ([dx], [dy]) in [ink] and the ink under it
    private fun overlap(
        ink: Mask,
        shape: Mask,
        dx: Int,
        dy: Int,
    ): Double {
        var both = 0
        var either = 0
        for (i in shape.bits.indices) {
            val a = ink.bits[(i / shape.width + dy) * ink.width + i % shape.width + dx]
            if (a && shape.bits[i]) both++
            if (a || shape.bits[i]) either++
        }
        return if (either == 0) 0.0 else both.toDouble() / either
    }

    /** Which pixels of a [width] by [height] image are ink. */
    private class Mask(
        val bits: BooleanArray,
        val width: Int,
        val height: Int,
    )

    // sets [theme] and waits until every badge read as [label] stands on a ground of that theme
    private fun useTheme(
        theme: ThemeChoice,
        label: String,
    ) {
        runBlocking { graph().settings.update { it.copy(theme = theme) } }
        val dark = theme == ThemeChoice.Dark
        compose.waitUntil(CONNECT_WAIT_MS) {
            runCatching {
                shownBadges(label).let { badges ->
                    badges.isNotEmpty() &&
                        badges.all { isDark(ground(pixels(it.captureToImage().asAndroidBitmap()))) == dark }
                }
            }.getOrDefault(false)
        }
    }

    private fun isDark(color: Int): Boolean = Color.red(color) + Color.green(color) + Color.blue(color) < MID_GREY

    private fun awaitBadge(label: String) =
        compose.waitUntil(CONNECT_WAIT_MS) { nodes(hasContentDescription(label)) > 0 }

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

    private fun graph() = (context.applicationContext as NgtermApplication).graph

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
        shell("screencap -p $EVIDENCE_DIR/status-checkmark-$name.png")
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

        const val MACHINE = "qa-check"
        const val SESSION = "qa-check-work"

        /** Typed into the session's shell (fish; `%s` is a space to `input text`): `t` names this session. */
        const val TARGET = "set%s-x%st%s\$NGTERM_SESSION_ID"

        /** The agent status `completed`, which without a shape of its own the app draws as a star. */
        const val DONE_WIRE = "completed"

        fun status(
            status: String,
            shape: String,
        ) = "ngtermctl%ssession%sstatus%s$status%s--shape%s$shape%s--target%s\$t"

        /** Typed until the shell prints [READY], which the typed line itself does not hold. */
        const val PING = "string%supper%sqaready"
        const val READY = "QAREADY"

        /** StatusGlyph's size in a badge. */
        const val GLYPH_DP = 12f

        /** How far in from the badge's leading edge the glyph may start, in pixels. */
        const val MAX_X_OFFSET = 8
        const val INK_DISTANCE = 96
        const val HALF_ALPHA = 128

        /** The sum of the red, green and blue of a mid grey: a darker ground is a dark theme's. */
        const val MID_GREY = 3 * 128
        const val TOP = 3

        /** The overlap any drawn glyph reaches with its own shape and the shapes like it. */
        const val MIN_MATCH = 0.5

        const val EVIDENCE_DIR = "/data/local/tmp/ngterm-e2e"
        const val CONNECT_WAIT_MS = 60_000L
        const val PING_WAIT_MS = 3_000L
    }
}
