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
import com.nickalie.ngterm.core.ffi.ChatToolKind
import com.nickalie.ngterm.core.ffi.ChatUsage
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.runner.RunWith
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.annotation.GraphicsMode

/**
 * The chat (src/test/screenshots/chat) on a phone (light, dark, font scale 2.0), an unfolded foldable and a landscape
 * tablet (light and dark): a long conversation, an open approval, a multi-select question, an expanded diff, the
 * empty and error states, the account and its weekly limit over the rows, the activity line open on a subagent and
 * a command left running, the agent working over an empty composer (the line's Stop, the field's Commands), and the
 * composer's `/` menu open on every command, narrowed (the agent working), and matching nothing.
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
    fun account() =
        capture(
            "account",
            ChatSamples.state(listOf(ChatSamples.prompt, ChatSamples.opening, ChatSamples.grep)).copy(
                // a reset more than a week out reads as a date, the same whenever the test runs
                account = ChatBody.Account("nick@example.com", ChatUsage(84u, RESETS_AT)),
            ),
        )

    @Test
    fun stillRunning() =
        capture(
            "running",
            ChatSamples.state(
                listOf(
                    ChatSamples.prompt,
                    ChatSamples.opening,
                    event("task", ChatBody.Tool(ChatSamples.running("Agent", ChatToolKind.TASK, "Review the parser"))),
                    event(
                        "t9",
                        ChatBody.Tool(ChatSamples.running("Bash", ChatToolKind.SHELL, "cargo test -p ngterm-core")),
                    ),
                    event("a9", reply("Both are running in the background; I'll report when they finish.")),
                ),
            ),
        ) {
            compose.onNode(hasText("Still running in the background")).performClick()
        }

    @Test
    fun working() =
        capture("working", ChatSamples.state(ChatSamples.long).copy(working = true), ChatSamples.composer(""))

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
        // noon UTC on 10 January 2036: the same date in every zone, and always more than a week out
        private const val RESETS_AT = 2_083_579_200uL

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
