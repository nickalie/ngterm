package com.nickalie.ngterm.android.ui.asks

import androidx.compose.runtime.Composable
import androidx.compose.ui.test.junit4.v2.createComposeRule
import com.github.takahirom.roborazzi.ExperimentalRoborazziApi
import com.github.takahirom.roborazzi.captureScreenRoboImage
import com.nickalie.ngterm.android.ui.SampleSessions
import com.nickalie.ngterm.android.ui.SessionsSample
import com.nickalie.ngterm.android.ui.Window
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.RemoteAsk
import com.nickalie.ngterm.core.ffi.RemoteAskButton
import com.nickalie.ngterm.core.ffi.RemoteAskChoice
import com.nickalie.ngterm.core.ffi.RemoteAskChoices
import com.nickalie.ngterm.core.ffi.RemoteAskInput
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runner.RunWith
import org.junit.runners.model.Statement
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.RobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode

/** A button ask: the default, a destructive choice and the cancel button. */
val ButtonAsk =
    RemoteAsk(
        id = "A1",
        title = "Deploy docs-site to production?",
        message = "The build passed on build-01. Deploying replaces the live site; rolling back restores yesterday's.",
        buttons =
            listOf(
                RemoteAskButton("deploy", "Deploy", "d"),
                RemoteAskButton("rollback", "Roll back", "r"),
                RemoteAskButton("wait", "Not yet", "n"),
            ),
        defaultId = "deploy",
        destructiveId = "rollback",
        cancelId = "wait",
        style = "terminal",
        align = "right",
        width = null,
        pane = null,
        input = null,
        choices = null,
    )

/** A radio list with subtitles and its free-form row. */
val ChoiceAsk =
    ButtonAsk.copy(
        id = "A2",
        title = "Which branch should the fix go on?",
        message = null,
        buttons = listOf(RemoteAskButton("ok", "Use this branch", null), RemoteAskButton("cancel", "Cancel", null)),
        defaultId = "ok",
        destructiveId = null,
        cancelId = "cancel",
        choices =
            RemoteAskChoices(
                choices =
                    listOf(
                        RemoteAskChoice("main", "main", "The release line"),
                        RemoteAskChoice("develop", "develop", "Has the new installer"),
                        RemoteAskChoice("release", "release/2.0", null),
                    ),
                multiple = false,
                checked = listOf("develop"),
                other = "Another branch",
            ),
    )

/** A multiline text field with the origin's text in it. */
val InputAsk =
    ButtonAsk.copy(
        id = "A3",
        title = "Describe the change for the changelog",
        message = "One or two lines; it goes under 2.0.1.",
        buttons = listOf(RemoteAskButton("save", "Save", null), RemoteAskButton("skip", "Skip", null)),
        defaultId = "save",
        destructiveId = null,
        cancelId = "skip",
        input = RemoteAskInput(true, "Fix the installer's PATH check on fish.", null),
    )

/** Sets the window before the compose rule starts its activity. */
private class AskWindowRule(
    private val window: Window,
) : TestRule {
    override fun apply(
        base: Statement,
        description: Description,
    ) = object : Statement() {
        override fun evaluate() {
            RuntimeEnvironment.setQualifiers(window.qualifiers)
            RuntimeEnvironment.setFontScale(window.fontScale)
            base.evaluate()
        }
    }
}

/**
 * The sheets with no text field (src/test/screenshots/asks): a button ask and the notification permission's
 * question, over a machine's sessions, on a phone (light, dark, font scale 2.0), an unfolded foldable and a landscape
 * tablet (light, dark).
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AskScreenshotTest(
    private val window: Window,
) {
    private val compose = createComposeRule()

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(AskWindowRule(window)).around(compose)

    @OptIn(ExperimentalRoborazziApi::class)
    private fun capture(
        name: String,
        sheet: @Composable () -> Unit,
    ) {
        compose.setContent {
            NgtermTheme {
                SessionsSample(SampleSessions)
                sheet()
            }
        }
        compose.waitForIdle()
        captureScreenRoboImage("src/test/screenshots/asks/${name}_$window.png")
    }

    @Test
    fun buttonAsk() = capture("ask_buttons") { AskSheet(ButtonAsk, onAnswer = {}) }

    @Test
    fun notificationPermission() =
        capture("notification_permission") {
            NotificationPermissionSheet(onAllow = {}, onLater = {})
        }

    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}")
        fun windows() =
            listOf(
                Window("compact_light", "w411dp-h891dp-xhdpi"),
                Window("compact_dark", "w411dp-h891dp-night-xhdpi"),
                Window("compact_font2", "w411dp-h891dp-xhdpi", fontScale = 2f),
                Window("medium_light", "w700dp-h1000dp-hdpi"),
                Window("expanded_light", "w1280dp-h800dp-land-mdpi"),
                Window("expanded_dark", "w1280dp-h800dp-land-night-mdpi"),
            ).map { arrayOf<Any>(it) }
    }
}

/**
 * The sheets with a text field: a choice list with its free-form row, and a multiline answer. Robolectric's default
 * window, light and dark: under a phone's qualifiers its layout of a text field in a dialog never settles (Task 14).
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class AskFormScreenshotTest {
    @get:Rule
    val compose = createComposeRule()

    @OptIn(ExperimentalRoborazziApi::class)
    private fun capture(
        name: String,
        ask: RemoteAsk,
    ) {
        compose.setContent {
            NgtermTheme {
                SessionsSample(SampleSessions)
                AskSheet(ask, onAnswer = {})
            }
        }
        compose.waitForIdle()
        captureScreenRoboImage("src/test/screenshots/asks/$name.png")
    }

    @Test
    fun choiceLight() = capture("ask_choices_light", ChoiceAsk)

    @Test
    @Config(qualifiers = "night")
    fun choiceDark() = capture("ask_choices_dark", ChoiceAsk)

    @Test
    fun inputLight() = capture("ask_input_light", InputAsk)

    @Test
    @Config(qualifiers = "night")
    fun inputDark() = capture("ask_input_dark", InputAsk)
}
