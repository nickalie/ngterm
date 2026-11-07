package com.nickalie.ngterm.android.ui.session

import androidx.compose.material3.Text
import androidx.compose.runtime.mutableStateOf
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.onNodeWithText
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.ffi.PaneRef
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** Layout fallback is observable even before a terminal handle is available. */
@RunWith(AndroidJUnit4::class)
class SplitLayoutFallbackTest {
    @get:Rule val compose = createComposeRule()

    @Test
    fun anEmptyLayoutDrawsNothingAndAnUnselectedOrRemovedPaneFallsBackToTheFirst() {
        val panes = mutableStateOf(emptyList<PaneSpot>())
        val focused = mutableStateOf<String?>(null)
        compose.setContent {
            NgtermTheme {
                SplitLayout(panes.value, "vertical", false, focused.value) { spot, modifier ->
                    Text(spot.ref.paneId, modifier)
                }
            }
        }
        compose.onNodeWithText("P1").assertDoesNotExist()
        panes.value = listOf(spot("P1"), spot("P2"))
        compose.onNodeWithText("P1").assertExists()
        compose.onNodeWithText("P2").assertDoesNotExist()
        focused.value = "P2"
        compose.onNodeWithText("P2").assertExists()
        compose.onNodeWithText("P1").assertDoesNotExist()
        panes.value = listOf(spot("P1"))
        compose.onNodeWithText("P1").assertExists()
        compose.onNodeWithText("P2").assertDoesNotExist()
    }

    @Test
    fun twoPanesWithoutAnAxisShowTheFocusedOneUntilTheSplitArrives() {
        val axis = mutableStateOf<String?>(null)
        compose.setContent {
            NgtermTheme {
                SplitLayout(listOf(spot("P1"), spot("P2")), axis.value, true, "P2") { spot, modifier ->
                    Text(spot.ref.paneId, modifier)
                }
            }
        }
        compose.onNodeWithText("P2").assertExists()
        compose.onNodeWithText("P1").assertDoesNotExist()
        axis.value = "vertical"
        compose.onNodeWithText("P1").assertExists()
        compose.onNodeWithText("P2").assertExists()
    }

    private fun spot(id: String) = PaneSpot(PaneRef("m1", "s1", id), null, null, null)
}
