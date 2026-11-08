package com.nickalie.ngterm.android.ui

import android.view.ViewGroup
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.hasText
import androidx.compose.ui.test.isSelectable
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.core.view.descendants
import androidx.lifecycle.Lifecycle
import androidx.test.ext.junit.runners.AndroidJUnit4
import com.nickalie.ngterm.android.data.OpenTarget
import com.nickalie.ngterm.android.data.SessionKey
import com.nickalie.ngterm.android.testMachine
import com.nickalie.ngterm.android.ui.session.SessionViewFixture
import com.nickalie.ngterm.android.ui.theme.NgtermTheme
import com.nickalie.ngterm.core.fake.fakeSession
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.annotation.Config
import org.robolectric.util.ReflectionHelpers

/** Notification fallback and foreground changes through the app's actual navigation and attached sessions. */
@RunWith(AndroidJUnit4::class)
@Config(qualifiers = "w1000dp-h800dp-mdpi")
class NgtermAppStateTest : SessionViewFixture() {
    @Test
    fun anAttachedSessionAppearsInTheRailBeforeAnySessionIsOpened() {
        test.seedMachine(testMachine(keyId = test.seedKey().id))
        test.core.sessions["m1"] = mutableListOf(fakeSession("s1", name = "docs-site"))
        compose.setContent {
            NgtermTheme { NgtermApp(test.graph, "rail-test", layout = NavLayout.Rail) }
        }
        waitFor { test.connections.trees.value["m1"] != null }
        compose.runOnIdle { runBlocking { test.connections.attach("m1", "s1") } }
        waitFor { described("docs-site") }
        compose.onNodeWithContentDescription("docs-site").assertIsNotSelected().performClick()
        waitFor { test.asks.onScreen.value == SessionKey("m1", "s1") }
        compose.onNodeWithContentDescription("docs-site").assertIsSelected()
    }

    @Test
    fun keepingTheScreenOnAppliesOnlyWhileTheSessionDestinationIsVisible() {
        runBlocking { test.settings.update { it.copy(keepScreenOn = true) } }
        layout.value = NavLayout.Sidebar
        openSession()
        waitFor {
            compose.waitForIdle()
            (compose.activity.window.decorView as ViewGroup).descendants.any { it.keepScreenOn }
        }
        compose.onNodeWithText("Settings").performClick()
        waitFor {
            compose.waitForIdle()
            (compose.activity.window.decorView as ViewGroup).descendants.none { it.keepScreenOn }
        }
        assertEquals(setOf("s1"), test.connections.attached.value["m1"])
    }

    @Test
    fun aConfigurationStopKeepsTheSessionVisibleUntilItsReplacementActivityStarts() {
        openSession()
        val key = SessionKey("m1", "s1")
        waitFor { test.asks.onScreen.value == key }
        // Android sets this read-only Activity flag while replacing a stopped activity for a configuration change.
        ReflectionHelpers.setField(compose.activity, "mChangingConfigurations", true)
        try {
            compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
            compose.waitForIdle()
            assertEquals(key, test.asks.onScreen.value)
        } finally {
            ReflectionHelpers.setField(compose.activity, "mChangingConfigurations", false)
            compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        }
    }

    @Test
    fun machineAndExpiredSessionNotificationsReturnToMachinesAndAreConsumed() {
        openSession()
        for (target in listOf(
            OpenTarget("m1", null, null),
            OpenTarget("m1", "gone", "P1"),
            OpenTarget("gone-machine", "s1", null),
        )) {
            compose.runOnIdle { test.opens.request(target) }
            waitFor { test.opens.pending.value == null }
            compose.onNode(hasText("Machines") and isSelectable()).assertIsSelected()
            assertNull(test.asks.onScreen.value)
            compose.runOnIdle { test.opens.request(OpenTarget("m1", "s1", null)) }
            waitFor { test.asks.onScreen.value == SessionKey("m1", "s1") }
        }
    }

    @Test
    fun anOpenSessionIsShownToAsksOnlyWhileItsActivityIsStarted() {
        layout.value = NavLayout.Sidebar
        openSession()
        val key = SessionKey("m1", "s1")
        waitFor { test.asks.onScreen.value == key }
        compose.activityRule.scenario.moveToState(Lifecycle.State.CREATED)
        waitFor { test.asks.onScreen.value == null }
        compose.activityRule.scenario.moveToState(Lifecycle.State.RESUMED)
        waitFor { test.asks.onScreen.value == key }
        compose.onNodeWithText("Settings").performClick()
        waitFor { test.asks.onScreen.value == null }
        assertEquals(setOf("s1"), test.connections.attached.value["m1"])
    }
}
