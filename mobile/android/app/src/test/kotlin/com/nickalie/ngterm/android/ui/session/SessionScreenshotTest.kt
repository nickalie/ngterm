package com.nickalie.ngterm.android.ui.session

import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import com.github.takahirom.roborazzi.captureRoboImage
import com.nickalie.ngterm.android.data.Link
import com.nickalie.ngterm.android.data.LiveSession
import com.nickalie.ngterm.android.data.PaneLive
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.ui.AdaptiveScaffold
import com.nickalie.ngterm.android.ui.AppDestinations
import com.nickalie.ngterm.android.ui.NavLayout
import com.nickalie.ngterm.android.ui.SampleAgtermPeer
import com.nickalie.ngterm.android.ui.Window
import com.nickalie.ngterm.android.ui.terminal.SelectingPane
import com.nickalie.ngterm.android.ui.terminal.TerminalState
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.FakeCursor
import com.nickalie.ngterm.core.ffi.PaneEnd
import com.nickalie.ngterm.core.ffi.PaneRef
import com.nickalie.ngterm.core.ffi.PaneRole
import com.nickalie.ngterm.core.ffi.RemoteHud
import com.nickalie.ngterm.core.ffi.RemoteLayout
import com.nickalie.ngterm.core.ffi.StreamState
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.rules.RuleChain
import org.junit.rules.TestRule
import org.junit.runner.Description
import org.junit.runner.RunWith
import org.junit.runners.model.Statement
import org.robolectric.ParameterizedRobolectricTestRunner
import org.robolectric.RuntimeEnvironment
import org.robolectric.annotation.GraphicsMode
import kotlin.coroutines.EmptyCoroutineContext

/** Sets the window before the compose rule starts its activity. */
private class SessionWindowRule(
    private val window: Window,
) : TestRule {
    override fun apply(
        base: Statement,
        description: Description,
    ) = object : Statement() {
        override fun evaluate() {
            RuntimeEnvironment.setQualifiers(window.qualifiers)
            base.evaluate()
        }
    }
}

/**
 * The session view (src/test/screenshots/session) in the app's frame at the three widths: a phone (one pane at a
 * time, the bar stepped aside), a medium window (the rail of attached sessions) and an expanded one (the sidebar of
 * machines and their sessions beside the split). Each shows one pane with the HUD, two panes side by side and
 * stacked, a hidden split, a follower's cover, a reconnecting pane and a session of an agterm machine (its badge in
 * the bar); and with none open, the attached sessions as a tree, a companion under its parent.
 */
