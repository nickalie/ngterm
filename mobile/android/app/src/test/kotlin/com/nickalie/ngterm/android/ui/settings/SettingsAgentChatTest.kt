package com.nickalie.ngterm.android.ui.settings

import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.state.ToggleableState
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.lifecycle.ViewModelProvider
import androidx.lifecycle.ViewModelStore
import androidx.lifecycle.viewmodel.initializer
import androidx.lifecycle.viewmodel.viewModelFactory
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.TestGraph
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.runBlocking
import org.junit.After
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder
import org.junit.runner.RunWith
import org.robolectric.annotation.Config

/**
 * Settings' Agent chat view switch as a finger reaches it: on by default, and a tap stores it off (what
 * `ChatModeTest` then shows as every pane's terminal) and back on.
 */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w411dp-h891dp-xhdpi")
class SettingsAgentChatTest {
    @get:Rule
    val dir = TemporaryFolder()

    @get:Rule
    val compose = createComposeRule()

    private lateinit var test: TestGraph
    private val models = ViewModelStore()

    @Before
    fun setUp() {
        test = TestGraph(dir.root)
    }

    @After
    fun tearDown() {
        models.clear()
        test.close()
    }

    private fun stored() = runBlocking { test.settings.settings.first() }.agentChat

    private fun toggled(label: SemanticsMatcher) =
        compose
            .onAllNodes(label)
            .fetchSemanticsNodes()
            .single()
            .config[SemanticsProperties.ToggleableState]

    @Test
    fun theAgentChatViewSwitchIsOnByDefaultAndATapStoresItOffAndOn() {
        val model =
            ViewModelProvider(models, viewModelFactory { initializer { SettingsViewModel(test.graph) } })[
                SettingsViewModel::class.java,
            ]
        val actions = SettingsActions(model::change, { _, _ -> }, {}, {}, {}, {}, {}, {})
        compose.setContent {
            val state by model.state.collectAsState()
            NgtermTheme {
                SettingsScreen(
                    state,
                    "0.1.0",
                    notificationsOn = true,
                    keyWork = KeyWork(),
                    actions = actions,
                )
            }
        }
        val label = hasText("Agent chat view")
        compose.waitUntil(10_000) { compose.onAllNodes(label).fetchSemanticsNodes().isNotEmpty() }
        val row = compose.onNode(label)
        row.performScrollTo().assertIsOn()
        assertTrue(stored())

        row.performClick()
        compose.waitUntil(10_000) { toggled(label) == ToggleableState.Off }
        assertFalse(stored())

        row.performClick()
        compose.waitUntil(10_000) { toggled(label) == ToggleableState.On }
        assertTrue(stored())
    }
}
