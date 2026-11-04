package com.nickalie.ngterm.android.ui.chat

import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import com.github.takahirom.roborazzi.captureRoboImage
import com.nickalie.ngterm.android.ui.Window
import com.nickalie.ngterm.android.ui.WindowRule
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.ChatBody
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.annotation.GraphicsMode

/**
 * The chat (src/test/screenshots/chat) on a phone (light, dark, font scale 2.0), an unfolded foldable and a landscape
 * tablet (light and dark): a long conversation, an open approval, a multi-select question, an expanded diff, the
 * empty and error states, and the composer's `/` menu open on every command, narrowed (the agent working, so Stop
 * shows), and matching nothing.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class ChatScreenshotTest(
    private val window: Window,
) {
    private val compose = createComposeRule()

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(WindowRule(window)).around(compose)

    private fun capture(
        scene: String,
        state: ChatUiState,
        composer: ChatComposer? = null,
        then: (() -> Unit)? = null,
    ) {
        compose.setContent {
            NgtermTheme(darkTheme = window.name.endsWith("dark")) {
                ChatScreen(
                    state,
                    "build-01",
                    ChatActions(),
                    Modifier.fillMaxSize().safeDrawingPadding(),
                    composer = composer,
                )
            }
        }
        compose.waitForIdle()
        then?.invoke()
        compose.waitForIdle()
        compose.onRoot().captureRoboImage("src/test/screenshots/chat/${scene}_$window.png")
    }

    @Test
    fun longConversation() = capture("conversation", ChatSamples.state(ChatSamples.long))

    @Test
    fun approval() = capture("approval", ChatSamples.state(ChatSamples.long).copy(approval = ChatSamples.approval))

    @Test
    fun multiSelectQuestion() {
        val question = event("q1", ChatBody.Question(ChatSamples.shells))
        val events = listOf(ChatSamples.prompt, ChatSamples.opening, ChatSamples.read, question)
        capture("question", ChatSamples.state(events).copy(question = OpenQuestion("q1", ChatSamples.shells)))
    }

    @Test
    fun expandedDiff() =
        capture("diff", ChatSamples.state(listOf(ChatSamples.prompt, ChatSamples.opening, ChatSamples.edit))) {
            compose.onNode(hasText("docs/install.md")).performClick()
        }

    @Test
    fun empty() = capture("empty", ChatUiState(phase = ChatPhase.Live, conversation = ChatSamples.conversation))

    @Test
    fun error() =
        capture(
            "error",
            ChatSamples
                .state(
                    listOf(ChatSamples.prompt, ChatSamples.opening, ChatSamples.grep),
                    older = OlderPages.Failed("the connection to build-01 was lost"),
                ).copy(problem = ChatProblem.NotOnScreen),
        )

    @Test
    fun openMenu() = capture("menu", ChatSamples.state(ChatSamples.long), ChatSamples.composer("/"))

    @Test
    fun filteredMenu() =
        capture(
            "menu_filtered",
            ChatSamples.state(ChatSamples.long).copy(working = true),
            ChatSamples.composer("/co"),
        )

    @Test
    fun menuWithNoMatch() =
        capture("menu_no_match", ChatSamples.state(ChatSamples.long), ChatSamples.composer("/deploy"))

    companion object {
        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}")
        fun windows() =
            listOf(
                Window("compact_light", "w411dp-h891dp-xhdpi"),
                Window("compact_dark", "w411dp-h891dp-night-xhdpi"),
                Window("compact_font2", "w411dp-h891dp-xhdpi", fontScale = 2f),
                Window("medium_light", "w700dp-h1000dp-hdpi"),
                Window("medium_dark", "w700dp-h1000dp-night-hdpi"),
                Window("expanded_light", "w1280dp-h800dp-land-mdpi"),
                Window("expanded_dark", "w1280dp-h800dp-land-night-mdpi"),
            ).map { arrayOf<Any>(it) }
    }
}