@RunWith(ParameterizedRobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
class SessionScreenshotTest(
    private val window: Window,
) {
    private val compose = createComposeRule()

    @get:Rule
    val rules: RuleChain = RuleChain.outerRule(SessionWindowRule(window)).around(compose)

    private val key = SessionKey("m1", "s1")

    private val navLayout =
        when {
            window.name.startsWith("compact") -> NavLayout.Bar
            window.name.startsWith("medium") -> NavLayout.Rail
            else -> NavLayout.Sidebar
        }

    private fun capture(
        scene: String,
        layout: RemoteLayout,
        panes: Map<String, PaneLive> = emptyMap(),
        hud: RemoteHud? = null,
        open: (() -> Unit)? = null,
    ) {
        val live = LiveSession(StreamState.Connected, null, CONTEXT, layout, hud, panes)
        shoot(scene, AttachedSession(key, "build-01", "docs-site", CONTEXT, Waiting, Link.Connected, live), open)
    }

    private fun shoot(
        scene: String,
        session: AttachedSession,
        open: (() -> Unit)? = null,
    ) {
        val layout = session.live.layout ?: return
        frame(groups(session), key) {
            SessionScreen(
                session = session,
                focused = layout.primary,
                panes = rememberStates(layout.panes),
                failures = emptyMap(),
                compact = navLayout == NavLayout.Bar,
                actions = actions(navLayout == NavLayout.Bar),
            )
        }
        compose.waitForIdle()
        open?.invoke()
        compose.onRoot().captureRoboImage("src/test/screenshots/session/${scene}_$window.png")
    }

    private fun frame(
        groups: List<AttachedGroup>,
        open: SessionKey?,
        content: @Composable () -> Unit,
    ) {
        compose.setContent {
            NgtermTheme(darkTheme = window.name.endsWith("dark")) {
                AdaptiveScaffold(
                    destinations = AppDestinations,
                    selected = "sessions",
                    onSelect = {},
                    layout = navLayout,
                    navigationVisible = open == null,
                    railContent = { expanded -> RailSessions(groups, open, expanded, {}) },
                    sidebarContent = { SidebarSessions(groups, open, {}) },
                    content = content,
                )
            }
        }
    }

    // build-01's docs-site and api-refactor, nas's backup; [companions] adds api-refactor's companion under it and a
    // companion whose parent isn't attached
    private fun groups(
        session: AttachedSession,
        companions: Boolean = false,
    ): List<AttachedGroup> {
        val api =
            session.copy(key = SessionKey("m1", "s2"), name = "api-refactor", context = null, status = Working)
        val review =
            api.copy(
                key = SessionKey("m1", "s4"),
                name = "api-refactor review",
                companionOf = "api-refactor",
                nested = true,
            )
        val drafts =
            session.copy(key = SessionKey("m1", "s5"), name = "release notes", status = null, companionOf = "changelog")
        val backup =
            session.copy(
                key = SessionKey("m2", "s3"),
                machine = "nas",
                name = "backup",
                context = null,
                status = null,
            )
        val built = if (companions) listOf(session, api, review, drafts) else listOf(session, api)
        return listOf(AttachedGroup("m1", "build-01", built), AttachedGroup("m2", "nas", listOf(backup)))
    }

    // a pane per id, drawing a few lines of its own, woken as the view sizes it
    @Composable
    private fun rememberStates(ids: List<String>): Map<PaneRef, TerminalState> {
        val made =
            remember(ids) {
                ids.associate { id ->
                    val pane = SelectingPane(SCREENS.getValue(id), FakeCursor(2, SCREENS.getValue(id).lastIndex))
                    PaneRef("m1", "s1", id) to (pane to TerminalState(pane, EmptyCoroutineContext))
                }
            }
        DisposableEffect(made) {
            made.values.forEach { (pane, state) -> pane.onWake = state::dirty }
            onDispose { made.values.forEach { (pane, _) -> pane.onWake = null } }
        }
        return made.mapValues { it.value.second }
    }

    @Test
    fun onePane() =
        capture(
            "one_pane",
            RemoteLayout(listOf("P1"), "P1", null, false),
            hud =
                RemoteHud(
                    "Building the docs",
                    "step 2 of 5",
                    "bar",
                    null,
                    null,
                    null,
                    "top-right",
                    false,
                    null,
                    false,
                    true,
                    null,
                    1,
                    null,
                ),
        )

    @Test
    fun sideBySide() = capture("split_vertical", RemoteLayout(listOf("P1", "P2"), "P2", "vertical", true))

    @Test
    fun stacked() = capture("split_horizontal", RemoteLayout(listOf("P1", "P2"), "P1", "horizontal", true))

    @Test
    fun hiddenSplit() = capture("split_hidden", RemoteLayout(listOf("P1", "P2"), "P1", "vertical", false))

    @Test
    fun duplicating() {
        val live = LiveSession(StreamState.Connected, null, CONTEXT, RemoteLayout(listOf("P1"), "P1", null, false))
        val session = AttachedSession(key, "build-01", "docs-site", CONTEXT, Waiting, Link.Connected, live)
        shoot("duplicating", session.copy(duplicating = true, canDuplicate = false))
    }

    @Test
    fun agterm() {
        val live = LiveSession(StreamState.Connected, null, CONTEXT, RemoteLayout(listOf("P1"), "P1", null, false))
        val session = AttachedSession(key, "build-01", "docs-site", CONTEXT, Waiting, Link.Connected, live)
        shoot("agterm", session.copy(peer = SampleAgtermPeer))
    }

    @Test
    fun follower() =
        capture(
            "follower",
            RemoteLayout(listOf("P1"), "P1", null, false),
            mapOf("P1" to PaneLive(PaneRole.FOLLOWER, covered = true)),
        )

    @Test
    fun railOpen() {
        // the rail opens only at the medium width
        assumeTrue(navLayout == NavLayout.Rail)
        capture("rail_open", RemoteLayout(listOf("P1", "P2"), "P1", "vertical", true)) {
            compose.onNodeWithContentDescription("Show navigation labels").performClick()
            compose.waitForIdle()
        }
    }

    @Test
    fun attachedTree() {
        val live = LiveSession(StreamState.Connected, null, CONTEXT, RemoteLayout(listOf("P1"), "P1", null, false))
        val groups = groups(AttachedSession(key, "build-01", "docs-site", CONTEXT, Waiting, Link.Connected, live), true)
        val none = AttachedActions({}, {}, { _, _ -> }, { _, _ -> }, { _, _ -> }, {}, {}, {}, {}, {})
        frame(groups, open = null) {
            SessionsDestination(AttachedUiState(groups), emptyMap(), emptyMap(), navLayout == NavLayout.Bar, null, none)
        }
        compose.waitForIdle()
        compose.onRoot().captureRoboImage("src/test/screenshots/session/attached_tree_$window.png")
    }

    @Test
    fun reconnecting() =
        capture(
            "reconnecting",
            RemoteLayout(listOf("P1"), "P1", null, false),
            mapOf("P1" to PaneLive(PaneRole.LEADER, end = PaneEnd.Lost("link down"))),
        )

    companion object {
        private const val CONTEXT = "Rewrite the install guide"
        private val Waiting = AgentStatus("blocked", null, null)
        private val Working = AgentStatus("active", null, null)

        private val SCREENS =
            mapOf(
                "P1" to
                    listOf(
                        "$ make docs",
                        "building site/ (14 pages)",
                        "  install.md    ok",
                        "  server.md     ok",
                        "  sessions.md   warn: 2 dead links",
                        "$ ",
                    ),
                "P2" to
                    listOf(
                        "$ git status --short",
                        " M docs/install.md",
                        " M docs/server.md",
                        "?? docs/android.md",
                        "$ ",
                    ),
            )

        // a phone's view has Back; the wider ones keep the list beside it
        private fun actions(compact: Boolean) =
            SessionViewActions(
                onBack = if (compact) ({}) else null,
                onFocus = { _, _ -> },
                onShow = { _, _ -> },
                onRetry = { _, _ -> },
                onTakeLead = {},
                onDetach = {},
                onEnd = {},
                onNextSession = {},
                onPreviousSession = {},
            )

        @JvmStatic
        @ParameterizedRobolectricTestRunner.Parameters(name = "{0}")
        fun windows() =
            listOf(
                Window("compact_dark", "w411dp-h891dp-night-xhdpi"),
                Window("compact_light", "w411dp-h891dp-xhdpi"),
                Window("medium_light", "w700dp-h1000dp-hdpi"),
                Window("expanded_dark", "w1280dp-h800dp-land-night-mdpi"),
            ).map { arrayOf<Any>(it) }
    }
}
