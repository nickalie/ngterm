package com.nickalie.ngterm.android.ui.components

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.test.click
import androidx.compose.ui.test.junit4.v2.createComposeRule
import androidx.compose.ui.test.longClick
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.swipeUp
import androidx.compose.ui.unit.dp
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/** The cover veils its pane: touches stop at it, TalkBack doesn't find the pane, and its own action still works. */
@RunWith(AndroidJUnit4::class)
class CoverTest {
    @get:Rule
    val compose = createComposeRule()

    @Test
    fun theVeilTakesTouchesAndHidesThePaneButItsActionAnswers() {
        var paneTaps = 0
        var actionTaps = 0
        compose.setContent {
            NgtermTheme {
                Box(Modifier.fillMaxSize()) {
                    Box(
                        Modifier
                            .fillMaxSize()
                            .underCover(covered = true)
                            .clickable { paneTaps++ },
                    ) { Text("pane text") }
                    Cover(
                        title = "Another device is driving this pane",
                        body = "Take control to type here.",
                        modifier = Modifier.testTag("cover"),
                        action = SectionAction("Take control") { actionTaps++ },
                    )
                }
            }
        }
        compose.onNodeWithTag("cover").performTouchInput {
            click(topLeft)
            longClick(bottomCenter)
            swipeUp()
        }
        assertEquals("no touch reaches the pane", 0, paneTaps)
        compose.onNodeWithText("pane text").assertDoesNotExist()
        compose.onNodeWithText("Take control").performClick()
        assertEquals(1, actionTaps)
    }

    @Test
    fun aTapThatMovesALittleStillAnswersTheAction() {
        var actionTaps = 0
        compose.setContent {
            NgtermTheme {
                Cover(
                    title = "Another device is driving this pane",
                    body = "Take control to type here.",
                    action = SectionAction("Take control") { actionTaps++ },
                )
            }
        }
        // a finger on glass reports a move or two even for a still tap
        compose.onNodeWithText("Take control").performTouchInput {
            down(center)
            moveBy(Offset(1f, 1f))
            moveBy(Offset(-1f, 0f))
            up()
        }
        assertEquals(1, actionTaps)
    }

    @Test
    fun aDragScrollsTheCoverInAShortPane() {
        compose.setContent {
            NgtermTheme {
                Box(Modifier.height(120.dp)) {
                    Cover(
                        title = "Another device is driving this pane",
                        body = "Take control to type here.",
                        modifier = Modifier.testTag("cover"),
                        action = SectionAction("Take control") {},
                    )
                }
            }
        }
        val before =
            compose
                .onNodeWithText("Take control")
                .fetchSemanticsNode()
                .positionInRoot.y
        compose.onNodeWithTag("cover").performTouchInput { swipeUp() }
        val after =
            compose
                .onNodeWithText("Take control")
                .fetchSemanticsNode()
                .positionInRoot.y
        assertTrue("the content scrolls up ($before -> $after)", after < before)
    }
}
